import { listen } from "@tauri-apps/api/event";
import { invoke } from "./ipc";

/** Observed native state. A newer event wins over a delayed initial IPC response. */
export type BackgroundContext = {
  visible: boolean;
  focused: boolean;
  power: "ac" | "battery" | "unknown";
  revision: number;
};

let state: BackgroundContext = { visible: false, focused: false, power: "unknown", revision: 0 };
let observed = false;
let starting: Promise<void> | null = null;
const subscribers = new Set<(next: BackgroundContext) => void>();

export const foreground = (value: BackgroundContext): boolean => value.visible && value.focused;
export const batteryBudget = (value: BackgroundContext): boolean => value.power !== "ac";
export const accept = (current: BackgroundContext, next: BackgroundContext): BackgroundContext =>
  next.revision >= current.revision ? next : current;

export const current = (): BackgroundContext => state;
export const hasObservation = (): boolean => observed;
/** The WebView still has focus/visibility signals if the native context cannot start. */
export const currentOrDocument = (): BackgroundContext => observed ? state : {
  ...state, visible: !document.hidden, focused: !document.hidden && document.hasFocus(),
};
export function subscribe(listener: (next: BackgroundContext) => void): () => void {
  subscribers.add(listener);
  return () => subscribers.delete(listener);
}

function update(next: BackgroundContext) {
  observed = true;
  const accepted = accept(state, next);
  if (accepted === state) return;
  state = accepted;
  for (const listener of subscribers) listener(state);
}

export function start(): Promise<void> {
  if (starting) return starting;
  starting = (async () => {
    await listen<BackgroundContext>("background-context", ({ payload }) => update(payload));
    update(await invoke("background_context"));
  })();
  return starting;
}
