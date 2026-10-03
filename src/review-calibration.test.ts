import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as browserStore from "./mock-review-calibration";
import type { CalibrationRecord } from "./review-calibration";

const data = new Map<string, string>();
beforeEach(() => {
  data.clear();
  vi.stubGlobal("localStorage", { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => data.set(key, value) });
});
afterEach(() => vi.unstubAllGlobals());
const record = (): CalibrationRecord => ({ v: 1, at: 100, model: "jev-1.13.0", language: "en", answers: [{ id: "task_kind", outcome: "feature", confidence: 0.9 }], suggested: ["business_rule"], action: "answered", created: true, latency_ms: 12 });

describe("browser calibration IPC contract", () => {
  it("rejects arbitrary model/answer text and unknown fields before storing", () => {
    browserStore.setEnabled(true);
    const invalid = [
      { ...record(), model: "confidentialclientname-1.2.3" },
      { ...record(), model: "jev-123456789.1.0" },
      { ...record(), answers: [{ id: "task_kind", outcome: "private issue text", confidence: 0.9 }] },
      { ...record(), answers: [{ id: "task_kind", outcome: "feature", confidence: 0.9, text: "private issue text" }] },
      { ...record(), draft: "private draft" },
      { ...record(), suggested: ["business_rule", "business_rule"] },
    ];
    for (const value of invalid) expect(() => browserStore.append(1, value as CalibrationRecord)).toThrow("err.calibration.invalid");
    expect(browserStore.status().records).toBe(0);
    expect(data.get("mock:reviewCalibration")).not.toMatch(/confidentialclientname|private/);
  });

  it("keeps the same default-off, Clear and disable generation behavior as native storage", () => {
    browserStore.append(0, record());
    expect(data.size).toBe(0);
    browserStore.setEnabled(true);
    browserStore.append(1, record());
    expect(browserStore.status()).toMatchObject({ records: 1, created: 1, actions: { answered: 1 } });
    expect(browserStore.exportCsv()).toContain('""id"":""task_kind""');
    browserStore.clear();
    browserStore.append(1, record());
    browserStore.setEnabled(false);
    browserStore.setEnabled(true);
    browserStore.append(2, record());
    expect(browserStore.status().records).toBe(0);
  });

  it("reports damaged history and recovers only through Clear", () => {
    data.set("mock:reviewCalibration", JSON.stringify({ enabled: true, generation: 1, records: [{ ...record(), model: "private prose" }] }));
    expect(() => browserStore.status()).toThrow("err.calibration.storage");
    expect(() => browserStore.append(1, record())).toThrow("err.calibration.storage");
    expect(() => browserStore.exportCsv()).toThrow("err.calibration.storage");
    expect(browserStore.clear()).toMatchObject({ enabled: true, generation: 2, records: 0 });
  });
});
