import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Board, Workspace } from "./types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), confirm: vi.fn(), dialog: vi.fn(), row: vi.fn(), buttons: new Map<string, { run: () => void; disabled: boolean }>() }));
function element() {
  const children: unknown[] = [];
  return { children, append: (...nodes: unknown[]) => children.push(...nodes),
    replaceChildren: vi.fn((...nodes: unknown[]) => children.splice(0, children.length, ...nodes)),
    setAttribute() {}, remove() {}, contains: () => false, dataset: {}, textContent: "",
  };
}
vi.mock("./util", () => ({ h: () => element() }));
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));
vi.mock("./i18n", () => ({ t: (key: string) => key, fromBack: String }));
vi.mock("./components/compositions", () => ({ sectionHeader: () => element(), itemRow: mocks.row }));
vi.mock("./ui", () => ({ formDialog: mocks.dialog, confirmDialog: mocks.confirm,
  button: (_label: string, run: () => void, _variant: string, track: string) => {
    const control = { ...element(), run, disabled: false }; mocks.buttons.set(track, control); return control;
  },
}));
import { runningProcesses, showMachineProcesses, type MachineTerminal } from "./machine-processes";

const terminal = (id: string, running = true): MachineTerminal => ({ id, instance: "original-process", workspace: "workspace", pid: 123, unit: null, started: 100, running, code: null, seq: 0 });
const board: Board = { projects: [], workspaces: [{ id: "workspace", title: "Remote workspace", host: "vps",
  tabs: [{ id: "idle", title: "Idle conversation", status: "pronta" }, { id: "ended", title: "Ended conversation", status: "rodando" }],
} as Workspace] };

beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks(); mocks.buttons.clear();
  vi.stubGlobal("document", { activeElement: null });
  mocks.dialog.mockReturnValue({ root: element(), body: element(), save: element(), open() {} });
  mocks.row.mockImplementation(() => ({ root: element() }));
  mocks.invoke.mockImplementation(async (command: string) => command === "load_board" ? board : command === "machine_terminals" ? [terminal("idle")] : undefined);
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it("uses host liveness, includes idle and unadopted terminals, and never borrows names from another machine", () => {
  const terminals = [terminal("idle"), terminal("ended", false), terminal("orphan"), { ...terminal("claude-login-123"), workspace: null }];
  const rows = runningProcesses(terminals, board, "vps");
  expect(rows.map(row => row.id).sort()).toEqual(["claude-login-123", "idle", "orphan"]);
  expect(rows.find(row => row.id === "idle")).toMatchObject({ title: "Idle conversation", workspaceName: "Remote workspace" });
  expect(rows.find(row => row.id === "orphan")?.title).toBe("orphan");
  expect(rows.find(row => row.id === "claude-login-123")).toMatchObject({ title: "machines.processes.login", workspaceName: "machines.processes.noWorkspace" });
  expect(runningProcesses([terminal("idle")], board, "other")[0]).toMatchObject({ title: "idle", workspaceName: "workspace" });
  const unnamed = { ...board, workspaces: [{ ...board.workspaces[0], agent: "claude" as const, model: "", tabs: [{ ...board.workspaces[0].tabs[0], title: "" }] }] };
  expect(runningProcesses([terminal("idle")], unnamed, "vps")[0].title).toBe("model.claude");
});

it("requires confirmation, stops only the selected terminal, refreshes, and stops polling when closed", async () => {
  showMachineProcesses("vps", "VPS");
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.row).toHaveBeenCalledWith(expect.objectContaining({ title: "Idle conversation" }));
  mocks.confirm.mockResolvedValueOnce(false);
  mocks.buttons.get("machines.processes.stop")!.run();
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.invoke.mock.calls.some(([command]) => command === "machine_terminal_stop")).toBe(false);
  mocks.confirm.mockResolvedValueOnce(true);
  mocks.buttons.get("machines.processes.stop")!.run();
  mocks.buttons.get("machines.processes.stop")!.run();
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.invoke.mock.calls.filter(([command]) => command === "machine_terminal_stop")).toEqual([["machine_terminal_stop", { id: "vps", terminal: "idle", instance: "original-process" }]]);
  const beforePoll = mocks.invoke.mock.calls.length;
  await vi.advanceTimersByTimeAsync(3000);
  expect(mocks.invoke.mock.calls.length).toBeGreaterThan(beforePoll);
  mocks.dialog.mock.calls[0][0].closed();
  const afterClose = mocks.invoke.mock.calls.length;
  await vi.advanceTimersByTimeAsync(6000);
  expect(mocks.invoke).toHaveBeenCalledTimes(afterClose);
});

