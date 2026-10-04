import { describe, expect, it } from "vitest";
import catalog from "./automation-catalog.json";
import contract from "../fixtures/automation-contract.json";
import { t, use, type Key } from "./i18n";
import { localizeBuiltinTemplate, automationTransportUnavailable, pendingApprovalNode, publicationEvidence, blankWorkflow, connectNodes, newNode, outputPorts, removeNode, workflowDiff, type Workflow, type WorkflowNode, type AutomationRun } from "./automations-model";

function graph(): Workflow {
  const nodes: WorkflowNode[] = [
    { id: "event", label: "Event", position: { x: 0, y: 0 }, config: { type: "trigger", event: "manual", baseline: "ignoreExisting" } },
    { id: "condition", label: "Ready?", position: { x: 0, y: 100 }, config: { type: "condition", path: "event.ready", operator: "truthy" } },
    { id: "agent", label: "Inspect", position: { x: 0, y: 200 }, config: { type: "agent", prompt: "Inspect the change." } },
    { id: "wait", label: "Wait", position: { x: 250, y: 200 }, config: { type: "wait", seconds: 30 } },
  ];
  return { ...blankWorkflow("Example"), nodes, edges: [{ from: "event", port: "next", to: "condition" }, { from: "condition", port: "true", to: "agent" }] };
}

describe("workflow graph editing", () => {
  it("replaces one output connection and preserves other branches without mutating the original", () => {
    const original = graph(); const branched = connectNodes(original, "condition", "false", "wait")!;
    const result = connectNodes(branched, "condition", "true", "wait")!;
    expect(result.edges).toContainEqual({ from: "condition", port: "false", to: "wait" });
    expect(result.edges.filter(e => e.from === "condition" && e.port === "true")).toEqual([{ from: "condition", port: "true", to: "wait" }]);
    expect(original.edges).toHaveLength(2);
  });
  it("rejects indirect cycles, self links, missing nodes, invalid ports, and incoming trigger links", () => {
    const workflow = graph();
    expect(connectNodes(workflow, "agent", "next", "condition")).toBeNull();
    expect(connectNodes(workflow, "agent", "next", "agent")).toBeNull();
    expect(connectNodes(workflow, "agent", "next", "absent")).toBeNull();
    expect(connectNodes(workflow, "condition", "next", "wait")).toBeNull();
    expect(connectNodes(workflow, "agent", "next", "event")).toBeNull();
  });
  it("removes incoming and outgoing edges when deleting a node", () => {
    const result = removeNode(graph(), "condition");
    expect(result.nodes.map(n => n.id)).toEqual(["event", "agent", "wait"]);
    expect(result.edges).toEqual([]);
  });
  it("distinguishes deterministic conditions and uncertain model output ports", () => {
    const workflow = graph();
    expect(outputPorts(workflow.nodes[1])).toEqual(["true", "false", "error"]);
    expect(outputPorts(workflow.nodes[2])).toContain("uncertain");
    expect(outputPorts(newNode("jev", "Classify", 0, []))).toContain("uncertain");
  });
  it("includes changed policy, scope, nodes and removed fields in proposal review", () => {
    const original = graph(); const proposed = structuredClone(original);
    proposed.policy.allowWrites = true; proposed.scope.repository = "owner/repository"; proposed.nodes[2].label = "Changed"; delete (proposed.policy as Partial<typeof proposed.policy>).maxRetries;
    const changes = workflowDiff(original, proposed);
    expect(changes).toContainEqual({ path: "policy.allowWrites", before: false, after: true });
    expect(changes).toContainEqual({ path: "scope.repository", before: undefined, after: "owner/repository" });
    expect(changes.find(change => change.path === "nodes")).toBeDefined();
    expect(changes).toContainEqual({ path: "policy.maxRetries", before: 2, after: undefined });
  });
});


