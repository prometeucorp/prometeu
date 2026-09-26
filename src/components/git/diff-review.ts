import { button } from "../primitives";
import { t } from "../../i18n";
import { rows } from "./patch";
import type { Change } from "../../types";
import { anchorSelection, REVIEW_ROWS, type ReviewAnchor, type ReviewScope, type ReviewSide, type ReviewEditorDraft } from "../../review-comments";
import { reviewEditor, reviewNote, type ReviewActions, type ReviewEntry } from "./review-note";

export type DiffReview = ReviewActions & {
  scope: ReviewScope; reference: string; entries: ReviewEntry[];
  add: (anchor: ReviewAnchor, body: string) => void;
  closed: () => void;
  drafts: ReviewEditorDraft[];
  draftChanged: (anchor: ReviewAnchor, body: string | null) => void;
};
/** Interaction and inline presentation only. The caller owns notes and all effects. */
export function diffReview(body: HTMLElement, head: HTMLElement, repo: string, change: Change, options: () => DiffReview | undefined) {
  const all = rows(change.patch).slice(0, REVIEW_ROWS);
  let cursor = -1, origin = -1, side: ReviewSide = "both", dragging = false;
  let editor: ReturnType<typeof reviewEditor> | undefined;
  let cardSignature = "";
  let editingAnchor: ReviewAnchor | undefined;
  const fileButton = button(t("review.file"), () => open(), "ghost"); fileButton.classList.add("review-file");
  const count = document.createElement("span"); count.className = "review-count";
  head.append(fileButton, count);
  const available = () => !!options() && all.some(r => r.kind !== "hunk");
  const usable = (index: number) => all[index] && all[index].kind !== "hunk" && (side === "both" || all[index][side] !== null);
  const targets = () => [...body.querySelectorAll<HTMLElement>(".drow")];
  function at(index: number) {
    return targets().find(row => [row.dataset.reviewBoth, row.dataset.reviewBefore, row.dataset.reviewAfter].includes(String(index)));
  }
  function paint() {
    for (const row of targets()) {
      const indexes = [row.dataset.reviewBoth, row.dataset.reviewBefore, row.dataset.reviewAfter].filter(v => v !== undefined).map(Number);
      row.classList.toggle("review-selected", cursor >= 0 && indexes.some(n => n >= Math.min(origin, cursor) && n <= Math.max(origin, cursor)));
    }
  }
  function close() {
    editor?.root.remove(); editor = undefined;
    if (editingAnchor) options()?.draftChanged(editingAnchor, null);
    editingAnchor = undefined; body.focus({ preventScroll: true });
    options()?.closed();
  }
  function open(file = true, retained?: ReviewEditorDraft, focus = true) {
    const review = options(); if (!available() || !review) return;
    const value = retained?.body ?? editor?.root.querySelector("textarea")?.value ?? "";
    const anchor = retained?.anchor ?? anchorSelection(repo, change.path, review.scope, change.patch, file ? undefined : origin, file ? undefined : cursor, side, review.reference);
    editingAnchor = anchor;
    review.draftChanged(anchor, value);
    editor?.root.remove();
    editor = reviewEditor(anchor, value, text => { review.add(anchor, text); close(); }, close, body => review.draftChanged(anchor, body));
    const line = !file && at(Math.max(origin, cursor));
    if (line) line.after(editor.root); else body.prepend(editor.root);
    if (focus) editor.focus();
  }
  function choose(index: number, extend: boolean, nextSide: ReviewSide) {
    if (!extend || origin < 0 || side !== nextSide) { side = nextSide; origin = cursor = index; }
    else {
      const step = index < origin ? -1 : 1;
      let last = origin;
      for (let n = origin; n !== index + step && n >= 0 && n < all.length; n += step) {
        if (all[n].kind === "hunk") break;
        if (usable(n)) last = n;
      }
      cursor = last;
    }
    paint();
  }
  function target(event: Event): { index: number; side: ReviewSide } | null {
    const control = (event.target as Element).closest<HTMLElement>("[data-review-index]");
    return control && body.contains(control) ? { index: Number(control.dataset.reviewIndex), side: control.dataset.reviewSide as ReviewSide } : null;
  }
  body.addEventListener("click", event => {
    if (!available() || !(event.target as Element).closest(".review-add") || event.detail) return;
    const hit = target(event); if (!hit) return;
    choose(hit.index, event.shiftKey, hit.side); open(false);
  });
  body.addEventListener("pointerdown", event => {
    if (!available() || event.button !== 0 || !(event.target as Element).closest(".review-add")) return;
    const hit = target(event); if (!hit) return;
    event.preventDefault(); choose(hit.index, event.shiftKey, hit.side); dragging = true;
    body.setPointerCapture(event.pointerId);
  });
  body.addEventListener("pointermove", event => {
    if (!dragging) return;
    const cell = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>("[data-review-index]");
    if (cell && body.contains(cell) && cell.dataset.reviewSide === side) choose(Number(cell.dataset.reviewIndex), true, side);
  });
  body.addEventListener("pointerup", event => {
    if (!dragging) return;
    dragging = false; body.releasePointerCapture(event.pointerId); open(false);
  });
  body.addEventListener("pointercancel", () => { dragging = false; });
  body.addEventListener("keydown", event => {
    if (!available() || event.metaKey || event.ctrlKey || event.altKey || (event.target as Element).closest(".review-note, .review-editor")) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      if (cursor < 0) { side = body.classList.contains("dlayout-split") ? "after" : "both"; cursor = all.findIndex((_, i) => usable(i)); origin = cursor; }
      else {
        let next = cursor + step;
        while (next >= 0 && next < all.length && !usable(next)) {
          if (event.shiftKey && all[next].kind === "hunk") { next = cursor; break; }
          next += step;
        }
        if (usable(next)) choose(next, event.shiftKey, side);
      }
      paint();
      // Keep keyboard navigation inside the reader; scrolling the app root can hide its header.
      const row = at(cursor), host = body.closest<HTMLElement>(".dlist");
      if (row && host) {
        const bounds = host.getBoundingClientRect(), line = row.getBoundingClientRect();
        const top = bounds.top + head.getBoundingClientRect().height;
        if (line.top < top) host.scrollTop -= top - line.top;
        else if (line.bottom > bounds.bottom) host.scrollTop += line.bottom - bounds.bottom;
      }
    } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      if (!body.classList.contains("dlayout-split")) return;
      event.preventDefault(); side = event.key === "ArrowLeft" ? "before" : "after";
      if (!usable(cursor)) cursor = all.findIndex((_, i) => usable(i));
      origin = cursor; paint();
    } else if (event.key.toLowerCase() === "c" || event.key === "Enter") {
      event.preventDefault();
      if (cursor < 0) { side = body.classList.contains("dlayout-split") ? "after" : "both"; cursor = all.findIndex((_, i) => usable(i)); origin = cursor; }
      if (cursor >= 0) open(false);
    } else if (event.key === "Escape") { cursor = origin = -1; paint(); }
  });
  return {
    mount() {
      for (const cell of body.querySelectorAll<HTMLElement>(".dno[data-review-index]")) {
        const plus = button("+", undefined, "ghost"); plus.classList.add("review-add");
        plus.onclick = null; // The file body delegates activation and owns editor focus.
        plus.tabIndex = -1; plus.setAttribute("aria-label", t("review.add")); cell.append(plus);
      }
      cardSignature = "";
      this.sync();
    },
    sync() {
      const review = options(), enabled = available();
      fileButton.hidden = !enabled; count.hidden = !review;
      body.tabIndex = enabled ? 0 : -1;
      if (enabled) body.setAttribute("aria-label", t("review.keyboard", { path: change.path })); else body.removeAttribute("aria-label");
      for (const plus of body.querySelectorAll<HTMLButtonElement>(".review-add")) plus.hidden = !enabled;
      const retained = review?.drafts.find(d => d.anchor.repo === repo && d.anchor.path === change.path && d.anchor.scope === review.scope && d.anchor.reference === review.reference);
      if (editor && !retained) { editor.root.remove(); editor = undefined; editingAnchor = undefined; }
      if (editor && retained) {
        const text = editor.root.querySelector("textarea")!;
        if (document.activeElement !== text && text.value !== retained.body) text.value = retained.body;
      }
      if (!editor && retained && body.querySelector(".drow")) open(true, retained, false);
      const entries = (review?.entries ?? []).filter(e => e.note.anchor.repo === repo && e.note.anchor.path === change.path);
      count.textContent = entries.length ? String(entries.filter(e => e.note.state !== "resolved").length) : "";
      const signature = JSON.stringify(entries);
      if (signature === cardSignature) return;
      cardSignature = signature;
      const active = document.activeElement as HTMLElement | null;
      const focusedNote = active?.closest<HTMLElement>(".review-note")?.dataset.note;
      const focusedLabel = active?.textContent;
      for (const card of body.querySelectorAll(".review-note")) card.remove();
      if (!review) return;
      for (const entry of entries) {
        const card = reviewNote(entry, review);
        const a = entry.placement.kind === "attached" ? entry.placement.anchor : null;
        const index = a ? all.map((row, index) => ({ row, index })).reverse().find(({ row }) => row.kind !== "hunk" && ((a.new && row.after === a.new[1]) || (a.old && row.before === a.old[1])))?.index ?? -1 : -1;
        const row = index >= 0 ? at(index) : null;
        if (row) row.after(card); else body.prepend(card);
      }
      if (focusedNote) [...body.querySelectorAll<HTMLButtonElement>(`.review-note[data-note="${CSS.escape(focusedNote)}"] button`)].find(b => b.textContent === focusedLabel)?.focus({ preventScroll: true });
    },
  };
}
