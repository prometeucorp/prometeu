import type { Tab, Workspace } from "./types";

/** Native slash syntax, excluding absolute paths such as /Users/file and //server/share. */
export function isNativeCommand(text: string): boolean {
  return /^\/[^\s/]*(?:\s|$)/u.test(text.trimStart());
}

/** Decision ownership belongs to the saved conversation, not the global provider catalog. */
export function decisionSurfaceOf(workspace: Workspace | undefined, tab: Tab | undefined): "terminal" | undefined {
  if (workspace?.remote) return tab?.decision_surface;
  const agent = tab?.choice?.agent ?? workspace?.agent;
  return (agent === "cursor" || agent === "antigravity") && tab?.conversation_mode === "terminal" ? "terminal" : undefined;
}
