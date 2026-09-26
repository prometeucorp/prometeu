import { button, checkbox, formDialog } from "../primitives";
import { h } from "../../util";
import { t } from "../../i18n";
import { reviewNote, type ReviewActions, type ReviewEntry } from "./review-note";
import type { ReviewNote, ReviewEditorDraft } from "../../review-comments";

export function reviewSummary(options: ReviewActions & {
  entries: ReviewEntry[]; blocked: boolean;
  drafts: ReviewEditorDraft[]; resume: (draft: ReviewEditorDraft) => void;
  validate: (notes: ReviewNote[]) => string | null;
  send: (at: HTMLElement, notes: ReviewNote[], close: () => void) => void;
  resolveChanged: (notes: ReviewNote[]) => void;
  closed: () => void;
}) {
  const dialog = formDialog({ title: t("review.title", { n: options.entries.length + options.drafts.length }), save: t("review.close"), cancel: t("review.close"), submit: async () => {}, error: String, closed: options.closed });
  dialog.save.remove(); dialog.root.classList.add("review-dialog");
  const selected = new Set(options.entries.filter(e => e.note.state === "draft").map(e => e.note.id));
  const include = checkbox(t("review.include"), false);
  const list = h("div", "review-summary-list"), error = h("p", "review-error"); error.setAttribute("role", "alert");
  const send = button("", () => options.send(send, chosen(), dialog.close), "pri");
  const changed = button(t("review.resolveChanged"), () => options.resolveChanged(options.entries.filter(e => e.changedSinceSent && e.note.state !== "resolved").map(e => e.note)), "ghost");
  const tools = h("div", "review-summary-tools"); tools.append(include.label, changed);
  dialog.body.append(h("p", "ui-hint", t("review.submitHint")), tools, list, error, send);
  const chosen = () => options.entries.filter(e => selected.has(e.note.id) && e.note.state !== "resolved").map(e => ({ ...e.note, anchor: e.placement.kind === "attached" ? e.placement.anchor : e.note.anchor }));
  function controls() {
    const notes = chosen(), reason = options.validate(notes);
    send.textContent = t("review.send", { n: notes.length });
    send.disabled = !notes.length || !!reason || options.blocked;
    error.textContent = options.blocked ? t("review.storage.read") : reason ?? "";
    changed.disabled = !options.entries.some(e => e.changedSinceSent && e.note.state !== "resolved") || options.blocked;
  }
  include.control.onchange = () => {
    for (const e of options.entries) if (e.note.state === "sent") include.control.checked ? selected.add(e.note.id) : selected.delete(e.note.id);
    paint();
  };
  function paint() {
    list.replaceChildren();
    for (const draft of options.drafts) {
      const pending = h("section", "review-summary-group");
      pending.append(h("strong", "", `${draft.anchor.repo} / ${draft.anchor.path}`), h("p", "review-note-body", draft.body),
        button(t("review.continue"), () => options.resume(draft), "ghost"));
      list.append(pending);
    }
    const groups = new Map<string, ReviewEntry[]>();
    for (const entry of options.entries) {
      const key = `${entry.note.anchor.repo} / ${entry.note.anchor.path}`;
      groups.set(key, [...(groups.get(key) ?? []), entry]);
    }
    for (const [name, entries] of groups) {
      const group = h("section", "review-summary-group"); group.append(h("strong", "", name));
      for (const entry of entries) {
        const pick = checkbox(t("review.select"), selected.has(entry.note.id)); pick.control.disabled = entry.note.state === "resolved" || options.blocked;
        pick.control.onchange = () => { pick.control.checked ? selected.add(entry.note.id) : selected.delete(entry.note.id); controls(); };
        group.append(pick.label, reviewNote(entry, options));
        if (entry.note.anchor.excerpt.length) group.append(h("pre", "review-detail", entry.note.anchor.excerpt.join("\n")));
        if (entry.note.sent) group.append(h("p", "ui-hint", t("review.round", { batch: entry.note.sent.batch, tab: entry.note.sent.tab, time: new Date(entry.note.sent.at).toLocaleString() })));
      }
      list.append(group);
    }
    if (!groups.size && !options.drafts.length) list.append(h("p", "ui-hint", t("review.empty")));
    controls();
  }
  paint(); dialog.open();
  return { close: dialog.close, update(entries: ReviewEntry[], drafts: ReviewEditorDraft[]) { options.entries = entries; options.drafts = drafts; paint(); } };
}
