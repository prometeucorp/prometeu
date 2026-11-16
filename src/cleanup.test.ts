import { beforeEach, afterEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), dialog: vi.fn(), toggle: vi.fn() }));
const element = () => ({ append() {}, setAttribute() {}, dataset: {}, classList: { add() {} }, querySelector: () => element() });
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));
vi.mock("./util", () => ({ h: () => element(), template: () => element() }));
vi.mock("./ui", () => ({ formDialog: mocks.dialog, toggle: mocks.toggle }));
vi.mock("./i18n", () => ({ current: () => "en", fromBack: String, t: (key: string) => key, tn: () => "Cleaned" }));
import { autoCleanupEnabled, cleanupAfterArchive, cleanupSettingsRow } from "./cleanup";

const say = vi.fn();
let stored: string | null;
beforeEach(() => {
  vi.resetAllMocks();
  stored = null;
  vi.stubGlobal("localStorage", {
    getItem: () => stored,
    setItem: (_key: string, value: string) => { stored = value; },
  });
  mocks.dialog.mockImplementation(() => ({ root: { ...element(), isConnected: false }, body: element(), save: {}, open() {} }));
  mocks.invoke.mockResolvedValue([]);
});
afterEach(() => vi.unstubAllGlobals());

test.each([null, "false", "invalid"])("old or disabled preference %s keeps the scoped dialog", async value => {
  stored = value;
  await cleanupAfterArchive(say, "target");
  expect(mocks.dialog).toHaveBeenCalled();
  expect(mocks.invoke).not.toHaveBeenCalledWith("cleanup_worktree", expect.anything());
});

test("setting persists explicit opt-in and can be disabled", () => {
  const control = { ...element(), checked: false, onchange: () => {} };
  mocks.toggle.mockReturnValue({ control, label: element() });
  cleanupSettingsRow(say);
  control.checked = true; control.onchange();
  expect(autoCleanupEnabled()).toBe(true);
  control.checked = false; control.onchange();
  expect(autoCleanupEnabled()).toBe(false);
});

test("automatic cleanup targets only the archived workspace and never forces", async () => {
  stored = "true";
  mocks.invoke.mockResolvedValue([{ id: "other", sizeKb: 4096 }, { id: "target", sizeKb: 1024 }]);
  await cleanupAfterArchive(say, "target");
  expect(mocks.invoke.mock.calls).toEqual([["cleanup_list"], ["cleanup_worktree", { id: "target", force: false }]]);
  expect(mocks.dialog).not.toHaveBeenCalled();
  expect(say).toHaveBeenCalledWith("Cleaned");
});

test("blocked work still requires explicit dialog selection", async () => {
  stored = "true";
  mocks.invoke.mockResolvedValue([{ id: "target", blocked: "dirty" }]);
  await cleanupAfterArchive(say, "target");
  expect(mocks.dialog).toHaveBeenCalled();
  expect(mocks.invoke).not.toHaveBeenCalledWith("cleanup_worktree", expect.anything());
});

test("ineligible workspaces are not deleted", async () => {
  stored = "true";
  await cleanupAfterArchive(say, "missing");
  expect(mocks.invoke.mock.calls).toEqual([["cleanup_list"]]);
  expect(mocks.dialog).not.toHaveBeenCalled();
});

test.each(["cleanup_list", "cleanup_worktree"])("failure in %s reports error without forcing or claiming success", async command => {
  stored = "true";
  mocks.invoke.mockImplementation(async name => {
    if (name === command) throw new Error("Safety check failed");
    return [{ id: "target", sizeKb: 1024 }];
  });
  await cleanupAfterArchive(say, "target");
  expect(say.mock.calls).toEqual([["Error: Safety check failed", true]]);
  expect(mocks.invoke).not.toHaveBeenCalledWith("cleanup_worktree", { id: "target", force: true });
});

test("unavailable storage defaults to asking", () => {
  vi.stubGlobal("localStorage", { getItem() { throw new Error("Unavailable"); } });
  expect(autoCleanupEnabled()).toBe(false);
});