it("clears stale controls on a failed query and ignores results after closing", async () => {
  showMachineProcesses("vps", "VPS");
  await vi.advanceTimersByTimeAsync(0);
  const rows = mocks.dialog.mock.results[0].value.body.children[3];
  expect(rows.children).toHaveLength(1);
  mocks.invoke.mockRejectedValueOnce("offline");
  mocks.buttons.get("machines.processes.refresh")!.run();
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.buttons.get("machines.processes.refresh")!.disabled).toBe(false);
  expect(rows.children).toHaveLength(0);
  mocks.row.mockClear();
  mocks.buttons.get("machines.processes.refresh")!.run();
  mocks.dialog.mock.calls[0][0].closed();
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.row).not.toHaveBeenCalled();
});

it("reports a failed stop without claiming the process ended", async () => {
  showMachineProcesses("vps", "VPS");
  await vi.advanceTimersByTimeAsync(0);
  mocks.confirm.mockResolvedValueOnce(true);
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "machine_terminal_stop") throw "connection lost";
    return command === "load_board" ? board : [terminal("idle")];
  });
  mocks.buttons.get("machines.processes.stop")!.run();
  await vi.advanceTimersByTimeAsync(0);
  const body = mocks.dialog.mock.results[0].value.body;
  expect(body.children[2].textContent).toBe("connection lost");
  expect(body.children[3].children).toHaveLength(1);
  expect(mocks.buttons.get("machines.processes.stop")!.disabled).toBe(false);
  mocks.dialog.mock.calls[0][0].closed();
});

it("keeps the confirmed process identity when the conversation restarts during confirmation", async () => {
  showMachineProcesses("vps", "VPS");
  await vi.advanceTimersByTimeAsync(0);
  let confirm!: (accepted: boolean) => void;
  mocks.confirm.mockReturnValueOnce(new Promise<boolean>(resolve => { confirm = resolve; }));
  mocks.buttons.get("machines.processes.stop")!.run();
  const replacement = { ...terminal("idle"), instance: "replacement-process" };
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "machine_terminal_stop") throw "terminal_changed";
    return command === "load_board" ? board : [replacement];
  });
  await vi.advanceTimersByTimeAsync(6000);
  confirm(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.invoke).toHaveBeenCalledWith("machine_terminal_stop", { id: "vps", terminal: "idle", instance: "original-process" });
  expect(mocks.dialog.mock.results[0].value.body.children[2].textContent).toBe("terminal_changed");
  expect(mocks.buttons.get("machines.processes.stop")!.disabled).toBe(false);
  mocks.dialog.mock.calls[0][0].closed();
});

it("lists legacy hosts without allowing an unguarded stop", async () => {
  mocks.invoke.mockImplementation(async (command: string) => command === "load_board" ? board : [{ ...terminal("idle"), instance: undefined }]);
  showMachineProcesses("vps", "VPS");
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.row).toHaveBeenCalledWith(expect.objectContaining({ title: "Idle conversation", status: "machines.processes.unsupported" }));
  expect(mocks.buttons.get("machines.processes.stop")!.disabled).toBe(true);
  mocks.buttons.get("machines.processes.stop")!.run();
  expect(mocks.confirm).not.toHaveBeenCalled();
  expect(mocks.invoke.mock.calls.some(([command]) => command === "machine_terminal_stop")).toBe(false);
  mocks.dialog.mock.calls[0][0].closed();
});
