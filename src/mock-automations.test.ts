import { describe, expect, it, vi } from "vitest";
import { blankWorkflow, type Workflow } from "./automations-model";
import { commands, simulate, validate } from "./mock-automations";

function graph(): Workflow {
  return {
    ...blankWorkflow("Branch routing"),
    nodes: [
      { id: "start", label: "Start", position: { x: 0, y: 0 }, config: { type: "trigger", event: "manual", baseline: "ignoreExisting" } },
      { id: "decision", label: "Ready?", position: { x: 1, y: 0 }, config: { type: "condition", path: "event.ready", operator: "truthy" } },
      { id: "approved", label: "Review", position: { x: 2, y: 0 }, config: { type: "approval", message: "Review the action" } },
      { id: "deferred", label: "Wait", position: { x: 2, y: 1 }, config: { type: "wait", seconds: 60 } },
    ],
    edges: [{ from: "start", to: "decision", port: "next" }, { from: "decision", to: "approved", port: "true" }, { from: "decision", to: "deferred", port: "false" }],
  };
}

describe("browser automation simulation", () => {
  it("persists CI-only grants for the reviewed scope and resets them on a scope change", async () => {
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) });
    try {
      const workflow = graph();
      workflow.scope = { projectId: "local", repository: "owner/repo", identity: "owner" };
      Object.assign(workflow.policy, { allowWrites: true, allowCommit: true, allowPush: true, requirePublishApproval: false, requireLocalChecks: false });
      const saved = await commands.automations_save({ workflow, expectedRevision: null });
      expect((await commands.automations_snapshot(undefined)).workflows[0].policy).toEqual(workflow.policy);
      const edited = await commands.automations_save({ workflow: { ...saved, name: "Updated name" }, expectedRevision: saved.revision });
      expect(edited.policy).toEqual(workflow.policy);
      const changed = await commands.automations_save({ workflow: { ...edited, enabled: true, scope: { ...edited.scope, repository: "owner/other" } }, expectedRevision: edited.revision });
      expect(changed.enabled).toBe(false);
      expect(changed.policy).toMatchObject({ allowWrites: false, allowCommit: false, allowPush: false, requirePublishApproval: true, requireLocalChecks: true });
    } finally { vi.unstubAllGlobals(); }
  });
  it("follows the actual condition and records approval without executing effects", () => {
    const result = simulate(graph(), { event: { ready: true }, outputs: {} });
    expect(result.effectsSuppressed).toBe(true);
    expect(result.steps.find(s => s.nodeId === "approved")?.output).toEqual({ approvalRequired: true });
    expect(result.steps.find(s => s.nodeId === "deferred")?.status).toBe("skipped");
    const other = simulate(graph(), { event: { ready: false }, outputs: {} });
    expect(other.steps.find(s => s.nodeId === "approved")?.status).toBe("skipped");
    expect(other.steps.find(s => s.nodeId === "deferred")?.output).toEqual({ seconds: 60 });
  });
  it("does not turn absent evidence into a false or successful decision", () => {
    const result = simulate(graph(), { event: {}, outputs: {} });
    expect(result.steps.find(s => s.nodeId === "decision")?.status).toBe("error");
    expect(result.steps.filter(s => ["approved", "deferred"].includes(s.nodeId)).every(s => s.status === "skipped")).toBe(true);
  });
  it("reports the native worker format and account requirements before saving", () => {
    const workflow = graph();
    workflow.scope.targets = [{ projectId: "project", repository: "owner/repo" }];
    workflow.nodes[2].config = { type: "agent", prompt: "Inspect checks", outputSchema: { type: "object", required: ["result"], properties: { result: { type: "string" } } } };
    expect(validate(workflow)).toEqual(expect.arrayContaining([
      expect.objectContaining({ code: "agent_schema", nodeId: "approved" }),
      expect.objectContaining({ code: "target" }),
    ]));
    workflow.scope.targets[0].identity = "reviewed-user";
    workflow.nodes[2].config.outputSchema = { type: "object", required: ["summary", "outcome"], properties: { summary: { type: "string" }, outcome: { type: "string" } } };
    expect(validate(workflow)).toEqual([]);
  });
  it("rejects cycles before walking a graph", () => {
    const workflow = graph(); workflow.edges.push({ from: "approved", to: "decision", port: "next" });
    expect(validate(workflow).some(issue => issue.code === "cycle")).toBe(true);
    expect(() => simulate(workflow, { event: {}, outputs: {} })).toThrow("acyclic");
  });
});
