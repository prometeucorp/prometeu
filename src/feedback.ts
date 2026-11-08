import { track } from "./analytics";
import { mountFeedback } from "./feedback-client";
import { current, fromBack, t } from "./i18n";
import * as cloud from "./cloud";
import { invoke } from "./ipc";
import { $ } from "./util";
import { version } from "../package.json";

let widget: ReturnType<typeof mountFeedback> | undefined;

export function init() {
  widget = mountFeedback({
    source: "desktop", locale: current(), version, error: fromBack,
    origin: () => cloud.current().origin || "https://app.prometeu.co",
    send: async report => {
      await invoke("feedback_send", { report });
      track({ name: "feedback_action_used", channel: "private", action: "submit", has_attachment: !!report.image });
    },
    // Checked when the panel opens, so connecting an account and opening it again shows the form.
    blocked: () => cloud.current().user ? undefined
      : { message: t("feedback.needAccount"), label: t("feedback.connect"), run: cloud.connect },
    capture: async () => {
      const base64 = await invoke("feedback_capture");
      if (!base64) return;
      return new File([Uint8Array.from(atob(base64), char => char.charCodeAt(0))], "feedback.png", { type: "image/png" });
    },
  });
  widget.trigger.id = "feedback";
  widget.trigger.dataset.track = "feedback.open";
  widget.root.dataset.trackArea = "feedback";
  widget.root.addEventListener("click", event => {
    if ((event.target as Element | null)?.closest("[data-track=\"feedback.public\"]"))
      track({ name: "feedback_action_used", channel: "public", action: "open", has_attachment: false });
  });
  widget.trigger.addEventListener("click", () => {
    if (widget?.trigger.getAttribute("aria-expanded") === "true") track({ name: "feedback_action_used", channel: "private", action: "open", has_attachment: false });
  });
  widget.trigger.className = "ui-button ghost sm";
  widget.trigger.title = t("feedback.trigger");
  widget.trigger.setAttribute("aria-label", t("feedback.trigger"));
  widget.trigger.textContent = t("feedback.trigger");
  $("update").before(widget.trigger);
  widget.root.classList.add("ui-feedback-desktop");
}

/** Tauri intercepts native file drops, so load the selected image through bounded IPC. */
export function fileDropTarget(element: Element | null) {
  const panel = element?.closest<HTMLElement>(".ui-feedback-panel");
  if (!widget || !panel || !widget.root.contains(panel) || !widget.canAttach()) return null;
  const target = widget;
  return {
    host: panel,
    put: ([path]: string[]) => {
      if (!path || !target.canAttach()) return;
      void target.attach(invoke("feedback_image", { path })
        .then(image => new File(
          [Uint8Array.from(atob(image.data), char => char.charCodeAt(0))],
          image.name,
          { type: image.type },
        )));
    },
  };
}
