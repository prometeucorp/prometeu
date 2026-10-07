import { invoke } from "./ipc";
import { t } from "./i18n";

export type NoticeKind = "approval" | "done" | "error" | "github";
export type NoticeStyle = "banner" | "notch" | "none";
export type NoticeTone = "soft" | "digital" | "bell";
export type NoticePermission = "granted" | "denied" | "default" | "unavailable";
export type Notice = {
  title: string;
  body: string;
  style: NoticeStyle;
  sound: NoticeTone | null;
  tab: string | null;
  openLabel: string;
  closeLabel: string;
};
export type NotificationPreferences = {
  version: 1;
  enabled: boolean;
  approval: boolean;
  done: boolean;
  error: boolean;
  github: boolean;
  style: NoticeStyle;
  sound: boolean;
  tone: NoticeTone;
};
export const NOTIFICATIONS_KEY = "prometeu:notifications";
export const defaults: NotificationPreferences = {
  version: 1, enabled: false, approval: true, done: true, error: true, github: true,
  style: "banner", sound: false, tone: "soft",
};

/** Old sound preferences stay inert. Invalid or future records cannot turn notifications on. */
export function readPreferences(): NotificationPreferences {
  try {
    const value = JSON.parse(localStorage.getItem(NOTIFICATIONS_KEY) ?? "null");
    if (!value || value.version !== 1) return { ...defaults };
    return {
      ...defaults,
      ...Object.fromEntries((["enabled", "approval", "done", "error", "github", "sound"] as const)
        .map(key => [key, typeof value[key] === "boolean" ? value[key] : defaults[key]])),
      style: ["banner", "notch", "none"].includes(value.style) ? value.style : defaults.style,
      tone: ["soft", "digital", "bell"].includes(value.tone) ? value.tone : defaults.tone,
    };
  } catch { return { ...defaults }; }
}

export function savePreferences(value: NotificationPreferences) {
  localStorage.setItem(NOTIFICATIONS_KEY, JSON.stringify(value));
}

export function makeNotice(kind: NoticeKind, tab: string | null, name: string, preferences = readPreferences()): Notice {
  return {
    title: t(`notifications.message.${kind}`), body: name.slice(0, 300),
    style: preferences.style, sound: preferences.sound ? preferences.tone : null,
    tab, openLabel: t("notifications.open"), closeLabel: t("notifications.dismiss"),
  };
}

/** Called only for new pending activity, never snapshots or replay. Read preferences at delivery. */
export async function deliver(kind: NoticeKind, tab: string, name: string) {
  const preferences = readPreferences();
  // The notch island already marks finished conversations; only requests open it.
  const style = preferences.style === "notch" && kind !== "approval" ? "none" : preferences.style;
  if (!preferences.enabled || !preferences[kind] || (style === "none" && !preferences.sound)) return;
  await invoke("notification_show", { notice: { ...makeNotice(kind, tab, name, preferences), style } });
}

/** GitHub activity from the Cloud feed; never called for the first load after startup. */
export async function deliverGitHub(title: string, body: string) {
  const preferences = readPreferences();
  if (!preferences.enabled || !preferences.github || (preferences.style === "none" && !preferences.sound)) return;
  await invoke("notification_show", { notice: { ...makeNotice("github", null, body, preferences), title: title.slice(0, 200) } });
}
