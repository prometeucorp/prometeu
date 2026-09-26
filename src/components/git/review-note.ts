import { button, input, field } from "../primitives";
import { h } from "../../util";
import { t, type Key } from "../../i18n";
import { bodyValid, type Placement, type ReviewAnchor, type ReviewNote } from "../../review-comments";
export type ReviewEntry = { note: ReviewNote; placement: Placement; changedSinceSent: boolean };
export type ReviewActions = { edit: (note: ReviewNote) => void; remove: (note: ReviewNote) => void; state: (note: ReviewNote, state: "draft" | "resolved") => void; jump?: (entry: ReviewEntry) => void };
export const reviewError = (error: unknown) => t((error instanceof Error && error.message.startsWith("review.") ? error.message : "review.limit") as Key);
export function anchorLabel(a: ReviewAnchor): string {
  const span = a.new ?? a.old;
  return span ? t("review.range", { start: span[0], end: span[1] }) : t("review.file");
}
export function reviewNote(entry: ReviewEntry, actions: ReviewActions): HTMLElement {
  const { note, placement, changedSinceSent } = entry;
  const box = h("article", "review-note"); box.dataset.note = note.id;
  const heading = h("div", "review-note-head");
  heading.append(h("strong", "", anchorLabel(placement.kind === "attached" ? placement.anchor : note.anchor)), h("span", "review-state", t(`review.${note.state}`)), h("span", "review-placement", t(changedSinceSent ? "review.sinceSent" : `review.${placement.kind}`)));
  box.append(heading, h("p", "review-note-body", note.body));
  const tools = h("div", "review-note-tools");
  if (actions.jump) tools.append(button(t("review.jump"), () => actions.jump!(entry), "ghost"));
  tools.append(button(t("review.edit"), () => actions.edit(note), "ghost"), button(t("review.delete"), () => actions.remove(note), "ghost"), button(t(note.state === "resolved" ? "review.reopen" : "review.resolve"), () => actions.state(note, note.state === "resolved" ? "draft" : "resolved"), "ghost"));
  if (note.state === "sent") tools.append(button(t("review.reopen"), () => actions.state(note, "draft"), "ghost"));
  box.append(tools); return box;
}
export function reviewEditor(anchor: ReviewAnchor, value: string, save: (body: string) => void | Promise<void>, cancel: () => void, changed: (body: string) => void = () => {}) {
  const root = h("div", "review-editor"), text = input(value, true);
  text.required = true;
  text.oninput = () => changed(text.value);
  const error = h("p", "review-error"); error.setAttribute("role", "alert");
  let saving = false;
  const submit = async () => {
    if (saving) return;
    if (!bodyValid(text.value)) { error.textContent = t("review.limit"); text.focus(); return; }
    saving = true; text.disabled = true;
    for (const control of tools.querySelectorAll("button")) control.disabled = true;
    try { await save(text.value); } catch (e) { error.textContent = reviewError(e); } finally {
      saving = false; text.disabled = false;
      for (const control of tools.querySelectorAll("button")) control.disabled = false;
    }
  };
  const tools = h("div", "review-note-tools");
  tools.append(button(t("review.save"), submit, "pri"), button(t("review.cancel"), cancel, "ghost"));
  root.append(h("strong", "", anchorLabel(anchor)), field(t("review.note"), text), error, tools);
  root.addEventListener("keydown", event => {
    event.stopPropagation();
    if (event.key === "Escape") { event.preventDefault(); cancel(); }
    else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); submit(); }
  });
  return { root, focus: () => text.focus() };
}
