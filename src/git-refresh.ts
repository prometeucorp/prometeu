import { batteryBudget, foreground, type BackgroundContext } from "./background";
import type { Status } from "./types";

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

/** A turn that settles in the displayed workspace may have created files or a pull request. */
export class TurnSettlePolicy {
  private last: { id: string; busy: boolean } | null = null;

  settled(workspace: { id: string; tabs: readonly { status: Status }[] }): boolean {
    const busy = workspace.tabs.some((tab) => tab.status === "rodando");
    const settled = this.last?.id === workspace.id && this.last.busy && !busy;
    this.last = { id: workspace.id, busy };
    return settled;
  }
}

export const changesInterval = (context: BackgroundContext): number | null =>
  foreground(context) ? batteryBudget(context) ? 10_000 : 5_000 : null;
export const marksInterval = (context: BackgroundContext): number | null =>
  foreground(context) ? batteryBudget(context) ? 30_000 : 15_000 : null;
