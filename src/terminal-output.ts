import { invoke } from "./ipc";
import type { Snapshot } from "../packages/terminal-state/src/types";

export type TerminalOutput = {
  data: number[]; seq?: number; running?: boolean; generation?: string;
  screen?: Snapshot;
  queryReplies?: "host";
};
export const terminalSnapshot = async (session: string): Promise<TerminalOutput> => ({ data: await invoke("pty_buffer", { session }) });
