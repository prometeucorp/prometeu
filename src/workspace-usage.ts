import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { t, tn } from "./i18n";
import { onTelemetryCleared, type TelemetryInsights } from "./telemetry";
import { pending, tabLabel, type Board, type Workspace } from "./types";
import { button } from "./ui";
import { h } from "./util";
import * as menu from "./menu";
import { openUsagePanel, usagePanel } from "./components/chat/usage";
import { replayIsCurrent, usageLabel, workspaceUsageData, workspaceUsageLabel } from "./usage-presentation";

const identity = (board: Board, kind: string, id: string) => board.telemetry_ids?.[`${kind}:${id}`] ?? id;
let cleared = 0;
onTelemetryCleared(() => { cleared++; });

/** One header chip owns its snapshot and invalidates reads whenever the workspace changes. */
export class WorkspaceUsage {
  readonly root = button(t("usage.title"), () => this.open(), "ghost");
  private label = h("span", "workspace-usage-label", t("usage.title"));
  private workspace: Workspace | null = null;
  private board: Board = { stages: [], projects: [], workspaces: [] };
  private revision = 0;
  private signature = "";
  private timer = 0;
  private snapshot: TelemetryInsights | null = null;
  private error = false;
  private panel: HTMLElement | null = null;

  constructor() {
    this.root.classList.add("workspace-usage"); this.root.id = "workspace-usage";
    this.root.replaceChildren(this.label); this.root.hidden = true;
    this.root.setAttribute("aria-haspopup", "dialog"); this.root.setAttribute("aria-expanded", "false");
    void listen("telemetry-changed", () => this.scheduleRefresh());
    onTelemetryCleared(() => {
      this.revision++; this.snapshot = null; this.error = false; clearTimeout(this.timer);
      this.paint(); if (this.workspace) void this.refresh();
    });
  }

  update(workspace: Workspace, board: Board) {
    if (workspace.remote || pending(workspace)) return this.hide();
    const changed = this.workspace?.id !== workspace.id;
    if (changed) this.hide();
    this.workspace = workspace; this.board = board; this.root.hidden = false;
    const signature = JSON.stringify([identity(board, "workspace", workspace.id), workspace.tabs.map(tab => [tab.id, tab.status]), workspace.repos.map(repo => repo.pr?.number)]);
    this.paint();
    if (signature === this.signature) return;
    this.signature = signature;
    this.scheduleRefresh(changed ? 0 : 150);
  }

  hide() {
    this.revision++; this.workspace = null; this.snapshot = null; this.signature = ""; this.error = false;
    this.root.hidden = true; clearTimeout(this.timer);
    if (this.panel?.isConnected) menu.close(); this.panel = null;
  }

  private data() {
    if (!this.snapshot || !this.workspace) return { title: t("usage.title"), description: t(this.error ? "err.telemetry.storage" : "usage.loading") };
    const names = {
      conversations: new Map(this.workspace.tabs.map(tab => [identity(this.board, "conversation", tab.id), tabLabel(this.workspace!, tab)])),
      repositories: new Map(this.workspace.repos.map(repo => [identity(this.board, "repository", repo.path), repo.name])),
    };
    return workspaceUsageData(this.snapshot, names);
  }

  private paint() {
    const snapshot = this.snapshot;
    this.label.textContent = snapshot ? usageLabel(snapshot.usage) || tn(snapshot.summary.turns, "usage.turns") : t("usage.title");
    if (snapshot?.pullRequests.length === 1 && this.workspace?.repos.length === 1) this.label.textContent = `${t("usage.pr", { n: snapshot.pullRequests[0].pullRequest })} · ${this.label.textContent}`;
    this.root.title = snapshot ? `${t("usage.title")} · ${workspaceUsageLabel(snapshot)}` : t("usage.title");
    this.root.setAttribute("aria-label", this.root.title);
    if (this.panel?.isConnected) {
      const scroll = this.panel.scrollTop;
      this.panel.replaceChildren(...usagePanel(this.data()).childNodes); this.panel.scrollTop = scroll;
    }
  }

  private open() {
    if (!this.workspace) return;
    if (this.panel?.isConnected) { menu.close(); return; }
    this.root.setAttribute("aria-expanded", "true");
    this.panel = openUsagePanel(this.root, this.data(), () => { this.panel = null; this.root.setAttribute("aria-expanded", "false"); });
    void this.refresh();
  }

  /** Naming and PR observations can commit without a visible board change. */
  private scheduleRefresh(delay = 150) {
    clearTimeout(this.timer);
    if (!this.workspace) return;
    this.revision++;
    this.timer = window.setTimeout(() => void this.refresh(), delay);
  }

  private async refresh() {
    clearTimeout(this.timer);
    const workspace = this.workspace;
    if (!workspace) return;
    const revision = ++this.revision;
    const current = () => replayIsCurrent({ key: workspace.id, version: revision }, { key: this.workspace?.id ?? null, version: this.revision }, false);
    try {
      const snapshot = await invoke("telemetry_insights", { filter: { workspaceId: identity(this.board, "workspace", workspace.id) } });
      if (!current()) return;
      this.error = snapshot.summary.health.unavailable;
      this.snapshot = this.error ? null : snapshot;
    } catch { if (!current()) return; this.error = true; this.snapshot = null; }
    this.paint();
  }
}

/** Finishing and archiving keep their existing flow and include a read-only total after success. */
export async function announceWorkspaceUsage(workspace: Workspace, board: Board, say: (text: string) => void, finished: boolean) {
  if (workspace.remote) return;
  const revision = cleared;
  try {
    const insight = await invoke("telemetry_insights", { filter: { workspaceId: identity(board, "workspace", workspace.id) } });
    if (revision !== cleared || insight.summary.health.unavailable) return;
    say(t(finished ? "usage.finished" : "usage.archived", { workspace: workspace.title, usage: workspaceUsageLabel(insight) }));
  } catch { /* A summary cannot change an already completed workspace operation. */ }
}
