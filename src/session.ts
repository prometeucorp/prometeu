import { ChatView, type Ctx, type Info } from "./chat";
import * as team from "./team";
import { $ } from "./util";

/// Coordinate conversation attachment and interaction through ChatView, including question, plan, and permission cards.

const view = new ChatView();
let attachVersion = 0;

export function init(onError: (m: string) => void, info: () => Info, comments: Pick<Ctx, "comment" | "thread" | "newConversation"> = {}) {
  view.open($("chatwrap"), { say: onError, info, ...comments });
  team.setSink({
    live: (tab, bytes) => view.remoteWrite(tab, bytes),
    reset: (tab, bytes) => {
      if (tab === view.current()) view.attachRemote(tab, bytes);
    },
  });
}

/// For remote workspaces, conversation lines come from the relay instead of the local backend.
export async function attach(id: string, remote?: string): Promise<boolean> {
  const version = ++attachVersion;
  if (remote) {
    const r = await team.attach(remote, id);
    if (!r || version !== attachVersion) return false;
    view.attachRemote(id, r.bytes);
  } else {
    // Direct remote-to-local navigation bypasses workspace.leave, so release the old watcher and wait here.
    team.detach();
    await view.attach(id);
    if (version !== attachVersion || view.current() !== id) return false;
  }
  if (version !== attachVersion) return false;
  view.focus();
  return true;
}

export function detach() {
  attachVersion++;
  team.detach();
  view.detach();
}

/// Remove a draft when its tab leaves the board.
export const forget = (alive: Set<string>) => view.forget(alive);

export const currentSession = () => view.current();
export const focus = () => view.focus();
export const refresh = () => view.refresh();
export const fileDropTarget = () => view.fileDropTarget();
export const contextTarget = () => view.contextTarget();
export const quoteSelection = () => view.quoteSelection();
export const focusAnchor = (anchor: string) => view.focusAnchor(anchor);
