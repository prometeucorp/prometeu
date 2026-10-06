import { icon } from "./icons";
import { fromBack, t } from "./i18n";
import { invoke } from "./ipc";
import { button, disclosure, field, radio, select, toggle } from "./ui";
import { h } from "./util";
import { makeNotice, readPreferences, savePreferences, type NoticeKind, type NoticePermission, type NoticeTone } from "./notifications";
import "./notifications.css";

export function settings(say: (text: string, error?: boolean) => void, compact = false): HTMLElement {
  let preferences = readPreferences();
  let permission: NoticePermission | null = null;
  let requestingPermission = false;
  let example: NoticeKind = "approval";
  const root = h("div", "notification-settings");
  const intro = h("div", "notification-intro");
  intro.append(h("h2", "", t("notifications.headline")), h("p", "ui-hint", t("notifications.intro")));
  const layout = h("div", "notification-layout");
  const form = h("div", "notification-form");
  const controls = document.createElement("fieldset");
  controls.className = "notification-controls";
  controls.setAttribute("aria-label", t("notifications.title"));
  const feedback = h("p", "ui-hint notification-feedback");
  feedback.setAttribute("role", "status");
  const fail = (error: unknown) => { feedback.textContent = fromBack(error); say(fromBack(error), true); };

  async function requestPermission() {
    if (requestingPermission) return;
    requestingPermission = true;
    paint();
    try {
      // Finish the initial read first so a stale result cannot overwrite the user's grant.
      await initialPermission;
      permission = await invoke("notification_permission", { request: true });
    }
    catch (error) { fail(error); }
    finally { requestingPermission = false; paint(); }
  }

  function save() {
    try {
      savePreferences(preferences);
      feedback.textContent = t("notifications.saved");
      void invoke("notification_dismiss").catch(fail);
    } catch (error) { preferences = readPreferences(); fail(error); }
    paint();
  }

  const enabled = toggle(t("notifications.enabled"), preferences.enabled);
  enabled.label.classList.add("notification-master");
  const masterCopy = enabled.label.querySelector("span")!;
  masterCopy.append(h("small", "", t("notifications.enabledHint")));
  enabled.control.onchange = () => {
    preferences.enabled = enabled.control.checked;
    save();
    if (preferences.enabled && preferences.style === "banner") void requestPermission();
  };
  const section = (title: string, hint?: string) => {
    const block = h("section", "notification-section");
    block.append(h("h2", "", title));
    if (hint) block.append(h("p", "ui-hint", hint));
    controls.append(block);
    return block;
  };
  const events = section(t("notifications.events"));
  const eventControls = (["approval", "done", "error", "github"] as const).map(kind => {
    const item = toggle(t(`notifications.${kind}`), preferences[kind]);
    item.label.classList.add("notification-row", kind);
    item.label.querySelector("span")!.append(h("small", "", t(`notifications.${kind}Hint`)));
    item.control.onchange = () => { preferences[kind] = item.control.checked; save(); };
    events.append(item.label);
    return { kind, ...item };
  });
  const styles = section(t("notifications.style"), t("notifications.styleHint"));
  const choices = h("div", "notification-styles");
  choices.setAttribute("role", "radiogroup");
  choices.setAttribute("aria-label", t("notifications.style"));
  const styleControls = (["banner", "notch", "none"] as const).map(style => {
    const choice = radio(t(`notifications.${style}`), "notification-style", style, preferences.style === style);
    choice.label.classList.add("notification-style");
    const screen = h("div", `notification-mini ${style}`);
    screen.setAttribute("aria-hidden", "true");
    screen.append(h("i", ""), h("b", ""));
    choice.label.prepend(screen);
    choice.control.onchange = () => {
      preferences.style = style; save();
      if (preferences.enabled && style === "banner") void requestPermission();
    };
    choices.append(choice.label);
    return { style, ...choice };
  });
  styles.append(choices);
  const sounds = h("section", "notification-section");
  controls.append(sounds);
  const sound = toggle(t("notifications.sound"), preferences.sound);
  sound.label.classList.add("notification-row");
  sound.label.querySelector("span")!.append(h("small", "", t("notifications.soundHint")));
  sound.control.onchange = () => { preferences.sound = sound.control.checked; save(); };
  const tone = select(preferences.tone, (["soft", "digital", "bell"] as const).map(key => [key, t(`notifications.${key}`)]));
  tone.onchange = () => { preferences.tone = tone.value as NoticeTone; save(); };
  const listen = button(t("notifications.listen"), () => {
    listen.disabled = true;
    void invoke("notification_sound", { tone: preferences.tone }).catch(fail).finally(() => paint());
  });
  const soundControls = h("div", "notification-sound-controls");
  soundControls.append(field(t("notifications.tone"), tone.root), listen);
  sounds.append(sound.label, soundControls);
  form.append(enabled.label, controls, feedback);

  const previewColumn = h("div", "notification-preview-column");
  const preview = h("aside", "notification-preview");
  preview.setAttribute("aria-label", t("notifications.preview"));
  const desktop = h("div", "notification-desktop");
  const camera = h("div", "notification-camera");
  const sample = h("div", "notification-sample");
  const logo = h("span", "notification-logo"); logo.innerHTML = icon("flame", 22);
  const sampleCopy = h("div", "notification-copy");
  const sampleTitle = h("strong", "");
  sampleCopy.append(sampleTitle, h("span", "", t("notifications.exampleWorkspace")));
  sample.append(logo, sampleCopy);
  const noVisual = h("p", "notification-no-visual", t("notifications.preview.none"));
  desktop.append(camera, sample, noVisual);
  const previewBody = h("div", "notification-preview-body");
  const description = h("p", "ui-hint");
  const event = select(example, (["approval", "done", "error"] as const).map(key => [key, t(`notifications.${key}`)]));
  event.onchange = () => { example = event.value as NoticeKind; paint(); };
  const test = button(t("notifications.test"), () => {
    test.disabled = true;
    void invoke("notification_show", { notice: makeNotice(example, null, t("notifications.exampleWorkspace"), preferences) })
      .then(() => { feedback.textContent = t("notifications.testSent"); })
      .catch(fail).finally(() => paint());
  }, "pri");
  const warning = h("div", "notification-permission");
  warning.setAttribute("role", "status");
  const warningText = h("p", "ui-hint");
  const allow = button(t("notifications.allow"), () => { void requestPermission(); });
  warning.append(warningText, allow);
  enabled.label.after(warning);
  previewBody.append(description, field(t("notifications.example"), event.root), test,
    h("p", "ui-hint", t("notifications.testHint")));
  preview.append(h("h3", "", t("notifications.preview")), desktop, previewBody);
  previewColumn.append(preview, h("p", "ui-hint notification-quiet", t("notifications.quiet")));
  layout.append(form, previewColumn);
  if (compact) {
    root.classList.add("compact");
    const details = disclosure(t("settings.notificationDetails"), layout);
    details.dataset.settingsDisclosure = "notifications";
    root.append(enabled.label, warning, details, feedback);
  } else root.append(intro, layout);

  function paint() {
    enabled.control.checked = preferences.enabled;
    enabled.control.disabled = requestingPermission;
    controls.disabled = !preferences.enabled || requestingPermission;
    eventControls.forEach(item => { item.control.checked = preferences[item.kind]; });
    styleControls.forEach(item => { item.control.checked = preferences.style === item.style; });
    sound.control.checked = preferences.sound;
    tone.value = preferences.tone;
    tone.control.disabled = !preferences.enabled || !preferences.sound;
    listen.disabled = !preferences.enabled || !preferences.sound;
    sample.dataset.style = preferences.style;
    sample.hidden = preferences.style === "none";
    noVisual.hidden = preferences.style !== "none";
    noVisual.textContent = t(preferences.sound ? "notifications.preview.none" : "notifications.noOutput");
    sampleTitle.textContent = t(`notifications.message.${example}`);
    description.textContent = t(`notifications.preview.${preferences.style}`);
    warning.hidden = !preferences.enabled || preferences.style !== "banner" || permission === "granted";
    warningText.textContent = t(permission === null || requestingPermission ? "notifications.permissionChecking"
      : permission === "unavailable" ? "notifications.permissionUnavailable"
      : permission === "default" ? "notifications.permissionRequired" : "notifications.permission");
    allow.hidden = permission !== "default";
    allow.disabled = requestingPermission;
    test.disabled = !preferences.enabled || !preferences[example]
      || (preferences.style === "none" && !preferences.sound)
      || (preferences.style === "banner" && permission !== "granted");
  }
  paint();
  const initialPermission = invoke("notification_permission", { request: false })
    .then(value => { permission = value; paint(); }).catch(fail);
  return root;
}
