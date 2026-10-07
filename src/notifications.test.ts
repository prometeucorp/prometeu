import { afterEach, beforeEach, expect, it, vi } from "vitest";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("./ipc", () => ({ invoke }));
import { defaults, deliver, NOTIFICATIONS_KEY, readPreferences, savePreferences } from "./notifications";

let saved: Map<string, string>;
beforeEach(() => {
  invoke.mockClear();
  saved = new Map();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => saved.set(key, value),
  });
});
afterEach(() => vi.unstubAllGlobals());

it("keeps legacy, missing, malformed, and future preferences silent", async () => {
  saved.set("prometeu:som", "true");
  for (const value of ["null", "{", '{"version":2,"enabled":true}', '{"enabled":true}']) {
    saved.set(NOTIFICATIONS_KEY, value);
    expect(readPreferences()).toEqual(defaults);
    await deliver("done", "tab", "workspace");
  }
  expect(invoke).not.toHaveBeenCalled();
});

it("validates each field and persists explicit choices without changing legacy data", () => {
  saved.set("prometeu:som", "true");
  saved.set(NOTIFICATIONS_KEY, JSON.stringify({ version: 1, enabled: "true", done: false, style: "other", sound: 1 }));
  expect(readPreferences()).toEqual({ ...defaults, done: false });
  const chosen = { ...defaults, enabled: true, style: "notch" as const, tone: "bell" as const };
  savePreferences(chosen);
  expect(readPreferences()).toEqual(chosen);
  expect(saved.get("prometeu:som")).toBe("true");
});

it("filters events and visual/audio channels independently at delivery time", async () => {
  savePreferences({ ...defaults, enabled: true, done: false, style: "notch" });
  await deliver("done", "tab", "workspace");
  expect(invoke).not.toHaveBeenCalled();
  await deliver("approval", "tab", "workspace");
  expect(invoke).toHaveBeenLastCalledWith("notification_show", { notice: expect.objectContaining({ style: "notch", sound: null, tab: "tab", body: "workspace" }) });
  invoke.mockClear();
  // The island marks finished conversations itself; with the notch they only play the sound.
  savePreferences({ ...defaults, enabled: true, style: "notch" });
  await deliver("done", "tab", "workspace");
  expect(invoke).not.toHaveBeenCalled();
  savePreferences({ ...defaults, enabled: true, style: "notch", sound: true });
  await deliver("done", "tab", "workspace");
  expect(invoke).toHaveBeenLastCalledWith("notification_show", { notice: expect.objectContaining({ style: "none", sound: "soft" }) });
  invoke.mockClear();
  savePreferences({ ...defaults, enabled: true, style: "none" });
  await deliver("error", "tab", "workspace");
  expect(invoke).not.toHaveBeenCalled();
  savePreferences({ ...defaults, enabled: true, style: "none", sound: true, tone: "bell" });
  await deliver("error", "tab", "workspace");
  expect(invoke).toHaveBeenLastCalledWith("notification_show", { notice: expect.objectContaining({ style: "none", sound: "bell" }) });
});

it("propagates native delivery failures", async () => {
  savePreferences({ ...defaults, enabled: true });
  invoke.mockRejectedValueOnce(new Error("permission denied"));
  await expect(deliver("error", "tab", "workspace")).rejects.toThrow("permission denied");
});
