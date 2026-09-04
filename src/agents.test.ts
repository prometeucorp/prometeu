import { describe, expect, it, vi } from "vitest";
import {
  capabilitiesOf,
  effortsOf,
  installed,
  isKnownModel,
  loadAgents,
  modelOf,
  providerOfModel,
  type AgentCapabilities,
} from "./agents";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));

const common: AgentCapabilities = {
  initialPlanMode: false,
  workspaceMcpSelection: true,
  workspacePluginSelection: true,
  resume: true,
  compact: true,
  contextReport: true,
  approvals: true,
  userQuestions: true,
  attachments: true,
};

describe("catálogo de agentes", () => {
  it("associa modelo, provider e capabilities sem a tela conhecer nomes", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "agents"
          ? {
              providers: [
                {
                  id: "claude",
                  label: "Claude",
                  installed: true,
                  models: [{ id: "claude-novo", label: "Claude novo", efforts: ["high"] }],
                  capabilities: {
                    ...common,
                    initialPlanMode: true,
                  },
                },
                {
                  id: "codex",
                  label: "Codex",
                  installed: true,
                  models: [
                    {
                      id: "gpt-teste",
                      label: "GPT de teste",
                      efforts: ["low", "high", "ultra"],
                    },
                  ],
                  capabilities: common,
                },
              ],
            }
          : [],
      ),
    );

    await loadAgents();

    expect(installed().map(({ id }) => id)).toEqual(["claude", "codex"]);
    expect(providerOfModel("gpt-teste")).toBe("codex");
    expect(modelOf("gpt-teste", "codex")?.label).toBe("GPT de teste");
    expect(isKnownModel("claude", "opus[1m]")).toBe(true);
    expect(effortsOf("codex", "gpt-teste")).toEqual(["low", "high", "ultracode"]);
    expect(capabilitiesOf("codex").workspacePluginSelection).toBe(true);
    expect(capabilitiesOf("claude").initialPlanMode).toBe(true);
  });
});
