import { afterEach, expect, it, vi } from "vitest";
import { Recovery } from "./recovery";

afterEach(() => vi.useRealTimers());

it("coalesces attachment loss and retries restoration without replaying application input", async () => {
  vi.useFakeTimers();
  const connect = vi.fn().mockRejectedValueOnce("unavailable").mockResolvedValueOnce(true).mockResolvedValue(false);
  const snapshot = vi.fn().mockRejectedValueOnce("read interrupted").mockResolvedValue(undefined);
  const status = vi.fn();
  const recovery = new Recovery(connect, status);
  recovery.subscribe(snapshot);
  const pending = recovery.recover();
  expect(recovery.recover()).toBe(pending);
  await vi.runAllTimersAsync();
  await pending;
  expect(connect).toHaveBeenCalledTimes(3);
  expect(snapshot).toHaveBeenCalledTimes(2);
  expect(status).toHaveBeenLastCalledWith(false);
});

it("ignores stale disconnects and removes disposed views", async () => {
  const connect = vi.fn().mockResolvedValueOnce(false).mockResolvedValue(true);
  const recovery = new Recovery(connect, () => {});
  const snapshot = vi.fn();
  const stop = recovery.subscribe(snapshot);
  await recovery.recover();
  expect(snapshot).not.toHaveBeenCalled();
  stop();
  await recovery.recover();
  expect(snapshot).not.toHaveBeenCalled();
});

it("checks a second loss arriving while views restore", async () => {
  const connect = vi.fn().mockResolvedValue(true);
  const recovery = new Recovery(connect, () => {});
  let finish!: () => void;
  const snapshot = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; })).mockResolvedValue(undefined);
  recovery.subscribe(snapshot);
  const pending = recovery.recover();
  await Promise.resolve();
  recovery.recover();
  finish();
  await pending;
  expect(connect).toHaveBeenCalledTimes(2);
  expect(snapshot).toHaveBeenCalledTimes(2);
});
