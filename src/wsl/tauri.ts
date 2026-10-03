import type { TerminalEvent, TerminalPort } from "./terminal";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { RequestResponse } from "../conversation";
import type { Commands } from "./ipc";
import type { Frame, SessionPort, Target } from "./session";
import type { WorkspacePort, WorktreeRequest } from "./workspaces";

function call<K extends keyof Commands>(name: K, args: Commands[K]["args"]): Promise<Commands[K]["result"]> {
  return invoke(name, args);
}
export class TauriSession implements SessionPort, TerminalPort, WorkspacePort {
  createWorktree(request: WorktreeRequest) { return call("wsl_workspace_worktree", { request }); }
  listWorkspaces() { return call("wsl_workspace_list", undefined); }
  createWorkspace(title: string, path: string) { return call("wsl_workspace_create", { title, path }); }
  selectWorkspace(id: string) { return call("wsl_workspace_select", { id }); }
  setWorkspaceStage(id: string, stage: string) { return call("wsl_workspace_stage", { id, stage }); }
  terminalSupported = false;
  private terminalListeners = new Set<(event: TerminalEvent) => void>();
  subscribeTerminal(receive: (event: TerminalEvent) => void) { this.terminalListeners.add(receive); return () => { this.terminalListeners.delete(receive); }; }
  private terminalEvent(event: TerminalEvent) { for (const receive of this.terminalListeners) receive(event); }
  currentTerminal() { return call("wsl_terminal_current", undefined); }
  openTerminal(cols: number, rows: number) { return call("wsl_terminal_open", { cols, rows }); }
  writeTerminal(id: string, data: number[]) { return call("wsl_terminal_write", { id, data }); }
  resizeTerminal(id: string, cols: number, rows: number) { return call("wsl_terminal_resize", { id, cols, rows }); }
  snapshotTerminal(id: string) { return call("wsl_terminal_snapshot", { id }); }
  acknowledgeTerminal(id: string, seq: number) { return call("wsl_terminal_acknowledge", { id, seq }); }
  closeTerminal(id: string) { return call("wsl_terminal_close", { id }); }
  private unlisten?: UnlistenFn;
  async connect(target: Target, receive: (frame: Frame) => void) {
    const connection = crypto.randomUUID();
    this.unlisten = await listen<{ connection: string; frame: Frame & { terminal?: TerminalEvent } }>("wsl-runtime", ({ payload }) => {
      if (payload.connection !== connection) return;
      if (payload.frame.terminal) { this.terminalEvent(payload.frame.terminal); return; }
      if (payload.frame.lifecycle === "disconnected") { this.terminalSupported = false; this.terminalEvent({ kind: "disconnected" }); }
      receive(payload.frame);
    });
    try { this.terminalSupported = await call("wsl_connect", { target, connection }); }
    catch (error) { this.unlisten(); this.unlisten = undefined; throw error; }
  }
  start() { return call("wsl_start", undefined); }
  snapshot() { return call("wsl_snapshot", undefined); }
  send(text: string) { return call("wsl_send", { text }); }
  respond(requestId: string, response: RequestResponse) { return call("wsl_respond", { requestId, response }); }
  stop() { return call("wsl_stop", undefined); }
  shutdown() { return call("wsl_shutdown", undefined); }
  async disconnect() {
    this.terminalSupported = false; this.terminalEvent({ kind: "disconnected" });
    this.unlisten?.(); this.unlisten = undefined;
    await call("wsl_disconnect", undefined);
  }
}
