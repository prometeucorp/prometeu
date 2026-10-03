import type { TerminalSnapshot } from "./terminal";
import type { RequestResponse } from "../conversation";
import type { Snapshot, Target } from "./session";
import type { Catalog, WorktreeRequest } from "./workspaces";

/** Native Windows host commands, including diagnostic preview operations. Shared application IPC is unchanged. */
export type Commands = {
  application_reconnect: { args: undefined; result: boolean };
  application_paths: { args: { paths: string[]; direction: "linux" | "windows" }; result: string[] };
  application_open: { args: { previous: Target | null }; result: Target };
  wsl_workspace_worktree: { args: { request: WorktreeRequest }; result: Catalog };
  wsl_workspace_list: { args: undefined; result: Catalog };
  wsl_workspace_create: { args: { title: string; path: string }; result: Catalog };
  wsl_workspace_select: { args: { id: string }; result: Catalog };
  wsl_workspace_stage: { args: { id: string; stage: string }; result: Catalog };
  wsl_connect: { args: { target: Target; connection: string }; result: boolean };
  wsl_start: { args: undefined; result: { generation: string; resuming: boolean } };
  wsl_snapshot: { args: undefined; result: Snapshot };
  wsl_send: { args: { text: string }; result: void };
  wsl_respond: { args: { requestId: string; response: RequestResponse }; result: void };
  wsl_stop: { args: undefined; result: void };
  wsl_terminal_open: { args: { cols: number; rows: number }; result: { id: string } };
  wsl_terminal_write: { args: { id: string; data: number[] }; result: void };
  wsl_terminal_resize: { args: { id: string; cols: number; rows: number }; result: void };
  wsl_terminal_snapshot: { args: { id: string }; result: TerminalSnapshot };
  wsl_terminal_acknowledge: { args: { id: string; seq: number }; result: void };
  wsl_terminal_close: { args: { id: string }; result: void };
  wsl_shutdown: { args: undefined; result: void };
  wsl_terminal_current: { args: undefined; result: TerminalSnapshot | null };
  wsl_disconnect: { args: undefined; result: void };
};
