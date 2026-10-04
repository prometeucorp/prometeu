/** Portable workflow wire types. Rust owns validation, persistence and execution. */
export type Position = { x: number; y: number };
export type NodeConfig =
  | { type: "trigger"; event: string; intervalSeconds?: number; baseline: "ignoreExisting" | "includeExisting" }
  | { type: "query" | "action" | "jev"; operation: string; version: number; inputs: Record<string, unknown> }
  | { type: "condition"; path: string; operator: "equals" | "notEquals" | "exists" | "truthy"; value?: unknown }
  | { type: "agent"; prompt: string; provider?: string; context?: unknown; tools?: string[]; checks?: { executable: string; args: string[] }[]; outputSchema?: unknown }
  | { type: "approval"; message: string }
  | { type: "wait"; seconds: number };
export type NodeKind = NodeConfig["type"];
export type WorkflowNode = { id: string; label: string; position: Position; config: NodeConfig };
export type WorkflowEdge = { from: string; to: string; port: string };
export type WorkflowPolicy = { maxConcurrentRuns: number; requireMergeApproval: boolean; allowWrites: boolean; allowCommit?: boolean; allowPush?: boolean; requirePublishApproval?: boolean; maxRetries: number; maxAgentTurns: number; maxCostUsd?: number };
export type WorkflowScope = { projectId?: string; repository?: string; identity?: string; linearProjectId?: string; targets?: { projectId: string; repository?: string; identity?: string }[] };
export type Workflow = { id: string; name: string; revision: number; enabled: boolean; nodes: WorkflowNode[]; edges: WorkflowEdge[]; policy: WorkflowPolicy; scope: WorkflowScope };
export type RegistryEntry = { id: string; version: number; kind: "query" | "action" | "jev"; title: string; requiredInputs: string[]; inputSchema: Record<string, "string" | "number" | "boolean" | "object" | "array" | "any">; outputPorts: string[]; effect: "read" | "write" | "merge" };
export type ValidationIssue = { code: string; message: string; nodeId?: string };
export type SimulationFixture = { event: unknown; outputs: Record<string, unknown> };
export type SimulationStep = { nodeId: string; status: "simulated" | "skipped" | "error" | "uncertain"; port?: string; input: unknown; output: unknown; message: string };
export type SimulationResult = { steps: SimulationStep[]; outputs: Record<string, unknown>; effectsSuppressed: true };

export function outputPorts(node: WorkflowNode, registry: RegistryEntry[] = []): string[] {
  const c = node.config;
  if (c.type === "condition") return ["true", "false", "error"];
  if (c.type === "query" || c.type === "action" || c.type === "jev") {
    const entry = registry.find(r => r.id === c.operation && r.version === c.version && r.kind === c.type);
    if (entry) return entry.outputPorts;
  }
  return (c.type === "jev" || c.type === "agent") ? ["next", "error", "uncertain"] : ["next", "error"];
}

/** One outgoing connection per port. Replacing a port never silently creates a cycle. */
export function connectNodes(workflow: Workflow, from: string, port: string, to: string, registry: RegistryEntry[] = []): Workflow | null {
  const source = workflow.nodes.find(n => n.id === from);
  const target = workflow.nodes.find(n => n.id === to);
  if (!source || !target || target.config.type === "trigger" || from === to || !outputPorts(source, registry).includes(port)) return null;
  const edges = workflow.edges.filter(e => e.from !== from || e.port !== port);
  const visit = [to]; const visited = new Set<string>();
  while (visit.length) {
    const current = visit.pop()!;
    if (current === from) return null;
    if (visited.has(current)) continue;
    visited.add(current);
    visit.push(...edges.filter(e => e.from === current).map(e => e.to));
  }
  return { ...workflow, edges: [...edges, { from, port, to }] };
}

export function removeNode(workflow: Workflow, id: string): Workflow {
  return { ...workflow, nodes: workflow.nodes.filter(n => n.id !== id), edges: workflow.edges.filter(e => e.from !== id && e.to !== id) };
}

export type WorkflowChange = { path: string; before: unknown; after: unknown };
/** Structural review includes policy and scope, so proposed privileges cannot hide in node copy. */
export function workflowDiff(before: Workflow, after: Workflow): WorkflowChange[] {
  const changes: WorkflowChange[] = [];
  const walk = (a: unknown, b: unknown, path: string) => {
    if (JSON.stringify(a) === JSON.stringify(b)) return;
    if (a && b && typeof a === "object" && typeof b === "object" && !Array.isArray(a) && !Array.isArray(b)) {
      const left = a as Record<string, unknown>, right = b as Record<string, unknown>;
      for (const key of new Set([...Object.keys(left), ...Object.keys(right)])) walk(left[key], right[key], path ? `${path}.${key}` : key);
    } else changes.push({ path, before: a, after: b });
  };
  walk(before, after, "");
  return changes;
}

