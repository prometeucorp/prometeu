import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CalibrationRecord, CalibrationStatus } from "./review-calibration";

const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./ipc", () => ipc);
beforeEach(() => { vi.resetModules(); ipc.invoke.mockReset(); });

const status: CalibrationStatus = { enabled: true, generation: 1, records: 0, created: 0, actions: { answered: 0, handed_to_agent: 0, dismissed: 0, none: 0 } };
const record: CalibrationRecord = { v: 1, at: 10, model: "jev-1.13.0", language: "en", answers: [], suggested: [], action: "none", created: false, latency_ms: 3 };

describe("evaluation shell", () => {
  it("publishes the effective model without invalidating the review epoch", async () => {
    const shell = await import("./typesafe");
    const changed = vi.fn();
    shell.onChange(changed);
    ipc.invoke.mockResolvedValue({ model: "jev-1.14.0", answers: [] });
    await shell.port.evaluate({ context: "Fix the import", questions: [] });
    expect(shell.lastModel()).toBe("jev-1.14.0");
    expect(shell.currentEpoch()).toBe(0);
    expect(changed).toHaveBeenCalledWith(false);
  });

  it("ignores model diagnostics from a configuration that was replaced", async () => {
    const shell = await import("./typesafe");
    let reply!: (value: unknown) => void;
    ipc.invoke.mockImplementation((command: string) => command === "context_evaluate"
      ? new Promise(resolve => { reply = resolve; }) : Promise.resolve({ configured: true, enabled: false, problem: null }));
    const evaluation = shell.port.evaluate({ context: "Fix the import", questions: [] });
    await shell.setEnabled(false);
    reply({ model: "jev-1.14.0", answers: [] });
    await evaluation;
    expect(shell.lastModel()).toBeUndefined();
  });

  it("keeps calibration failures visible without changing evaluation availability", async () => {
    const shell = await import("./typesafe");
    ipc.invoke.mockResolvedValueOnce({ configured: true, enabled: true, problem: null });
    await shell.setEnabled(true);
    ipc.invoke.mockResolvedValueOnce(status);
    await shell.setCalibrationEnabled(true);
    expect(shell.calibrationPort.consent()).toBe(1);
    ipc.invoke.mockRejectedValueOnce('i18n:{"code":"err.calibration.storage"}');
    shell.calibrationPort.append(1, record);
    await vi.waitFor(() => expect(shell.calibrationError()).toContain("err.calibration.storage"));
    expect(shell.available()).toBe(true);
    expect(shell.calibrationPort.consent()).toBeNull();
  });

  it("does not let an older summary restore consent after Clear", async () => {
    const shell = await import("./typesafe");
    let reply!: (value: unknown) => void;
    ipc.invoke.mockImplementation((command: string) => command === "review_calibration_status"
      ? new Promise(resolve => { reply = resolve; }) : Promise.resolve({ ...status, generation: 2 }));
    const refresh = shell.refreshCalibration();
    await shell.clearCalibration();
    reply(status);
    await refresh;
    expect(shell.calibrationPort.consent()).toBe(2);
  });

  it("ignores an append failure from before a successful Clear", async () => {
    const shell = await import("./typesafe");
    ipc.invoke.mockResolvedValueOnce(status);
    await shell.setCalibrationEnabled(true);
    let reject!: (error: unknown) => void;
    ipc.invoke.mockImplementation((command: string) => command === "review_calibration_append"
      ? new Promise((_, fail) => { reject = fail; }) : Promise.resolve({ ...status, generation: 2 }));
    shell.calibrationPort.append(1, record);
    await shell.clearCalibration();
    reject('i18n:{"code":"err.calibration.storage"}');
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(shell.calibrationError()).toBeNull();
    expect(shell.calibrationPort.consent()).toBe(2);
  });

  it("ignores a failed append summary that finishes after Clear", async () => {
    const shell = await import("./typesafe");
    ipc.invoke.mockResolvedValueOnce(status);
    await shell.setCalibrationEnabled(true);
    let reject!: (error: unknown) => void;
    ipc.invoke.mockImplementation((command: string) => command === "review_calibration_status"
      ? new Promise((_, fail) => { reject = fail; }) : Promise.resolve({ ...status, generation: 2 }));
    shell.calibrationPort.append(1, record);
    await new Promise(resolve => setTimeout(resolve, 0));
    await shell.clearCalibration();
    reject('i18n:{"code":"err.calibration.storage"}');
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(shell.calibrationError()).toBeNull();
    expect(shell.calibrationPort.consent()).toBe(2);
  });

  it("keeps Clear authoritative when an older append completes during it", async () => {
    const shell = await import("./typesafe");
    ipc.invoke.mockResolvedValueOnce(status);
    await shell.setCalibrationEnabled(true);
    let appended!: () => void;
    let cleared!: (value: CalibrationStatus) => void;
    ipc.invoke.mockImplementation((command: string) => command === "review_calibration_append"
      ? new Promise<void>(resolve => { appended = resolve; }) : command === "review_calibration_clear"
        ? new Promise(resolve => { cleared = resolve; }) : Promise.resolve(status));
    shell.calibrationPort.append(1, record);
    const clear = shell.clearCalibration();
    appended();
    await new Promise(resolve => setTimeout(resolve, 0));
    cleared({ ...status, generation: 2 });
    await clear;
    expect(shell.calibrationPort.consent()).toBe(2);
    expect(shell.calibrationError()).toBeNull();
  });
});
