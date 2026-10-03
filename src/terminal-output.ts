import { invoke } from "./ipc";

export type TerminalOutput = { data: number[]; seq?: number; running?: boolean };
export interface TerminalSnapshots {
  read(session: string): Promise<TerminalOutput>;
}

let snapshots: TerminalSnapshots = {
  read: async session => ({ data: await invoke("pty_buffer", { session }) }),
};
export const useTerminalSnapshots = (source: TerminalSnapshots) => { snapshots = source; };
export const terminalSnapshot = (session: string) => snapshots.read(session);