export function newNode(kind: NodeKind, label: string, index: number, registry: RegistryEntry[]): WorkflowNode {
  let config: NodeConfig;
  switch (kind) {
    case "trigger": config = { type: kind, event: "manual", baseline: "ignoreExisting" }; break;
    case "condition": config = { type: kind, path: "event.ready", operator: "truthy" }; break;
    case "agent": config = { type: kind, prompt: "", provider: "codex" }; break;
    case "approval": config = { type: kind, message: "" }; break;
    case "wait": config = { type: kind, seconds: 60 }; break;
    default: {
      const entry = registry.find(r => r.kind === kind);
      config = { type: kind, operation: entry?.id ?? "", version: entry?.version ?? 1, inputs: Object.fromEntries((entry?.requiredInputs ?? []).map(key => [key, ""])) };
    }
  }
  return { id: crypto.randomUUID(), label, position: { x: 60 + (index % 3) * 260, y: 60 + Math.floor(index / 3) * 190 }, config };
}

export function blankWorkflow(name: string): Workflow {
  return { id: crypto.randomUUID(), name, revision: 0, enabled: false, nodes: [], edges: [], scope: {}, policy: { maxConcurrentRuns: 1, requireMergeApproval: true, allowWrites: false, allowCommit: false, allowPush: false, requirePublishApproval: true, maxRetries: 2, maxAgentTurns: 5 } };
}

export type AutomationRun = { id: string; workflow: Workflow; status: "queued" | "running" | "waiting" | "awaitingApproval" | "paused" | "succeeded" | "failed" | "cancelled"; event: unknown; eventKey: string; resourceKey?: string; createdAt: number; updatedAt: number; history: { sequence: number; at: number; nodeId?: string; kind: string; message: string }[]; approvals: { nodeId: string; headSha?: string; eventKey: string; approvedAt: number; actor: string }[]; outputs: Record<string, unknown>; completedPorts?: Record<string, string>; measuredCostUsd?: number; costUnknown?: boolean; pendingApprovalNodeId?: string };
export type AutomationSnapshot = { workflows: Workflow[]; revisions: Workflow[]; runs: AutomationRun[]; registry: RegistryEntry[]; templates: Workflow[]; diagnostics?: { workflowId: string; error: string | null; lastPolledAt: number }[] };
export type AutomationProposal = { workflow: Workflow; summary: string };
export type WorkflowProposal = AutomationProposal;

/** Unsupported IPC is a capability gap, distinct from a transient connection failure. */
export function automationTransportUnavailable(error: unknown): boolean {
  const text = typeof error === "string" ? error : error instanceof Error ? error.message : JSON.stringify(error);
  return typeof text === "string" && (/application_(?:command|operation)_unsupported/.test(text)
    || /(?:unknown|unsupported|not found|not implemented)[^\n]*automations_/i.test(text)
    || /automations_[^\n]*(?:unknown|unsupported|not found|not implemented)/i.test(text));
}

/** The executor selects the current approval from the frozen graph, not its history. */
export function pendingApprovalNode(run: AutomationRun): WorkflowNode | undefined {
  const id = run.pendingApprovalNodeId;
  if (run.status !== "awaitingApproval" || !id || run.completedPorts?.[id] !== undefined
    || run.approvals.some(approval => approval.nodeId === id)) return undefined;
  return run.workflow.nodes.find(node => node.id === id && node.config.type === "approval");
}

/** Only persisted outputs from the run's frozen definition are approval evidence. */
export function publicationEvidence(run: AutomationRun): { nodeId: string; operation: string; value: unknown }[] {
  return run.workflow.nodes.flatMap(node => {
    const c = node.config;
    return (c.type === "action" || c.type === "query") && ["workspace.validate", "workspace.commit"].includes(c.operation) && run.outputs[node.id] !== undefined
      ? [{ nodeId: node.id, operation: c.operation, value: run.outputs[node.id] }] : [];
  });
}

const builtinTemplateNodes: Record<string, string[]> = {
  "template-pr-followup": ["trigger", "status", "needs_action", "diagnose", "review"],
  "template-pr-repair": ["trigger", "status", "needs_action", "repair", "validate", "commit", "approve", "publish", "manual"],
  "template-pr-risk": ["trigger", "status", "complete", "classify", "low", "approve", "merge", "manual"],
  "template-linear-assignment": ["trigger", "approve", "workspace", "investigate"],
};

/** Localize a fresh catalog copy only. Saved workflows and their user content are never translated. */
export function localizeBuiltinTemplate(template: Workflow, translate: (key: string) => string): Workflow {
  const copy = structuredClone(template);
  const nodes = builtinTemplateNodes[template.id];
  if (!nodes || template.revision !== 0) return copy;
  const prefix = `automations.templates.${template.id}`;
  copy.name = translate(`${prefix}.name`);
  for (const node of copy.nodes) {
    if (!nodes.includes(node.id)) continue;
    node.label = translate(`${prefix}.${node.id}.label`);
    if (node.config.type === "approval") node.config.message = translate(`${prefix}.${node.id}.message`);
  }
  return copy;
}
