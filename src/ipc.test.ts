import { expect, expectTypeOf, it, vi } from "vitest";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { invoke, useIpc, type IpcCall, type IpcHandlers, type IpcTransport } from "./ipc";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

it("infers results from command names and forwards arguments and options unchanged", async () => {
  vi.mocked(tauriInvoke).mockResolvedValue("file contents");
  const args = { id: "workspace", rel: "README.md" };
  const options = { headers: { "x-test": "contract" } };
  const result = invoke("read_file", args, options);
  expectTypeOf(result).toEqualTypeOf<Promise<string>>();
  expect(await result).toBe("file contents");
  expect(tauriInvoke).toHaveBeenCalledWith("read_file", args, options);

  // These cases are checked by tsc; they must never reach the transport.
  if (false) {
    // @ts-expect-error Command names must exist in the shared contract.
    invoke("missing_command");
    // @ts-expect-error Catalog refresh uses cloud_status with refresh: true.
    invoke("catalog_refresh");
    // @ts-expect-error Sending a message resumes stopped tabs through chat_send.
    invoke("resume_tab", { tab: "tab" });
    // @ts-expect-error Git uses the per-repository workspace_git_* commands.
    invoke("workspace_diff", { id: "workspace" });
    // @ts-expect-error Opening a workspace root uses reveal_path with an empty relative path.
    invoke("reveal", { id: "workspace" });
    // @ts-expect-error File writes require their full argument object.
    invoke("write_file");
    // @ts-expect-error Misspelled argument names are rejected.
    invoke("read_file", { id: "workspace", path: "README.md" });
    // @ts-expect-error Unknown properties are rejected even when required arguments exist.
    invoke("read_file", { ...args, path: "README.md" });
    // @ts-expect-error Argument values must match the command.
    invoke("pty_resize", { session: "terminal", cols: "80", rows: 24 });
    // @ts-expect-error Callers cannot invent a response type.
    invoke<number>("read_file", args);
    // @ts-expect-error Persisting security state requires an explicit state payload.
    invoke("team_security_set", {});
    // @ts-expect-error Mock responses use the same command result type.
    const invalidHandler: IpcHandlers["read_file"] = () => 42;
    void invalidHandler;
  }
});

it("keeps command arguments correlated when a caller chooses between commands", async () => {
  const forward = (call: IpcCall<"read_file" | "write_file">) => invoke(...call);
  vi.mocked(tauriInvoke).mockResolvedValue(undefined);
  const args = { id: "workspace", rel: "README.md", text: "updated", was: "original" };
  const result = forward(["write_file", args]);
  expectTypeOf(result).toEqualTypeOf<Promise<string | void>>();
  await result;
  expect(tauriInvoke).toHaveBeenCalledWith("write_file", args, undefined);

  if (false) {
    const command = "" as "read_file" | "write_file";
    // @ts-expect-error An unresolved command union requires a correlated argument tuple.
    invoke(command, { id: "workspace", rel: "README.md" });
    // @ts-expect-error Explicitly widening the command type cannot relax write arguments.
    invoke<"read_file" | "write_file">("write_file", { id: "workspace", rel: "README.md" });
    // @ts-expect-error The tuple accepted by forwarding helpers preserves the same correlation.
    forward(["write_file", { id: "workspace", rel: "README.md" }]);
  }
});

it("injects an application transport without changing screen command contracts", async () => {
  const calls: unknown[] = [];
  const port: IpcTransport = { invoke: async (command, args, options) => {
    calls.push([command, args, options]);
    throw new Error("remote failure");
  } };
  useIpc(port);
  try {
    await expect(invoke("chat_send", { session: "explicit-session", text: "Hello" })).rejects.toThrow("remote failure");
    expect(calls).toEqual([["chat_send", { session: "explicit-session", text: "Hello" }, undefined]]);
  } finally { useIpc({ invoke: tauriInvoke }); }
});
