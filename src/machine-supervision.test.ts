import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "./ipc";
import { connectionTestMessage, setupAgentConnection } from "./machine-supervision";
import { t } from "./i18n";

vi.mock("./ipc", () => ({ invoke: vi.fn() }));
vi.mock("./cloud", () => ({ connect: vi.fn() }));
beforeEach(() => { vi.mocked(invoke).mockReset(); });

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
