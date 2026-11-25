import { invoke } from "./ipc";
import { fromBack } from "./i18n";
import type { TerminalSession } from "./types";

/// Terminal sessions: local shells owned by the app, not by a workspace or project (ADR 0096). The board lists them; this module knows which have a running shell and ends a session when its shell exits.

/// The person's name wins over the folder's name; the root folder shows its path.
export function title(session: TerminalSession): string {
  return session.name || session.folder.split("/").filter(Boolean).pop() || session.folder;
}

/// A session runs one shell, under the same dock key format as workspaces and projects.
export const keyOf = (id: string) => `${id}:terminal`;

/// The session whose shell a dock key names, if any.
export function sessionOf(key: string, sessions: TerminalSession[]): TerminalSession | null {
  return sessions.find((session) => key === keyOf(session.id)) ?? null;
}

type Ctx = {
  sessions: () => TerminalSession[];
  /// The session on screen, if any.
  current: () => string | null;
  show: (session: TerminalSession) => void;
  /// Leave the view of a session that ended.
  left: () => void;
  /// Redraw what shows which sessions are running.
  changed: () => void;
  say: (message: string, error?: boolean) => void;
  /// Choose the folder for "New terminal in folder…"; null when the person cancels.
  pick: () => Promise<string | null>;
};
let ctx: Ctx;
const live = new Set<string>();

export function init(context: Ctx) {
  ctx = context;
}

export const isLive = (id: string) => live.has(id);

/// The view starts or reattaches the shell at its own size. Its attachment can end after a newer view replaced it, whether or not the shell started, so only the backend says whether one runs.
export async function check(id: string) {
  const docks = await invoke("dock_state", { id }).catch(() => []);
  const running = docks.some((dock) => dock.alive);
  if (running === live.has(id)) return;
  if (running) live.add(id);
  else live.delete(id);
  ctx.changed();
}

/// Without a folder the session starts in the home folder. Nothing is registered as a project.
export async function create(folder: string | null = null) {
  try {
    ctx.show(await invoke("terminal_create", { folder }));
  } catch (error) {
    ctx.say(fromBack(error), true);
  }
}

export async function createIn() {
  try {
    const folder = await ctx.pick();
    if (folder) await create(folder);
  } catch (error) {
    ctx.say(fromBack(error), true);
  }
}

/// A null name returns the session to its folder's name.
export async function rename(id: string, name: string | null) {
  try {
    await invoke("terminal_rename", { id, name });
  } catch (error) {
    ctx.say(fromBack(error), true);
  }
}

/// × and `exit` end the session: the view leaves first, then the backend stops the shell and drops the record.
export async function close(id: string) {
  live.delete(id);
  if (ctx.current() === id) ctx.left();
  try {
    await invoke("terminal_close", { id });
  } catch (error) {
    ctx.say(fromBack(error), true);
  }
  ctx.changed();
}

/// A shell that exits on its own, by `exit` or a crash, ends its session as × does. Quitting the app reports nothing, so the session stays listed for the next start.
export function closed(key: string) {
  const session = sessionOf(key, ctx.sessions());
  if (session) void close(session.id);
}

/// Shells outlive a webview reload; ask which listed sessions still have one.
export async function sync() {
  await Promise.all(ctx.sessions().map((session) => check(session.id)));
}

/// Command-W ends the session on screen while its terminal has focus, as it closes a focused dock shell.
export function closeFocused(focused: boolean): boolean {
  const id = ctx.current();
  if (!id || !focused) return false;
  void close(id);
  return true;
}
