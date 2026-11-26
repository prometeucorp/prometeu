import { afterEach, beforeEach, expect, it, vi } from "vitest";

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
import { connectedComputers, showMachineClients, type MachineClient } from "./machine-clients";

const client = (device: string, connected: number, current = false, label = `Mac ${device}`): MachineClient => ({ device, label, connected, current });

beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks(); mocks.buttons.clear();
  mocks.dialog.mockReturnValue({ root: element(), body: element(), save: element(), open() {} });
  mocks.row.mockImplementation(() => ({ root: element() }));
  mocks.invoke.mockImplementation(async (command: string) => command === "machine_clients" ? [client("other", 200), client("this", 300, true)] : undefined);
});
afterEach(() => { vi.useRealTimers(); });

it("lists this computer first, then the others by connection time, naming unlabeled ones", () => {
  const rows = connectedComputers([client("late", 300), client("early", 100), client("this", 200, true), client("abcdef123456", 50, false, "")]);
  expect(rows.map(row => row.device)).toEqual(["this", "abcdef123456", "early", "late"]);
  expect(rows[1].name).toBe("abcdef12");
  expect(connectedComputers([client("", 1, false, "")])[0].name).toBe("machines.clients.unnamed");
});

it("offers disconnecting only other computers and asks before doing it", async () => {
  showMachineClients("vps", "VPS");
  await vi.waitFor(() => expect(mocks.row).toHaveBeenCalledTimes(2));
  expect(mocks.row.mock.calls.map(([row]) => [row.title, row.actions.length])).toEqual([["Mac this", 0], ["Mac other", 1]]);

  mocks.confirm.mockResolvedValueOnce(false);
  mocks.buttons.get("machines.clients.detach")!.run();
  await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(1));
  expect(mocks.invoke).not.toHaveBeenCalledWith("machine_client_detach", expect.anything());

  mocks.confirm.mockResolvedValueOnce(true);
  mocks.buttons.get("machines.clients.detach")!.run();
  await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("machine_client_detach", { id: "vps", device: "other" }));
});
