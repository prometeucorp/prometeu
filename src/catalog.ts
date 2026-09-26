import { listen } from "@tauri-apps/api/event";
import { invoke } from "./ipc";
import { fromBack, t } from "./i18n";
import { field, formDialog, input, confirmDialog } from "./ui";
import type { Item } from "./menu";
import type { ResourceItem, ResourceOrigin } from "./resources/model";
import { h } from "./util";

import type { CatalogProject } from "./projects";

export type Kind = "plugins" | "mcp" | "skills";
export type CatalogPlugin = { id: string; source: string; note: string; local_id: string; installed: boolean; source_changed: boolean };
export type CatalogSkill = { id: string; description: string; content: string; local_id: string; installed: boolean };
export type OrganizationItem = { organization: string; organization_name: string; revision: number | null; kind: Kind; id: string; description: string; installed: boolean; local_id?: string | null };
export type CatalogState = {
  projects?: CatalogProject[];
  connected: boolean;
  revision: number | null;
  plugins: CatalogPlugin[];
  mcp: string[];
  skills: CatalogSkill[];
  shared: Record<string, string>;
  organization_items?: OrganizationItem[];
};
let state: CatalogState = { connected: false, revision: null, plugins: [], mcp: [], skills: [], shared: {} };
const watchers = new Set<() => void>();
let refreshHubs = async () => {};
export const current = () => state;
export const onChange = (fn: () => void) => { watchers.add(fn); return () => watchers.delete(fn); };
export const shared = (kind: Kind, id: string) => !!state.shared[`${kind}:${id}`];
export async function load() {
  state = await invoke("catalog_state");
  for (const fn of watchers) fn();
}
export async function refresh() { await refreshHubs(); await load(); }
export function init(refresh: () => Promise<void>) {
  refreshHubs = refresh;
  void load().catch(() => {});
  void listen("catalog", () => { void refreshHubs().then(load).catch(() => {}); }).catch(() => {});
}
type Say = (text: string, bad?: boolean) => void;
type Origin = ResourceOrigin;
/** A catalog definition missing from this Mac, with the action that installs it. */
type Pending = { id: string; description: string; origin: string; install: (say: Say) => Promise<void> };

const sameName = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const organizationItems = (kind: Kind) => (state.organization_items ?? []).filter(item => item.kind === kind);
const personal = (kind: Kind): { local_id: string; installed: boolean }[] =>
  kind === "plugins" ? state.plugins : kind === "skills" ? state.skills : [];
/** The backend reports an unlinked local item that already matches a personal definition. */
const equivalent = (kind: Kind, id: string) => !shared(kind, id) && personal(kind).some(item => item.installed && item.local_id === id);

function pending(kind: Kind): Pending[] {
  const items: Pending[] = [];
  if (kind === "plugins") for (const plugin of state.plugins.filter(p => !p.installed)) items.push({
    id: plugin.id, origin: t("catalog.personal"),
    description: plugin.note.trim() ? `${plugin.source} · ${plugin.note.trim()}` : plugin.source,
    // Plugin code comes from a remote source, so installation confirms the address first.
    install: () => new Promise<void>(resolve => {
      const dialog = formDialog({ title: t("catalog.install"), save: t("catalog.install"), cancel: t("actions.cancel"), error: fromBack,
        closed: resolve,
        submit: async () => { await invoke("catalog_install_plugin", { id: plugin.id }); await refresh(); } });
      dialog.body.append(h("p", "ui-hint", plugin.source), h("p", "ui-hint", t("catalog.installHint"))); dialog.open();
    }),
  });
  if (kind === "skills") for (const skill of state.skills.filter(s => !s.installed)) items.push({
    id: skill.id, description: skill.description, origin: t("catalog.personal"),
    install: say => invoke("catalog_install_skill", { id: skill.id }).then(refresh).catch(error => say(fromBack(error), true)),
  });
  for (const item of organizationItems(kind).filter(item => !item.installed)) items.push({
    id: item.id, description: item.description, origin: item.organization_name,
    install: say => invoke("catalog_install_organization_item", { organization: item.organization, kind, id: item.id, revision: item.revision })
      .then(refresh).catch(error => say(fromBack(error), true)),
  });
  return items;
}

