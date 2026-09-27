import { appendToDraft, canReview, createReview, type ReviewContext, type ReviewView } from "./context-review";
import { t, type Key } from "./i18n";
import * as typesafe from "./typesafe";
import { button, notice } from "./ui";
import { h } from "./util";

const NONE: Record<Extract<ReviewView, { phase: "none" }>["reason"], Key> = {
  clear: "review.none.clear",
  investigation: "review.none.investigation",
  out_of_scope: "review.none.outOfScope",
  uncertain: "review.none.uncertain",
  limit: "review.none.limit",
};

/// Launcher presentation of missing-context review. The action exists only while the integration is
/// configured and enabled; creation never waits for it. Answers and investigation requests are
/// appended to the draft as editable text, and `edited` lets the launcher record the new revision.
export function reviewControls(options: {
  panel: HTMLElement;
  prompt: HTMLTextAreaElement;
  context: () => ReviewContext;
  edited: () => void;
}) {
  const { panel, prompt } = options;
  const trigger = button(t("review.action"), () => void review.review(options.context()), "ghost");
  trigger.id = "d-review-go";
  trigger.title = t("review.action.title");
  panel.setAttribute("aria-live", "polite");

  const drawTrigger = () => {
    trigger.hidden = !typesafe.available();
    trigger.disabled = !canReview(options.context()) || review.view().phase === "evaluating";
  };

  const insert = (key: "review.answerBlock" | "review.investigateBlock") => {
    const suggestion = review.resolve(key === "review.answerBlock" ? "answered" : "handed_to_agent");
    if (!suggestion) return;
    prompt.value = appendToDraft(prompt.value, t(key, { question: t(suggestion.question) }));
    prompt.focus();
    prompt.setSelectionRange(prompt.value.length, prompt.value.length);
    options.edited();
  };

  const render = (view: ReviewView) => {
    drawTrigger();
    panel.hidden = view.phase === "idle";
    panel.replaceChildren();
    const close = () => button(t("review.close"), () => { review.clear(); prompt.focus(); }, "ghost");
    if (view.phase === "evaluating") panel.append(h("p", "ui-hint", t("review.evaluating")));
    if (view.phase === "none") {
      const row = h("div", "launcher-review-row");
      row.append(h("p", "ui-hint", t(NONE[view.reason])), close());
      panel.append(row);
    }
    if (view.phase === "failed") {
      const row = h("div", "launcher-review-row");
      row.append(notice(t(`err.evaluation.${view.code}` as Key), "warning"), close());
      panel.append(row);
    }
    if (view.phase === "suggesting") {
      const question = h("p", "launcher-review-question", t(view.suggestion.question));
      question.id = "d-review-question";
      const actions = h("div", "launcher-review-actions");
      actions.setAttribute("role", "group");
      actions.setAttribute("aria-labelledby", question.id);
      const answer = button(t("review.answer"), () => insert("review.answerBlock"));
      const investigate = button(t("review.investigate"), () => insert("review.investigateBlock"), "ghost");
      const dismiss = button(t("review.dismiss"), () => { review.dismiss(); if (review.view().phase !== "suggesting") prompt.focus(); }, "ghost");
      actions.append(answer, investigate, dismiss);
      panel.append(h("p", "ui-hint", t("review.count", { n: view.index })), question);
      if (options.context().attachments) panel.append(h("p", "ui-hint", t("review.uninspected")));
      panel.append(actions);
    }
    if (view.phase === "suggesting" || view.phase === "none") {
      panel.append(h("p", "ui-hint", t("review.model", { model: view.model ?? t("review.model.unknown") })));
    }
  };

  const review = createReview({
    port: typesafe.port,
    epoch: typesafe.currentEpoch,
    available: typesafe.available,
    changed: render,
    calibration: typesafe.calibrationPort,
  });
  // Enabling, disabling or changing the key invalidates pending and shown results.
  const forget = typesafe.onChange(configuration => { if (configuration) review.reset(); drawTrigger(); });
  drawTrigger();

  return {
    trigger,
    /// Call after any change to the draft, issue, project, base or attachments.
    update() { review.update(options.context()); drawTrigger(); },
    /// Closing or submitting the launcher discards anything still pending.
    close(creation: Promise<boolean> = Promise.resolve(false)) {
      review.close(); forget();
      void creation.then(created => review.finish(created), () => review.finish(false));
    },
  };
}
