import type { BackgroundContext } from "./background";

type Repos = { id: string; repos: readonly { worktree: string; base: string; name: string }[] };

/** A board update matters to Git only when the displayed workspace's repository set changes. */
export class GitRefreshPolicy {
  private key: string | null = null;

  consider(workspace: Repos): boolean {
    const key = JSON.stringify([workspace.id, workspace.repos.map(repo => [repo.worktree, repo.base, repo.name])]);
    if (key === this.key) return false;
    this.key = key;
    return true;
  }

  clear() { this.key = null; }
}

const foreground = (context: BackgroundContext) => context.visible && context.focused;
export const changesInterval = (context: BackgroundContext): number | null =>
  foreground(context) ? context.power === "ac" ? 5_000 : 10_000 : null;
export const marksInterval = (context: BackgroundContext): number | null =>
  foreground(context) ? context.power === "ac" ? 15_000 : 30_000 : null;
