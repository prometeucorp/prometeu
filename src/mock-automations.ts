/** Browser-only development backend. Never invokes providers or external services. */
import type { IpcHandlers } from "./ipc";
import catalog from "./automation-catalog.json";
import { outputPorts, type Workflow, type RegistryEntry, type ValidationIssue, type SimulationFixture, type SimulationResult, type SimulationStep, type AutomationSnapshot } from "./automations-model";

const key = "mock:automations";
const registry = catalog.registry as unknown as RegistryEntry[];
function templates(): Workflow[] { return structuredClone(catalog.templates) as Workflow[]; }
function state(): AutomationSnapshot {
  const data = JSON.parse(localStorage.getItem(key) ?? "null");
  return { workflows: data?.workflows ?? [], revisions: data?.revisions ?? [], runs: data?.runs ?? [], registry, templates: templates(), diagnostics: [] };
}
function persist(data: AutomationSnapshot) { localStorage.setItem(key, JSON.stringify(data)); }

export function validate(workflow: Workflow): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  const add = (code: string, message: string, nodeId?: string) => issues.push({ code, message, nodeId });
  if (!workflow.id.trim() || !workflow.name.trim()) add("required", "Workflow id and name are required");
  if (workflow.nodes.filter(n => n.config.type === "trigger").length !== 1) add("trigger", "A workflow requires exactly one trigger");
  const ids = new Set<string>();
  for (const node of workflow.nodes) {
    if (ids.has(node.id) || !node.id) add("node_id", "Node ids must be unique", node.id); ids.add(node.id);
    const c = node.config;
    if (c.type === "agent" && !c.prompt.trim()) add("required", "Agent prompt is required", node.id);
    if (c.type === "approval" && !c.message.trim()) add("required", "Approval message is required", node.id);
    if (c.type === "condition" && !c.path.trim()) add("required", "Condition path is required", node.id);
    if (c.type === "wait" && c.seconds <= 0) add("required", "Wait duration must be positive", node.id);
    if (c.type === "trigger" && c.intervalSeconds != null && c.intervalSeconds < 60) add("interval", "Polling interval must be at least 60 seconds", node.id);
    if (c.type === "query" || c.type === "jev" || c.type === "action") {
      const op = registry.find(r => r.id === c.operation && r.version === c.version && r.kind === c.type);
      if (!op) add("operation", "Unknown operation", node.id);
      else for (const field of op.requiredInputs) if (c.inputs[field] == null || c.inputs[field] === "") add("required_input", `Input ${field} is required`, node.id);
    }
  }
  for (const edge of workflow.edges) {
    const source = workflow.nodes.find(n => n.id === edge.from), target = workflow.nodes.find(n => n.id === edge.to);
    if (!source || !target) add("dangling_edge", "Edge references a missing node");
    else if (!outputPorts(source, registry).includes(edge.port) || target.config.type === "trigger") add("port", "Incompatible connection", source.id);
  }
  const order = topological(workflow);
  if (order.length !== workflow.nodes.length) add("cycle", "Workflow must be a directed acyclic graph");
  const reachable = new Set(workflow.nodes.filter(n => n.config.type === "trigger").map(n => n.id));
  for (const node of order) for (const edge of workflow.edges.filter(e => e.from === node.id)) if (reachable.has(edge.from)) reachable.add(edge.to);
  for (const node of workflow.nodes) if (!reachable.has(node.id)) add("unreachable", "Node is not reachable from the trigger", node.id);
  return issues;
}
function topological(workflow: Workflow) {
  const remaining = new Map(workflow.nodes.map(n => [n.id, workflow.edges.filter(e => e.to === n.id).length]));
  const queue = workflow.nodes.filter(n => remaining.get(n.id) === 0).sort((a,b) => a.id.localeCompare(b.id));
  const ordered: Workflow["nodes"] = [];
  while (queue.length) {
    const node = queue.shift()!; ordered.push(node);
    for (const edge of workflow.edges.filter(e => e.from === node.id)) {
      remaining.set(edge.to, (remaining.get(edge.to) ?? 0) - 1);
      const next = workflow.nodes.find(n => n.id === edge.to);
      if (next && remaining.get(edge.to) === 0) queue.push(next);
    }
  }
  return ordered;
}
function lookup(root: unknown, path: string): unknown {
  const parts = path.startsWith("/") ? path.slice(1).split("/").map(p => p.replace(/~1/g, "/").replace(/~0/g, "~")) : path.split(".");
  return parts.reduce<unknown>((value, part) => value && typeof value === "object" ? (value as Record<string, unknown>)[part] : undefined, root);
}
function resolve(value: unknown, context: unknown): unknown {
  if (!value || typeof value !== "object") return value;
  if ("$ref" in value && typeof value.$ref === "string") {
    const result = lookup(context, value.$ref); if (result === undefined) throw new Error(`Missing reference ${value.$ref}`); return result;
  }
  return Array.isArray(value) ? value.map(v => resolve(v, context)) : Object.fromEntries(Object.entries(value).map(([k,v]) => [k,resolve(v,context)]));
}
export function simulate(workflow: Workflow, fixture: SimulationFixture): SimulationResult {
  const errors = validate(workflow); if (errors.length) throw new Error(errors.map(e => e.message).join("; "));
  const result: SimulationResult = { steps: [], outputs: {}, effectsSuppressed: true }, selected = new Map<string,string>();
  for (const node of topological(workflow)) {
    const c = node.config, active = c.type === "trigger" || workflow.edges.some(e => e.to === node.id && selected.get(e.from) === e.port);
    const step: SimulationStep = { nodeId: node.id, status: active ? "simulated" : "skipped", input: null, output: null, message: active ? "" : "Branch not selected" };
    if (active) {
      step.port = "next";
      const context = { event: fixture.event, nodes: result.outputs };
      try {
        if (c.type === "trigger") { step.output = fixture.event; step.message = "Provided event"; }
        else if (c.type === "condition") {
          const actual = lookup(context, c.path); step.input = actual ?? null;
          if (actual === undefined && c.operator !== "exists") throw new Error(`Missing condition path ${c.path}`);
          const pass = c.operator === "exists" ? actual != null : c.operator === "truthy" ? actual === true : c.operator === "equals" ? JSON.stringify(actual) === JSON.stringify(c.value) : JSON.stringify(actual) !== JSON.stringify(c.value);
          step.port = String(pass); step.output = pass; step.message = `Selected ${pass} branch`;
        } else if (c.type === "approval") { step.output = { approvalRequired: true }; step.message = `Would pause for approval: ${c.message}`; }
        else if (c.type === "wait") { step.output = { seconds: c.seconds }; step.message = "Wait planned; time not advanced"; }
        else {
          step.input = c.type === "agent" ? c.prompt : resolve(c.inputs, context);
          const output = fixture.outputs[node.id];
          if (output !== undefined) { step.output = output; step.message = "Provided fixture; effects suppressed"; if (output && typeof output === "object" && "$port" in output) step.port = String(output.$port); }
          else if (c.type === "action") { step.output = { suppressed: true }; step.message = "Action planned; effect suppressed"; }
          else { step.status = c.type === "agent" || c.type === "jev" ? "uncertain" : "error"; step.port = step.status; step.message = "No fixture supplied; no provider invoked"; }
        }
      } catch (error) { step.status = "error"; step.port = "error"; step.message = String(error); }
      if (!outputPorts(node, registry).includes(step.port!)) { step.status = "error"; step.message = "Fixture selected unsupported port"; step.port = undefined; }
      if (step.port) selected.set(node.id, step.port);
      result.outputs[node.id] = step.output;
    }
    result.steps.push(step);
  }
  return result;
}

