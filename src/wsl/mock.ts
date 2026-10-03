import type { TerminalPort, TerminalEvent, TerminalSnapshot } from "./terminal";
import type { RequestResponse } from "../conversation";
import type { Frame, SessionPort, Snapshot, Target } from "./session";
import type { Catalog, WorkspacePort, WorktreeRequest } from "./workspaces";
import type { Workspace } from "../types";

const emptyState = () => ({ generation: 0, running: false, seq: 0, text: "", terminalId: 0, terminalActive: false, terminalSeq: 0, terminalBytes: [] as number[] });
function workspace(id: string, title: string, path: string): Workspace {
  return { id, title, project: path, repo: path, repo_name: path.split("/").pop()!, worktree: path, branch: "",
    repos: [{ path, name: title, worktree: path, base: "", pr: null }], stage: "Preparando", archived: false, pinned: false,
    unread: false, agent: "codex", model: "", effort: "", mcp: null, plugins: null, skills: null, port: null,
    issue: null, cleaned: false, shared: false, audience: null, remote_control: false, preparing: false, failed: null,
    remote: null, tabs: [], active: null };
}

/** Deterministic browser composition. No IPC or platform detection. */
export class MockSession implements SessionPort, TerminalPort, WorkspacePort {
  private catalog: Catalog = { v: 1, active: "primary", board: { stages: ["Preparando", "Fazendo", "Code review", "Travado", "Feito"], projects: [], workspaces: [workspace("primary", "Project", "/home/user/project")] } };
  private states = new Map<string, ReturnType<typeof emptyState>>();
  private serial = 0;
  private terminalSerial = 0;
  async listWorkspaces() { return structuredClone(this.catalog); }
  async createWorkspace(title: string, path: string) {
    if (!title.trim() || !path.startsWith("/")) throw new Error("workspace_folder_invalid");
    this.catalog.board.workspaces.push(workspace(crypto.randomUUID(), title, path));
    return this.listWorkspaces();
  }
  async createWorktree(request: WorktreeRequest) {
    if (!request.branch.trim() || request.branch.startsWith("-")) throw new Error("workspace_branch_invalid");
    if (!request.base.trim()) throw new Error("workspace_base_invalid");
    if (this.catalog.board.workspaces.some(w => w.repo === request.path && w.branch === request.branch)) throw new Error("workspace_branch_invalid");
    const id = crypto.randomUUID();
    const created = workspace(id, request.title, request.path);
    created.branch = request.branch;
    created.worktree = `/preview/checkouts/${id}`;
    created.repos[0].worktree = created.worktree;
    created.repos[0].base = request.base;
    this.catalog.board.workspaces.push(created);
    return this.listWorkspaces();
  }
  async selectWorkspace(id: string) {
    if (!this.catalog.board.workspaces.some(w => w.id === id)) throw new Error("workspace_not_found");
    this.states.set(this.catalog.active, { generation: this.generation, running: this.running, seq: this.seq, text: this.text,
      terminalId: this.terminalId, terminalActive: this.terminalActive, terminalSeq: this.terminalSeq, terminalBytes: [...this.terminalBytes] });
    Object.assign(this, this.states.get(id) ?? emptyState());
    this.catalog.active = id;
    return this.listWorkspaces();
  }
  async setWorkspaceStage(id: string, stage: string) {
    const target = this.catalog.board.workspaces.find(w => w.id === id);
    if (!target || !this.catalog.board.stages.includes(stage)) throw new Error("workspace_stage_invalid");
    target.stage = stage;
    return this.listWorkspaces();
  }
  terminalSupported = true;
  private terminalListeners = new Set<(event: TerminalEvent) => void>();
  private terminalId = 0;
  private terminalActive = false;
  private terminalSeq = 0;
  private terminalBytes: number[] = [];
  subscribeTerminal(receive: (event: TerminalEvent) => void) { this.terminalListeners.add(receive); return () => { this.terminalListeners.delete(receive); }; }
  terminalEvent(event: TerminalEvent) { for (const receive of this.terminalListeners) receive(event); }
  async currentTerminal() { return this.terminalActive ? this.snapshotTerminal(String(this.terminalId)) : null; }
  async openTerminal(_cols: number, _rows: number) {
    this.terminalActive = true;
    this.terminalId = ++this.terminalSerial; this.terminalSeq = 0; this.terminalBytes = [];
    await this.writeTerminal(String(this.terminalId), Array.from(new TextEncoder().encode("Preview shell\r\n$ ")));
    return { id: String(this.terminalId) };
  }
  async writeTerminal(id: string, data: number[]) {
    this.terminalBytes.push(...data);
    this.terminalEvent({ kind: "output", id, seq: ++this.terminalSeq, data });
  }
  async resizeTerminal(_id: string, _cols: number, _rows: number) {}
  async snapshotTerminal(id: string): Promise<TerminalSnapshot> { return { id, data: [...this.terminalBytes], seq: this.terminalSeq, running: true, code: null }; }
  async acknowledgeTerminal(_id: string, _seq: number) {}
  async closeTerminal(id: string) { this.terminalActive = false; this.terminalEvent({ kind: "closed", id, code: 0 }); }
  receive: (frame: Frame) => void = () => {};
  generation = 0;
  running = false;
  seq = 0;
  text = "";
  connected = false;
  async connect(_target: Target, receive: (frame: Frame) => void) { this.receive = receive; this.connected = true; }
  async start() {
    this.running = true;
    this.generation = ++this.serial;
    this.seq = 0;
    this.event({ v: 1, type: "session.identity", at: 1, providerSession: "preview" });
    return { generation: String(this.generation), resuming: this.generation > 1 };
  }
  async snapshot(): Promise<Snapshot> { return { generation: this.running ? String(this.generation) : null, snapshot: { text: this.text, seq: this.seq }, providerSession: "preview", running: this.running, ready: this.running }; }
  async send(text: string) {
    this.event({ v: 1, type: "user.message", at: 1, content: [{ kind: "text", text }] });
    this.event({ v: 1, type: "assistant.block", at: 2, messageId: String(this.seq), index: 0, block: { kind: "text", text } });
    this.event({ v: 1, type: "turn.completed", at: 3, outcome: "ok", message: "", durationMs: null, costUsd: null });
  }
  async respond(requestId: string, _response: RequestResponse) { this.event({ v: 1, type: "request.closed", at: 4, requestId, outcome: "allowed" }); }
  async stop() { this.running = false; this.receive({ v: 1, generation: String(this.generation), lifecycle: "exited" }); }
  async shutdown() {
    await this.stop(); this.terminalActive = false;
    for (const state of this.states.values()) { state.running = false; state.terminalActive = false; }
  }
  async disconnect() { this.connected = false; this.terminalEvent({ kind: "disconnected" }); }
  event(event: NonNullable<Frame["event"]>) {
    this.text += JSON.stringify(event) + "\n";
    this.receive({ v: 1, generation: String(this.generation), seq: ++this.seq, event });
  }
}
