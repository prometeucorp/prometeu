import { beforeEach, describe, expect, it, vi } from "vitest";
import { capabilitiesOf, catalogOf, effortsOf, installed, isKnownModel, loadAgents, modelLabelOf, modelOf, modelsOf, onCatalogChange, refreshModels, type AgentCapabilities, type AgentModel } from "./agents";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));
const common: AgentCapabilities = { initialPlanMode: false, workspaceMcpSelection: true, workspacePluginSelection: true, resume: true, compact: true, contextReport: true, approvals: true, userQuestions: true, attachments: true, readOnlyProfile: true };
let models: Record<string, AgentModel[]>;
beforeEach(() => {
  models = { claude: [{ id: "opus", label: "Live Opus", efforts: ["high", "xhigh"] }], codex: [{ id: "shared", label: "GPT", efforts: ["none", "low", "ultra", "future"] }], antigravity: [{ id: "shared", label: "Gemini", efforts: [] }] };
  mocks.invoke.mockReset().mockImplementation(async (cmd, args) => cmd === "agents" ? { providers: Object.keys(models).map(id => ({ id, label: id, installed: true, models: [], capabilities: common, authMethods: [] })) } : { models: models[args.agent], fetchedAt: Date.now() });
});
async function loaded() { await loadAgents(); await Promise.all(installed().map(p => refreshModels(p.id))); }

describe("live model catalogs", () => {
  it("uses advertised labels and does not resurrect retired aliases or synthesize efforts", async () => {
    await loaded();
    expect(modelLabelOf("opus", "claude")).toBe("Live Opus");
    expect(modelLabelOf("opus[1m]", "claude")).toBe("Opus · 1M");
    expect(isKnownModel("claude", "opus[1m]")).toBe(false);
    expect(effortsOf("claude", "opus")).toEqual(["high", "xhigh"]);
    expect(effortsOf("codex", "shared")).toEqual(["none", "low", "ultra", "future"]);
    expect(effortsOf("antigravity", "shared")).toEqual([]);
    expect(capabilitiesOf("codex").resume).toBe(true);
  });
  it("keeps identical model IDs in their explicit provider", async () => {
    await loaded();
    expect(modelOf("shared", "codex")?.label).toBe("GPT");
    expect(modelOf("shared", "antigravity")?.label).toBe("Gemini");
  });
  it("distinguishes successful empty catalogs from failed refresh with previous data", async () => {
    await loaded();
    mocks.invoke.mockRejectedValue({ code: "err.modelsCatalog.timeout" });
    await refreshModels("claude", true);
    expect(catalogOf("claude").status).toBe("error");
    expect(catalogOf("claude").error).toBe("err.modelsCatalog.timeout");
    expect(modelsOf("claude")[0].id).toBe("opus");
    mocks.invoke.mockResolvedValue({ models: [], fetchedAt: Date.now() });
    await refreshModels("claude", true);
    expect(catalogOf("claude").status).toBe("ready");
    expect(modelsOf("claude")).toEqual([]);
  });
  it("notifies subscribers on updates and coalesces concurrent requests", async () => {
    await loaded();
    let resolve!: (v: unknown) => void;
    mocks.invoke.mockImplementation(() => new Promise(r => { resolve = r; }));
    const changes: string[] = [];
    const stop = onCatalogChange(() => changes.push(catalogOf("claude").status));
    const first = refreshModels("claude", true), second = refreshModels("claude", true);
    expect(first).toBe(second);
    await Promise.resolve();
    resolve({ models: [{ id: "new", label: "New", efforts: [] }], fetchedAt: Date.now() });
    await first;
    stop();
    expect(changes).toEqual(["loading", "ready"]);
    expect(modelOf("new", "claude")).toBeDefined();
  });
  it("ignores a late response from the previous account generation", async () => {
    let old!: (v: unknown) => void;
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((cmd, args) => cmd === "agent_models" && args.agent === "claude" ? new Promise(r => { old = r; }) : original(cmd, args));
    await loadAgents();
    mocks.invoke.mockImplementation(original);
    models.claude = [{ id: "new-account", label: "New", efforts: [] }];
    await loaded();
    old({ models: [{ id: "old-account", label: "Old", efforts: [] }], fetchedAt: Date.now() });
    await Promise.resolve();
    expect(modelOf("old-account", "claude")).toBeUndefined();
    expect(modelOf("new-account", "claude")).toBeDefined();
  });
  it("refreshes only after five minutes unless explicitly forced", async () => {
    await loaded();
    models.claude = [{ id: "next", label: "Next", efforts: [] }];
    await refreshModels("claude");
    expect(modelOf("opus", "claude")).toBeDefined();
    vi.spyOn(Date, "now").mockReturnValue(Date.now() + 301_000);
    await refreshModels("claude");
    expect(modelOf("next", "claude")).toBeDefined();
    vi.restoreAllMocks();
  });
  it("does not advertise optional capabilities when installation discovery fails", async () => {
    mocks.invoke.mockRejectedValue(new Error("offline"));
    await loadAgents();
    expect(capabilitiesOf("antigravity").resume).toBe(false);
    expect(installed().map(p => p.id)).not.toContain("antigravity");
    expect(modelsOf("claude")).toEqual([]);
  });
});