describe("workflow runtime presentation", () => {
  it("distinguishes unavailable runtime commands from retryable transport errors", () => {
    expect(automationTransportUnavailable("application_command_unsupported: automations_snapshot")).toBe(true);
    expect(automationTransportUnavailable(new Error("Command automations_snapshot not found"))).toBe(true);
    expect(automationTransportUnavailable({ code: "application_operation_unsupported" })).toBe(true);
    expect(automationTransportUnavailable("Connection lost while sending automations_snapshot")).toBe(false);
    expect(automationTransportUnavailable("automation_store_corrupt")).toBe(false);
  });

  it("offers the first pending approval from the native run without a history node id", () => {
    const run = contract.waitingRun as AutomationRun;
    expect(run.status).toBe("awaitingApproval");
    expect(run.approvals).toEqual([]);
    expect(run.history.some(entry => entry.nodeId === "approval")).toBe(false);
    expect(pendingApprovalNode(run)?.id).toBe("approval");
  });

  it("uses the current native selection instead of a previous approval in history", () => {
    const run = structuredClone(contract.waitingRun) as AutomationRun;
    run.workflow.nodes.push({ id: "later", label: "Later approval", position: { x: 0, y: 0 }, config: { type: "approval", message: "Approve the next result" } });
    run.history.push({ sequence: 100, at: 105, nodeId: "approval", kind: "approved", message: "Previously approved" });
    run.approvals.push({ nodeId: "approval", eventKey: run.eventKey, actor: "local-user", approvedAt: 105 });
    run.completedPorts = { ...run.completedPorts, approval: "next" };
    run.pendingApprovalNodeId = "later";
    expect(pendingApprovalNode(run)?.id).toBe("later");
    delete run.pendingApprovalNodeId;
    expect(pendingApprovalNode(run)).toBeUndefined();
  });

  it("does not offer stale, completed, already approved or invalid selections", () => {
    const run = contract.waitingRun as AutomationRun;
    expect(pendingApprovalNode({ ...run, status: "succeeded" })).toBeUndefined();
    expect(pendingApprovalNode({ ...run, pendingApprovalNodeId: "missing" })).toBeUndefined();
    expect(pendingApprovalNode({ ...run, pendingApprovalNodeId: "trigger" })).toBeUndefined();
    expect(pendingApprovalNode({ ...run, completedPorts: { ...run.completedPorts, approval: "next" } })).toBeUndefined();
    expect(pendingApprovalNode({ ...run, approvals: [{ nodeId: "approval", eventKey: run.eventKey, actor: "local-user", approvedAt: 105 }] })).toBeUndefined();
    expect(pendingApprovalNode(contract.run as AutomationRun)).toBeUndefined();
  });

  it("shows publication evidence only from recorded validation and commit nodes", () => {
    const workflow = graph();
    workflow.nodes.push({ id: "validation", label: "Validate", position: { x: 0, y: 300 }, config: { type: "action", operation: "workspace.validate", version: 1, inputs: {} } });
    workflow.nodes.push({ id: "commit", label: "Commit", position: { x: 0, y: 400 }, config: { type: "action", operation: "workspace.commit", version: 1, inputs: {} } });
    const run: AutomationRun = { id: "run", workflow, status: "awaitingApproval", event: {}, eventKey: "event", createdAt: 1, updatedAt: 2, history: [], approvals: [], outputs: {
      validation: { passed: true, sourceFingerprint: "validated-content", commands: ["npm test"] },
      commit: { headSha: "local-commit", sourceFingerprint: "validated-content", changedFiles: ["src/file.ts"] },
      agent: { headSha: "untrusted-agent-claim", passed: true },
    } };
    expect(publicationEvidence(run)).toEqual([
      { nodeId: "validation", operation: "workspace.validate", value: run.outputs.validation },
      { nodeId: "commit", operation: "workspace.commit", value: run.outputs.commit },
    ]);
  });
});


describe("built-in workflow template localization", () => {
  it("localizes every catalog name, step label and approval message on a fresh copy", () => {
    use("pt-BR");
    try {
      for (const template of catalog.templates as unknown as Workflow[]) {
        const original = structuredClone(template);
        const localized = localizeBuiltinTemplate(template, key => t(key as Key));
        expect(localized.name).not.toBe(template.name);
        for (const node of localized.nodes) {
          expect(node.label).not.toMatch(/^automations\.templates\./);
          expect(node.label).not.toBe(template.nodes.find(item => item.id === node.id)!.label);
          if (node.config.type === "approval") {
            expect(node.config.message).not.toMatch(/^automations\.templates\./);
            expect(node.config.message).not.toBe((template.nodes.find(item => item.id === node.id)!.config as { message: string }).message);
          }
        }
        expect(template).toEqual(original);
      }
    } finally { use("en"); }
  });

  it("preserves saved workflows and user-authored drafts regardless of current UI language", () => {
    const saved = structuredClone(catalog.templates[0]) as unknown as Workflow;
    saved.revision = 3; saved.name = "My custom English name"; saved.nodes[0].label = "My event";
    const translated = () => "must not be used";
    expect(localizeBuiltinTemplate(saved, translated)).toEqual(saved);
    const draft = { ...saved, id: "user-authored-workflow", revision: 0 };
    expect(localizeBuiltinTemplate(draft, translated)).toEqual(draft);
  });
});
