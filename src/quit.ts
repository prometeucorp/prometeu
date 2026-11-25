import { getCurrentWindow } from "@tauri-apps/api/window";
import { exit } from "@tauri-apps/plugin-process";
import { t, tn } from "./i18n";
import { invoke } from "./ipc";
import type { Board } from "./types";
import { confirmDialog } from "./ui";

/// Ask before quitting, closing the main window or restarting for an update while work on this computer would be cut off (ADR 0097). Ending the app ends every agent and terminal it runs: conversations come back interrupted with Continue, while terminal commands and workspace preparation are simply lost.

/// Work that ending the app would cut off.
export type Pending = { conversations: number; preparing: number; terminals: number };

/// Count the tabs `Board::revive` marks interrupted after a restart and the workspaces whose preparation would fail. Shares of other people run on their owners' computers. Terminals come from the backend, which sees the commands they run.
export function pending(board: Board, terminals: number): Pending {
  // Remote machines keep running their workspaces after the app quits (ADR 0101).
  const local = board.workspaces.filter((ws) => !ws.remote && !ws.host);
  return {
    conversations: local.reduce((n, ws) => n + ws.tabs.filter((tab) => tab.status === "rodando" || tab.status === "querendo").length, 0),
    preparing: local.filter((ws) => ws.preparing).length,
    terminals,
  };
}

/// "2 conversations in progress and 1 terminal running a command", in the interface language; empty when nothing would be cut off.
export function summarize(work: Pending): string {
  const parts = [
    work.conversations ? tn(work.conversations, "quit.conversations") : "",
    work.preparing ? tn(work.preparing, "quit.preparing") : "",
    work.terminals ? tn(work.terminals, "quit.terminals") : "",
  ].filter(Boolean);
  const last = parts.pop() ?? "";
  return parts.length ? t("quit.list", { first: parts.join(", "), last }) : last;
}

export type Leaving = { kind: "quit" } | { kind: "restart"; version: string };

let board: () => Board = () => ({ projects: [], workspaces: [] });
/// One question at a time: a request made while it is open is refused, and the open question decides.
let asking = false;

/// Resolve whether the app may end now, asking only when local work would be cut off.
export async function allow(leaving: Leaving): Promise<boolean> {
  if (asking) return false;
  asking = true;
  try {
    return await ask(leaving);
  } catch {
    // A broken question must never trap the person in the app: it fails open, as ending it did before.
    return true;
  } finally {
    asking = false;
  }
}

async function ask(leaving: Leaving): Promise<boolean> {
  // Without the terminal count, the board still decides.
  const work = summarize(pending(board(), await invoke("running_terminals").catch(() => 0)));
  if (!work) return true;
  // A minimized window would hide the question.
  const main = getCurrentWindow();
  await main.unminimize().catch(() => {});
  await main.setFocus().catch(() => {});
  return confirmDialog(leaving.kind === "restart"
    ? { track: "update.restart.confirm", area: "update", title: t("quit.restart.title"), message: t("quit.restart.message", { work, version: leaving.version }), accept: t("quit.restart.accept"), cancel: t("actions.cancel") }
    : { track: "quit.confirm", area: "app", title: t("quit.title"), message: t("quit.message", { work }), accept: t("quit.accept"), cancel: t("actions.cancel") });
}

/// Quit from the menu or ⌘Q. The system's Quit item ends the app without a request it could hold, so the menu runs this instead.
export async function request() {
  if (await allow({ kind: "quit" })) await exit(0);
}

/// Closing the main window quits the app (ADR 0089). While a close listener exists, Tauri holds every close until it answers.
export function init(context: { board: () => Board }) {
  board = context.board;
  void getCurrentWindow().onCloseRequested(async (event) => {
    if (!(await allow({ kind: "quit" }))) event.preventDefault();
  });
}
