import { invoke } from "@tauri-apps/api/core";
import type { IpcTransport, IpcResult, IpcArgs } from "../ipc";

const terminalWrites = new Map<string, Promise<unknown>>();

/** The native window transports the existing application contract to its WSL host. */
export const wslCommands: IpcTransport = {
  invoke: (command, args) => {
    const send = () => invoke<IpcResult<typeof command>>("application_request", { command, args: args ?? null }).catch(error => {
      throw applicationError(error);
    });
    if (command !== "pty_write") return send();
    const { session } = args as IpcArgs<"pty_write">;
    // Native requests use blocking workers whose lock acquisition can reorder keystrokes.
    // Preserve each terminal's byte order before crossing IPC. A failed prefix rejects its
    // already queued suffix so an incomplete command cannot silently become another command.
    const previous = terminalWrites.get(session);
    const result = previous ? previous.then(send) : send();
    terminalWrites.set(session, result);
    const complete = () => {
      if (terminalWrites.get(session) === result) terminalWrites.delete(session);
    };
    result.then(complete, complete);
    return result;
  },
};

function applicationError(error: unknown): unknown {
  if (typeof error !== "string" || !error.startsWith("application-error:")) return error;
  try { return JSON.parse(error.slice("application-error:".length)); }
  catch { return error; }
}