export const commands: Pick<IpcHandlers, "automations_snapshot" | "automations_save" | "automations_validate" | "automations_simulate" | "automations_pause" | "automations_delete" | "automations_propose" | "automations_run" | "automations_approve" | "automations_resume" | "automations_cancel"> = {
  automations_snapshot: () => structuredClone(state()),
  automations_save: ({ workflow, expectedRevision }) => {
    const data = state(), previous = data.workflows.find(w => w.id === workflow.id);
    if ((previous?.revision ?? 0) !== (expectedRevision ?? workflow.revision)) throw new Error("automation_revision_conflict");
    const errors = validate(workflow); if (errors.length) throw new Error(errors.map(e => e.message).join("; "));
    const saved = structuredClone({ ...workflow, revision: (previous?.revision ?? 0) + 1 });
    data.workflows = [...data.workflows.filter(w => w.id !== workflow.id), saved]; data.revisions.push(saved); persist(data); return structuredClone(saved);
  },
  automations_validate: ({ workflow }) => validate(workflow),
  automations_simulate: ({ workflow, fixture }) => simulate(workflow, fixture),
  automations_pause: ({ id, paused }) => {
    const data = state(), workflow = data.workflows.find(w => w.id === id); if (!workflow) throw new Error("automation_not_found");
    workflow.enabled = !paused; workflow.revision++; data.revisions.push(structuredClone(workflow)); persist(data); return structuredClone(workflow);
  },
  automations_delete: ({ id }) => { const data = state(); data.workflows = data.workflows.filter(w => w.id !== id); persist(data); },
  automations_propose: () => { throw new Error("automation_browser_only: Open the desktop app with a connected Claude account to generate a real proposal."); },
  automations_run: () => { throw new Error("automation_browser_only: Runtime execution requires the desktop app. Use simulation here."); },
  automations_resume: () => { throw new Error("automation_browser_only: Resume requires the desktop app."); },
  automations_cancel: () => { throw new Error("automation_browser_only: Cancellation requires the desktop app."); },
  automations_approve: () => { throw new Error("automation_browser_only: Runtime approvals require the desktop app."); },
};
