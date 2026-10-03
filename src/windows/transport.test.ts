import { beforeEach, expect, it, vi } from "vitest";
const native = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: native }));
import { wslCommands } from "./transport";
beforeEach(() => { native.mockReset(); });
it("preserves structured catalog failures across the string-based runtime envelope", async () => {
  native.mockRejectedValue('application-error:{"code":"err.modelsCatalog.noAccount"}');
  await expect(wslCommands.invoke("agent_models", { agent: "codex" })).rejects.toEqual({ code: "err.modelsCatalog.noAccount" });
  expect(native).toHaveBeenCalledWith("application_request", { command: "agent_models", args: { agent: "codex" } });
});
it("keeps unstructured and invalid encoded failures recoverable", async () => {
  for (const error of ["runtime connection closed", "application-error:{invalid"]) {
    native.mockRejectedValue(error);
    await expect(wslCommands.invoke("load_board", undefined)).rejects.toBe(error);
  }
});
it("orders terminal bytes before native scheduling without blocking another terminal", async () => {
  let finish!: () => void;
  native.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
  native.mockResolvedValue(undefined);
  const first = wslCommands.invoke("pty_write", { session: "a", data: "prefix" });
  const second = wslCommands.invoke("pty_write", { session: "a", data: "suffix\r" });
  const other = wslCommands.invoke("pty_write", { session: "b", data: "independent" });
  expect(native.mock.calls.map(call => call[1].args.data)).toEqual(["prefix", "independent"]);
  finish();
  await Promise.all([first, second, other]);
  expect(native.mock.calls.map(call => call[1].args.data)).toEqual(["prefix", "independent", "suffix\r"]);
});
it("rejects queued terminal suffixes after an uncertain prefix without replaying them", async () => {
  let fail!: (reason: string) => void;
  native.mockImplementationOnce(() => new Promise<void>((_, reject) => { fail = reject; }));
  const first = wslCommands.invoke("pty_write", { session: "a", data: "prefix" });
  const second = wslCommands.invoke("pty_write", { session: "a", data: "suffix\r" });
  const results = Promise.allSettled([first, second]);
  fail("connection lost");
  expect(await results).toEqual([
    { status: "rejected", reason: "connection lost" },
    { status: "rejected", reason: "connection lost" },
  ]);
  expect(native).toHaveBeenCalledTimes(1);
  native.mockResolvedValue(undefined);
  await wslCommands.invoke("pty_write", { session: "a", data: "new input" });
  expect(native).toHaveBeenCalledTimes(2);
});
