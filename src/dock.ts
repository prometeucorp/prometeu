import { invoke } from "./ipc";
import { Term } from "./term";
import { isTerm, type DockKind } from "./types";

/// Two terminal views keep setup/run output beside the shell in the center.
/// Agent conversations render separately in chat.ts.
const SKIN = { fontSize: 12, foreground: "#a4a09d", scrollback: 4000 };
const scripts = new Term(SKIN);
const shells = new Term(SKIN);

/// Setup and run use the side panel; shells use the center.
const view = (kind: DockKind) => (isTerm(kind) ? shells : scripts);
export type Where = "scripts" | "shell";
const at = (where: Where) => (where === "scripts" ? scripts : shells);

export function init(scriptsEl: HTMLElement, shellEl: HTMLElement) {
  const sink = (key: string, data: string | number[]) => {
    return invoke("pty_write", { session: key, ...(typeof data === "string" ? { data } : { bytes: data }) });
  };
  scripts.open(scriptsEl, sink);
  shells.open(shellEl, sink);
}

/// Use fallback dimensions when a workspace starts before its panel is visible.
const size = (where: Where) => {
  const { cols, rows } = at(where).dims();
  return { cols: cols || 80, rows: rows || 12 };
};
export const dims = () => size("scripts");

/// Reopening attaches to the existing process. The selected run name matters only on first start.
export async function open(workspace: string, kind: DockKind, name?: string) {
  await view(kind).attach(`${workspace}:${kind}`, invoke("open_dock", {
    id: workspace,
    kind,
    name: name ?? null,
    ...size(isTerm(kind) ? "shell" : "scripts"),
  }));
}

/// Show the retained output of an exited process without restarting it or accepting input.
export const show = (workspace: string, kind: DockKind) => view(kind).show(`${workspace}:${kind}`);

/// Navigation detaches both views without stopping their processes.
export function detach() {
  scripts.detach();
  shells.detach();
}

export const focus = (where: Where) => at(where).focus();

/// The active PTY key for external input routing.
export const currentKey = (where: Where) => at(where).current();

/// Only explicit closure kills a dock. Wait for completion before reopening,
/// so a replacement cannot attach to the process that is still shutting down.
export async function kill(workspace: string, kind: DockKind) {
  const term = view(kind);
  if (term.current() === `${workspace}:${kind}`) term.detach();
  await invoke("close_dock", { id: workspace, kind });
}
