import { listen } from "@tauri-apps/api/event";
import { invoke } from "./ipc";
import { fromBack, t } from "./i18n";
import { field, formDialog, input, confirmDialog } from "./ui";
import type { Item } from "./menu";
import type { ResourceItem } from "./resources/model";
import { h } from "./util";

import type { CatalogProject } from "./projects";

export type Kind = "plugins" | "mcp" | "skills";
export type CatalogPlugin = { id: string; source: string; note: string; local_id: string; installed: boolean; source_changed: boolean };
export type CatalogSkill = { id: string; description: string; content: string; local_id: string; installed: boolean };
export type OrganizationItem = { organization: string; organization_name: string; revision: number | null; kind: Kind; id: string; description: string; installed: boolean };
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
export function tag(kind: Kind, id: string): string {
  return t(shared(kind, id) ? "catalog.cloud" : "catalog.local");
}
const installing = new Set<string>();
export function organizationResources(kind: Kind, say: (text: string, bad?: boolean) => void): ResourceItem[] {
  return (state.organization_items ?? []).filter(item => item.kind === kind && !item.installed).map(item => {
    const key = `organization-${item.organization}-${kind}-${item.id}`;
    return {
      key, id: item.id, kind, scope: item.organization, origin: item.organization_name, glyph: "building",
      description: `${item.organization_name} · ${item.description} · ${t("catalog.notInstalled")}`,
      busy: installing.has(key),
      actions: [{ label: t("catalog.install"), run: () => {
        if (installing.has(key)) return;
        installing.add(key); watchers.forEach(fn => fn());
        void invoke("catalog_install_organization_item", { organization: item.organization, kind, id: item.id, revision: item.revision })
          .then(refresh).catch(error => say(fromBack(error), true))
          .finally(() => { installing.delete(key); watchers.forEach(fn => fn()); });
      } }],
    };
  });
}
export function resourceActions(kind: Kind, id: string): Item[] {
  if (shared(kind, id)) return [{ label: t("catalog.copy"), run: () => {
    const name = input(`${id.slice(0, 45)}-local`); name.required = true;
    name.maxLength = kind === "skills" ? 56 : 128;
    if (kind === "skills") name.pattern = "[a-z0-9][a-z0-9-]{0,55}";
    const dialog = formDialog({ title: t("catalog.copy"), save: t("catalog.copy"), cancel: t("actions.cancel"), error: fromBack,
      submit: async () => { await invoke("catalog_copy", { kind, id, newId: name.value }); await refresh(); } });
    dialog.body.append(h("p", "ui-hint", t("catalog.copyHint")), field(t("catalog.copyName"), name)); dialog.open();
  } }];
  if (!state.connected) return [];
  return [{ label: t("catalog.share"), run: () => {
    const dialog = formDialog({ title: t("catalog.share"), save: t("catalog.share"), cancel: t("actions.cancel"), error: fromBack,
      submit: async () => { await invoke("catalog_share", { kind, id }); await refresh(); } });
    dialog.body.append(h("p", "ui-hint", t("catalog.shareHint"))); dialog.open();
  } }];
}
export async function confirmRemoval(kind: Kind, id: string): Promise<boolean> {
  if (!shared(kind, id)) return true;
  return confirmDialog({ title: t("catalog.delete"), message: t("catalog.deleteHint"), accept: t("catalog.delete"), cancel: t("actions.cancel") });
}
