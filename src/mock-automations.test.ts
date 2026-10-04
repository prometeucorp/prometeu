import { describe, expect, it } from "vitest";
import { blankWorkflow, type Workflow } from "./automations-model";
import { simulate, validate } from "./mock-automations";

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
  it("rejects cycles before walking a graph", () => {
    const workflow = graph(); workflow.edges.push({ from: "approved", to: "decision", port: "next" });
    expect(validate(workflow).some(issue => issue.code === "cycle")).toBe(true);
    expect(() => simulate(workflow, { event: {}, outputs: {} })).toThrow("acyclic");
  });
});
