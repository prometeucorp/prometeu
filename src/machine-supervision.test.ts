import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "./ipc";
import { connectionTestMessage, setupAgentConnection, showAgentConnection } from "./machine-supervision";
import { t } from "./i18n";
import { formDialog } from "./ui";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));
vi.mock("./cloud", () => ({ connect: vi.fn() }));
function element() { return { append() {}, setAttribute() {}, disabled: false, value: "" }; }
vi.mock("./util", () => ({ h: () => element() }));
vi.mock("./ui", () => ({
  button: () => element(), input: () => element(), field: () => element(),
  checkbox: (_label: string, checked: boolean) => ({ label: element(), control: { ...element(), checked } }),
  select: () => ({ root: element(), control: element(), native: element() }),
  formDialog: vi.fn(() => ({ body: element(), save: element(), open() {}, close() {} })),
}));
beforeEach(() => { vi.mocked(invoke).mockReset(); });

it("keeps the form's selected revision through stale retries and reads a fresh revision only after reopening", async () => {
  const account = "https://cloud.example#owner";
  let revision = "selected-revision";
  const saved = vi.fn();
  vi.mocked(formDialog).mockClear();
  mocks.invoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === "machine_supervision_options") return {
      account, supervisor: { id: "receiver", revision, name: "Receiver", url: "https://receiver.example/events",
        enabled: true, webhook_protocol: "openbot" },
    };
    if (command === "machine_supervision_enable") {
      const { setup } = args as { setup: { supervisor_revision?: string } };
      if (setup.supervisor_revision !== revision) throw new Error("receiver changed");
      return;
    }
    throw new Error(`Unexpected command: ${command}`);
  });

  showAgentConnection("vps", "VPS", true, saved);
  const dialog = vi.mocked(formDialog).mock.calls[0][0];
  await vi.waitFor(() => expect(vi.mocked(formDialog).mock.results[0].value.save.disabled).toBe(false));
  revision = "fresh-revision";
  for (let attempt = 0; attempt < 2; attempt++) {
    await expect(dialog.submit()).rejects.toThrow("receiver changed");
    expect(invoke).toHaveBeenLastCalledWith("machine_supervision_enable", {
      id: "vps", setup: { account, supervisor_id: "receiver", supervisor_revision: "selected-revision" },
    });
    expect(saved).not.toHaveBeenCalled();
    expect(vi.mocked(formDialog).mock.results[0].value.save.disabled).toBe(false);
  }
  expect(mocks.invoke.mock.calls.filter(([command]) => command === "machine_supervision_options")).toHaveLength(1);

  dialog.closed?.();
  showAgentConnection("vps", "VPS", true, saved);
  await vi.waitFor(() => expect(vi.mocked(formDialog).mock.results[1].value.save.disabled).toBe(false));
  await vi.mocked(formDialog).mock.calls[1][0].submit();
  expect(invoke).toHaveBeenLastCalledWith("machine_supervision_enable", {
    id: "vps", setup: { account, supervisor_id: "receiver", supervisor_revision: "fresh-revision" },
  });
  expect(saved).toHaveBeenCalledExactlyOnceWith(null, true);
  expect(mocks.invoke.mock.calls.filter(([command]) => command === "machine_supervision_options")).toHaveLength(2);
});

it("passes event consent with first MCP setup and leaves ordinary setup account-free", async () => {
  const setup = { config: "mcp-config", preview: "masked", moved_from: null };
  vi.mocked(invoke).mockResolvedValue(setup);
  expect(await setupAgentConnection("vps", false)).toBe(setup);
  expect(invoke).toHaveBeenLastCalledWith("machine_agents_enable", { id: "vps", rotate: false, supervision: undefined });
  const supervision = { account: "https://cloud.example#owner", supervisor_id: "receiver", supervisor_revision: "selected-revision" };
  await setupAgentConnection("vps", false, supervision);
  expect(invoke).toHaveBeenLastCalledWith("machine_agents_enable", { id: "vps", rotate: false, supervision });
});

it("adds events to an existing connection without rotating its MCP token", async () => {
  const setup = { account: "https://cloud.example#owner", supervisor_id: "receiver", supervisor_revision: "selected-revision" };
  expect(await setupAgentConnection("vps", true, setup)).toBeNull();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("machine_supervision_enable", { id: "vps", setup });
  vi.mocked(invoke).mockRejectedValue(new Error("offline"));
  await expect(setupAgentConnection("vps", true, setup)).rejects.toThrow("offline");
});

it("reports confirmed receipt only after the receiver acknowledges the host test", () => {
  expect(connectionTestMessage({ status: "delivered", failed: false })).toBe(t("machines.events.delivered"));
  for (const status of ["pending", "sending"] as const) {
    expect(connectionTestMessage({ status, failed: false })).toBe(t("machines.events.pending"));
    expect(connectionTestMessage({ status, failed: true })).toBe(t("machines.events.failed"));
  }
  expect(connectionTestMessage({ status: "cancelled", failed: false })).toBe(t("machines.events.failed"));
});
