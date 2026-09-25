import { invoke } from "./ipc";
import type { ResourceItem } from "./resources/model";
import { fromBack, t } from "./i18n";
import { h } from "./util";
import * as ui from "./ui";
import * as catalog from "./catalog";

export type Skill = { id: string; description: string; content: string };
let hub: Skill[] = [];
let say = (_text: string, _bad?: boolean) => {};
let afterChange = async () => {};
const watchers = new Set<() => void>();
export const onChange = (fn: () => void) => { watchers.add(fn); return () => watchers.delete(fn); };
export const packageIds = () => new Set(hub.map(s => `skill-${s.id}`));
export const list = () => hub;
export function init(report: typeof say, onChanged: () => Promise<void>) {
  say = report;
  afterChange = onChanged;
  void refresh().catch(e => say(fromBack(e), true));
}
export async function refresh() { hub = await invoke("skill_hub"); for (const fn of watchers) fn(); }
async function changed() { await refresh(); await afterChange(); }

export function settingsActions() {
  return [{ label: t("skill.add"), run: () => editor(null) }];
}
const installing = new Set<string>();
export function resourceItems(): ResourceItem[] {
  return [
    ...hub.map((skill): ResourceItem => ({
      key: `skill-actions-${skill.id}`, id: skill.id, kind: "skills", description: skill.description,
      origin: catalog.tag("skills", skill.id), glyph: "sparkles", actions: [
        { label: t("actions.edit"), run: () => editor(skill) },
        ...catalog.resourceActions("skills", skill.id),
        { label: t("skill.remove"), danger: true, run: () => {
          void invoke("skill_remove", { id: skill.id }).then(changed).catch(e => say(fromBack(e), true));
        } },
      ],
    })),
    ...catalog.current().skills.filter(s => !s.installed).map((skill): ResourceItem => ({
      key: `skill-cloud-${skill.id}`, id: skill.id, kind: "skills", glyph: "sparkles", origin: t("catalog.cloud"),
      description: `${skill.description} · ${t("catalog.notInstalled")}`, busy: installing.has(skill.id),
      actions: [{ label: t("catalog.install"), run: () => {
        if (installing.has(skill.id)) return;
        installing.add(skill.id); watchers.forEach(fn => fn());
        void invoke("catalog_install_skill", { id: skill.id }).then(changed).catch(e => say(fromBack(e), true))
          .finally(() => { installing.delete(skill.id); watchers.forEach(fn => fn()); });
      } }],
    })),
    ...catalog.organizationResources("skills", say),
  ];
}
function editor(skill: Skill | null) {
  const revision = skill ? catalog.current().revision : null;
  const id = ui.input(skill?.id ?? ""); id.required = true; id.maxLength = 56;
  id.pattern = "[a-z0-9][a-z0-9-]{0,55}"; id.readOnly = !!skill;
  const description = ui.input(skill?.description ?? ""); description.required = true; description.maxLength = 2000;
  const content = ui.input(skill?.content ?? "", true); content.required = true; content.rows = 14; content.maxLength = 65536;
  const dialog = ui.formDialog({ title: t(skill ? "skill.edit" : "skill.add"), save: t("actions.save"), cancel: t("actions.cancel"), error: fromBack,
    submit: async () => {
      await invoke("skill_save", { revision, skill: { id: id.value.trim(), description: description.value.trim(), content: content.value } }); await changed();
    } });
  dialog.body.append(ui.field(t("skill.name"), id), ui.field(t("skill.description"), description),
    ui.field(t("skill.content"), content, t("skill.contentHint")),
    h("p", "ui-hint", t(skill && catalog.shared("skills", skill.id) ? "catalog.liveHint" : "catalog.privateHint")));
  dialog.open();
}