/** Every catalog offering an installed item, plus same-name definitions that differ from it. */
export function installedOrigins(kind: Kind, id: string): Origin[] {
  const origins: Origin[] = [{ label: t("catalog.thisMac"), here: true }];
  if (shared(kind, id)) origins.push({ label: `${t("catalog.personal")} ⇄`, hint: t("catalog.liveHint") });
  else if (equivalent(kind, id)) origins.push({ label: t("catalog.personal") });
  for (const item of organizationItems(kind)) if (item.local_id === id) origins.push({ label: item.organization_name });
  for (const item of pending(kind)) if (sameName(item.id, id)) origins.push({ label: `${item.origin} ≠`, hint: t("catalog.differsHint") });
  return origins;
}

/** Group missing definitions by name; names already installed show on that row instead. */
export function pendingGroups(kind: Kind, installed: string[]): Pending[][] {
  const groups = new Map<string, Pending[]>();
  for (const item of pending(kind)) {
    if (installed.some(id => sameName(id, item.id))) continue;
    const name = item.id.toLowerCase();
    groups.set(name, [...groups.get(name) ?? [], item]);
  }
  return [...groups.values()];
}

const installing = new Set<string>();
export function pendingResources(kind: Kind, installed: string[], say: Say): ResourceItem[] {
  return pendingGroups(kind, installed).map(items => {
    const [first] = items;
    const key = `catalog-${kind}-${first.id.toLowerCase()}`;
    return {
      key, id: first.id, kind, glyph: kind === "plugins" ? "puzzle" : kind === "mcp" ? "plug" : "sparkles",
      description: `${first.description} · ${t("catalog.notInstalled")}`,
      origins: items.map(item => ({ label: item.origin })), busy: installing.has(key),
      actions: items.map(item => ({
        label: items.length > 1 ? t("catalog.installFrom", { origin: item.origin }) : t("catalog.install"),
        run: () => {
          if (installing.has(key)) return;
          installing.add(key); watchers.forEach(fn => fn());
          void item.install(say).catch(error => say(fromBack(error), true))
            .finally(() => { installing.delete(key); watchers.forEach(fn => fn()); });
        },
      })),
    };
  });
}
export function resourceActions(kind: Kind, id: string, say: Say): Item[] {
  const alternatives: Item[] = pending(kind).filter(item => sameName(item.id, id))
    .map(item => ({ label: t("catalog.installFrom", { origin: item.origin }), run: () => void item.install(say) }));
  const copy: Item = { label: t("catalog.copy"), run: () => {
    const name = input(`${id.slice(0, 45)}-local`); name.required = true;
    name.maxLength = kind === "skills" ? 56 : 128;
    if (kind === "skills") name.pattern = "[a-z0-9][a-z0-9-]{0,55}";
    const dialog = formDialog({ title: t("catalog.copy"), save: t("catalog.copy"), cancel: t("actions.cancel"), error: fromBack,
      submit: async () => { await invoke("catalog_copy", { kind, id, newId: name.value }); await refresh(); } });
    dialog.body.append(h("p", "ui-hint", t("catalog.copyHint")), field(t("catalog.copyName"), name)); dialog.open();
  } };
  if (shared(kind, id)) return [copy, ...alternatives];
  if (!state.connected) return [];
  // Preserve linking to an equivalent personal definition instead of publishing a duplicate.
  const [title, hint] = equivalent(kind, id) ? ["catalog.link", "catalog.linkHint"] as const : ["catalog.share", "catalog.shareHint"] as const;
  return [{ label: t(title), run: () => {
    const dialog = formDialog({ title: t(title), save: t(title), cancel: t("actions.cancel"), error: fromBack,
      submit: async () => { await invoke("catalog_share", { kind, id }); await refresh(); } });
    dialog.body.append(h("p", "ui-hint", t(hint))); dialog.open();
  } }, ...alternatives];
}
export async function confirmRemoval(kind: Kind, id: string): Promise<boolean> {
  if (!shared(kind, id)) return true;
  return confirmDialog({ title: t("catalog.delete"), message: t("catalog.deleteHint"), accept: t("catalog.delete"), cancel: t("actions.cancel") });
}
