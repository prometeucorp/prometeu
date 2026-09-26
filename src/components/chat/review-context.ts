import type { ReviewContext } from "../../review-context";
import { button, formDialog } from "../primitives";
import { h } from "../../util";
import { t, tn } from "../../i18n";
export function reviewContextChip(context: ReviewContext, remove?: () => void): HTMLElement {
  const chip = h("span", "review-context");
  const open = button(t("review.chip", { notes: tn(context.comments.length, "review.notes"), files: tn(new Set(context.comments.map(c => c.file)).size, "diff.files") }), () => {
    const dialog = formDialog({ title: t("review.details"), save: t("review.close"), cancel: t("review.close"), submit: async () => {}, error: String });
    dialog.save.remove(); dialog.root.classList.add("review-dialog");
    if (chip.closest(".ui-comfortable")) dialog.root.classList.add("ui-comfortable");
    if (remove) dialog.body.append(h("p", "ui-hint", t("review.snapshot")));
    for (const comment of context.comments) {
      const row = h("section", "review-detail"), span = comment.new ?? comment.old;
      row.append(h("strong", "", `#${comment.n} · ${comment.repo} · ${comment.file}`), h("p", "ui-hint", `${comment.in} · ${span ? t("review.range", { start: span[0], end: span[1] }) : t("review.file")}`), h("pre", "", comment.excerpt.join("\n")), h("p", "review-note-body", comment.body));
      dialog.body.append(row);
    }
    dialog.open();
  }, "ghost");
  open.setAttribute("aria-haspopup", "dialog"); chip.append(open);
  if (remove) { const close = button("×", remove, "ghost"); close.setAttribute("aria-label", t("review.remove")); chip.append(close); }
  return chip;
}
