import { invoke } from "./ipc";
import { fromBack, t, type Key } from "./i18n";
import { h } from "./util";
import type { Project } from "./types";
import { icon, type IconName } from "./icons";
import * as ui from "./ui";
import { sectionHeader, toolbar, listState } from "./components/compositions";
import { localizeBuiltinTemplate, automationTransportUnavailable, publicationEvidence, blankWorkflow, connectNodes, newNode, outputPorts, removeNode, workflowDiff } from "./automations-model";
import type { AutomationProposal, AutomationSnapshot, NodeKind, SimulationResult, ValidationIssue, Workflow, WorkflowNode } from "./automations-model";
import "./automations.css";

const kinds: NodeKind[] = ["trigger", "query", "condition", "jev", "agent", "action", "approval", "wait"];
const glyphs: Record<NodeKind, IconName> = { trigger: "play", query: "file", condition: "git-branch", jev: "sparkles", agent: "terminal", action: "check", approval: "user", wait: "rotate" };
const key = (value: string) => `automations.${value}` as Key;
const label = (value: string) => t(key(value));
const json = (value: unknown) => JSON.stringify(value, null, 2) ?? "—";
const portLabel = (port: string) => ["next", "true", "false", "error", "uncertain"].includes(port) ? label(port) : port;

/** This mounted controller owns only drafts. Native IPC owns saved definitions and runs. */
export function mountAutomations(host: HTMLElement): () => void {
  let alive = true;
  let snapshot: AutomationSnapshot = { workflows: [], revisions: [], runs: [], registry: [], templates: [] };
  let draft: Workflow | null = null;
  let saved = "";
  let projects: Project[] = [];
  let selected: string | null = null;
  let view: "graph" | "runs" | "history" = "graph";
  let issues: ValidationIssue[] | null = null;
  let simulation: SimulationResult | null = null;
  let proposal: { result: AutomationProposal; base: string } | null = null;
  let connecting: { from: string; port: string } | null = null;
  let zoom = 1;
  let busy = false;
  let unavailable = false;
  let requestText = "";
  let assistantVisible = false;
  let fixtureText = '{\n  "event": { "ready": true },\n  "outputs": {}\n}';
  const undo: Workflow[] = [];
  const root = h("section", "automations"); root.setAttribute("aria-label", label("title"));
  const header = h("div", "automations-header");
  const bar = h("div", "automations-bar");
  const body = h("div", "automations-workbench");
  const assistant = h("aside", "automations-assistant");
  const main = h("div", "automations-main");
  const inspector = h("aside", "automations-inspector"); inspector.setAttribute("aria-label", label("inspector"));
  const tabs = h("div", "automations-tabs");
  const validation = h("div", "automations-validation");
  const content = h("div", "automations-content");
  const footer = h("div", "automations-simulation");
  const notice = h("div", "automations-notice"); notice.setAttribute("role", "status"); notice.setAttribute("aria-live", "polite");
  main.append(tabs, validation, content, footer); body.append(assistant, main, inspector);
  root.append(header, bar, body, notice); host.replaceChildren(root);
  const dirty = () => !!draft && JSON.stringify(draft) !== saved;
  const say = (message: string, error = false) => { notice.textContent = message; notice.dataset.error = String(error); notice.setAttribute("role", error ? "alert" : "status"); };
  const act = (name: string, run: () => void, variant: ui.ButtonVariant = "outline") => { const control = ui.button(label(name), run, variant); control.dataset.focus = `automations-${name}`; return control; };
  const task = async (run: () => Promise<void>) => {
    if (busy) return;
    busy = true; root.setAttribute("aria-busy", "true"); renderHeader();
    const disabled = [...root.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLTextAreaElement>("button, input, textarea")].map(control => ({ control, disabled: control.disabled }));
    disabled.forEach(({ control }) => { control.disabled = true; });
    try { await run(); } catch (error) { if (alive) say(fromBack(error), true); }
    finally { disabled.forEach(item => { if (item.control.isConnected) item.control.disabled = item.disabled; }); busy = false; if (alive) { root.removeAttribute("aria-busy"); renderHeader(); } }
  };
  const mutate = (next: Workflow, redraw = true) => {
    if (draft) undo.push(structuredClone(draft));
    if (undo.length > 40) undo.shift();
    draft = next; issues = null; simulation = null;
    if (redraw) render(); else { renderHeader(); renderValidation(); }
  };
  const load = async () => { const [next, board] = await Promise.all([invoke("automations_snapshot"), invoke("load_board")]); snapshot = next; projects = board.projects; };
  const choose = (workflow: Workflow, isNew = false) => {
    draft = structuredClone(workflow); saved = isNew ? "" : JSON.stringify(draft); selected = null; issues = null; simulation = null; proposal = null; connecting = null; undo.length = 0; view = "graph"; render();
    if (draft.nodes.length && content.clientWidth) { zoom = Math.max(.4, Math.min(1, content.clientWidth / Math.max(900, ...draft.nodes.map(n => n.position.x + 300)))); renderContent(); }
  };
  const switchTo = (workflow: Workflow, isNew = false) => {
    if (!dirty()) { choose(workflow, isNew); return; }
    const dialog = ui.formDialog({ error: fromBack, title: label("leaveDraft"), save: label("discard"), cancel: label("cancel"), submit: async () => choose(workflow, isNew) }); dialog.open();
  };

  function renderHeader() {
    header.replaceChildren(sectionHeader({ title: draft?.name || label("title"), description: label(unavailable ? "unsupportedHelp" : "intro"), actions: unavailable ? [] : draft ? [
      act("save", () => void task(async () => {
        if (!draft) return;
        const candidate = structuredClone(draft);
        issues = await invoke("automations_validate", { workflow: candidate }); renderValidation();
        if (issues.length) return;
        const result = await invoke("automations_save", { workflow: candidate, expectedRevision: candidate.revision || null });
        await load(); if (alive) { choose(result); say(label("saved")); }
      }), "pri"),
      act(draft.enabled ? "pause" : "activate", () => void task(async () => {
        if (!draft) return;
        const result = await invoke("automations_pause", { id: draft.id, paused: draft.enabled }); await load(); if (alive) choose(result);
      })),
    ] : [act("new", createMenu, "pri")] }));
    const controls = header.querySelectorAll("button");
    controls.forEach(control => { control.disabled = busy; });
    if (draft && controls[1]) controls[1].disabled = busy || dirty() || draft.revision === 0;
    bar.replaceChildren();
    if (!draft) return;
    const picker = ui.select(draft.id, [...snapshot.workflows.map(w => [w.id, w.name] as [string, string]), ...(!snapshot.workflows.some(w => w.id === draft!.id) ? [[draft.id, draft.name] as [string, string]] : [])]);
    picker.control.setAttribute("aria-label", label("select")); picker.onchange = () => { const w = snapshot.workflows.find(w => w.id === picker.value); if (w) switchTo(w); };
    bar.append(picker.control, act("new", createMenu), h("span", "automations-state", label(draft.enabled ? "active" : "paused")), h("span", "automations-version", `v${draft.revision}`), h("span", "ui-hint", dirty() ? label("draft") : label("saved")));
    const tools = h("div", "automations-bar-tools");
    const assistantToggle = act("assistantTab", () => { assistantVisible = !assistantVisible; body.classList.toggle("show-assistant", assistantVisible); renderHeader(); }, "ghost"); assistantToggle.classList.add("automations-assistant-toggle"); assistantToggle.setAttribute("aria-expanded", String(assistantVisible));
    tools.append(assistantToggle, act("policy", policyDialog, "ghost"), act("definition", () => jsonDialog(label("definition"), draft), "ghost")); bar.append(tools);
  }

  function createMenu() {
    const templates = snapshot.templates.map(template => localizeBuiltinTemplate(template, key => t(key as Key)));
    const dialog = ui.formDialog({ error: fromBack, title: label("new"), save: label("new"), cancel: label("cancel"), submit: async () => {
      const template = templates.find(w => w.id === pick.value);
      const workflow = template ? structuredClone(template) : blankWorkflow(name.value.trim() || label("untitled"));
      workflow.id = crypto.randomUUID(); workflow.revision = 0; workflow.enabled = false; workflow.name = name.value.trim() || template?.name || label("untitled"); switchTo(workflow, true);
    } });
    const name = ui.input(""); name.placeholder = label("untitled");
    const pick = ui.select("", [["", label("blank")], ...templates.map(w => [w.id, w.name] as [string, string])]);
    let suggestedName = "";
    pick.onchange = () => { const nextName = templates.find(w => w.id === pick.value)?.name ?? ""; if (!name.value || name.value === suggestedName) name.value = nextName; suggestedName = nextName; };
    dialog.body.append(ui.field(label("name"), name), ui.field(label("templates"), pick.control)); dialog.open();
  }

  function renderAssistant() {
    assistant.replaceChildren();
    assistant.append(sectionHeader({ title: label("assistant"), description: label("assistantHelp") }));
    const messages = h("div", "automations-proposals");
    if (proposal && draft) {
      messages.append(h("h3", "", label("proposal")), h("p", "ui-hint", proposal.result.summary));
      const changes = workflowDiff(draft, proposal.result.workflow);
      if (!changes.length) messages.append(h("p", "ui-hint", label("noChanges")));
      for (const change of changes) {
        const item = h("details", "automations-change"); item.append(h("summary", "", change.path));
        item.append(h("small", "", label("before")), h("pre", "automations-before", json(change.before)), h("small", "", label("after")), h("pre", "automations-after", json(change.after))); messages.append(item);
      }
      const apply = act("applyProposal", () => {
        if (!proposal || !draft) return;
        if (JSON.stringify(draft) !== proposal.base) { say(label("staleProposal"), true); return; }
        const next = { ...proposal.result.workflow, id: draft.id, revision: draft.revision, enabled: draft.enabled }; proposal = null; mutate(next);
      }, "pri"); apply.disabled = JSON.stringify(draft) !== proposal.base;
      messages.append(toolbar([act("discard", () => { proposal = null; renderAssistant(); }), apply]));
      if (apply.disabled) messages.append(h("p", "ui-hint", label("staleProposal")));
    } else messages.append(h("p", "ui-hint", label("inspectHelp")), h("p", "ui-hint", label("connectHelp")));
    const prompt = ui.input(requestText, true); prompt.placeholder = label("request"); prompt.setAttribute("aria-label", label("request")); prompt.oninput = () => { requestText = prompt.value; };
    const send = act("propose", () => void task(async () => {
      if (!draft || !requestText.trim()) return;
      const base = JSON.stringify(draft); send.disabled = true; send.textContent = label("proposing");
      try {
        const result = await invoke("automations_propose", { prompt: `${requestText.trim()}\n\nCurrent workflow definition:\n${base}`, projectId: draft.scope.projectId, workflow: structuredClone(draft) });
        if (alive) { proposal = { result, base }; renderAssistant(); }
      } catch (error) { say(`${label("proposalUnavailable")} ${fromBack(error)}`, true); }
      finally { send.disabled = false; send.textContent = label("propose"); }
    }), "pri"); send.disabled = !draft;
    const composer = h("div", "automations-composer"); composer.append(prompt, send); assistant.append(messages, composer);
  }

  function renderValidation() {
    validation.replaceChildren(); if (!draft) return;
    if (issues === null) validation.append(h("span", "ui-hint", label("unvalidated")));
    else if (!issues.length) validation.append(h("span", "automations-valid", t("automations.valid", { nodes: draft.nodes.length, edges: draft.edges.length })));
    else {
      const details = h("details", "automations-errors") as HTMLDetailsElement; details.open = true; details.append(h("summary", "", t("automations.validation", { n: issues.length })));
      for (const issue of issues) details.append(ui.button(`${issue.nodeId ? `${draft.nodes.find(n => n.id === issue.nodeId)?.label ?? issue.nodeId}: ` : ""}${issue.message}`, () => { selected = issue.nodeId ?? null; view = "graph"; renderContent(); renderInspector(); }, "ghost"));
      validation.append(details);
    }
    validation.append(act("validate", () => void task(async () => { if (draft) { const base = JSON.stringify(draft); const result = await invoke("automations_validate", { workflow: structuredClone(draft) }); if (alive && JSON.stringify(draft) === base) { issues = result; renderValidation(); } } }), "ghost"));
  }

  function render() {
    if (!alive) return;
    renderHeader(); renderAssistant(); renderValidation(); renderTabs(); renderContent(); renderInspector(); renderSimulation();
  }
  function renderTabs() {
    tabs.replaceChildren();
    const buttons = (["graph", "runs", "history"] as const).map(name => {
      const b = act(name, () => { view = name; renderTabs(); renderContent(); renderInspector(); }); b.classList.toggle("selected", name === view); b.setAttribute("aria-pressed", String(name === view)); return b;
    });
    const revert = act("undo", () => { const previous = undo.pop(); if (previous) { draft = previous; issues = null; simulation = null; render(); } }, "ghost"); revert.disabled = !undo.length;
    const add = act("add", addDialog); add.disabled = !draft;
    tabs.append(toolbar(buttons, [revert, add]));
  }
  function addDialog() {
    if (!draft) return;
    const dialog = ui.formDialog({ error: fromBack, title: label("add"), save: label("add"), cancel: label("cancel"), submit: async () => {} });
    dialog.body.classList.add("automations-palette");
    for (const kind of kinds) {
      const b = ui.button("", () => { if (!draft) return; const n = newNode(kind, label(kind), draft.nodes.length, snapshot.registry); selected = n.id; mutate({ ...draft, nodes: [...draft.nodes, n] }); dialog.close(); });
      b.classList.add("automations-palette-item"); const mark = h("span", `automations-kind ${kind}`); mark.innerHTML = icon(glyphs[kind]);
      const text = h("span", ""); text.append(h("b", "", label(kind)), h("small", "", label(`${kind}Help`))); b.append(mark, text); dialog.body.append(b);
    }
    dialog.open(); dialog.body.closest("dialog")?.querySelector<HTMLButtonElement>('button[type="submit"]')?.remove();
  }

  function renderContent() {
    const oldViewport = content.querySelector(".automations-viewport"); const scroll = { x: oldViewport?.scrollLeft ?? 0, y: oldViewport?.scrollTop ?? 0 };
    content.replaceChildren();
    if (!draft) { content.append(listState({ kind: "empty", text: label("emptyHelp") }), act("new", createMenu, "pri")); return; }
    if (view === "runs") { renderRuns(); return; }
    if (view === "history") { renderHistory(); return; }
    const viewport = h("div", "automations-viewport");
    const canvas = h("div", "automations-canvas"); canvas.setAttribute("aria-label", label("graph"));
    const width = Math.max(900, ...draft.nodes.map(n => n.position.x + 300)); const height = Math.max(620, ...draft.nodes.map(n => n.position.y + 220));
    const plane = h("div", "automations-plane"); plane.style.width = `${width * zoom}px`; plane.style.height = `${height * zoom}px`;
    canvas.style.width = `${width}px`; canvas.style.height = `${height}px`; canvas.style.transform = `scale(${zoom})`;
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg"); svg.classList.add("automations-edges"); svg.setAttribute("width", String(width)); svg.setAttribute("height", String(height)); canvas.append(svg);
    const drawEdges = () => {
      svg.replaceChildren();
      for (const edge of draft!.edges) {
        const from = draft!.nodes.find(n => n.id === edge.from), to = draft!.nodes.find(n => n.id === edge.to); if (!from || !to) continue;
        const x = from.position.x + 110, y = from.position.y + 110, tx = to.position.x + 110, ty = to.position.y;
        const p = document.createElementNS(svg.namespaceURI, "path"); p.setAttribute("d", `M ${x} ${y} C ${x} ${y + 55}, ${tx} ${ty - 55}, ${tx} ${ty}`);
        if (simulation?.steps.some(s => s.nodeId === edge.from && s.port === edge.port)) p.classList.add("traversed"); svg.append(p);
        const text = document.createElementNS(svg.namespaceURI, "text"); text.setAttribute("x", String((x + tx) / 2 + 7)); text.setAttribute("y", String((y + ty) / 2)); text.textContent = portLabel(edge.port); svg.append(text);
      }
    }; drawEdges();
    for (const node of draft.nodes) {
      const card = h("article", `automations-node ${node.config.type}`); card.dataset.nodeId = node.id; card.classList.toggle("selected", selected === node.id); card.classList.toggle("traversed", !!simulation?.steps.some(s => s.nodeId === node.id && s.status === "simulated"));
      card.style.left = `${node.position.x}px`; card.style.top = `${node.position.y}px`;
      const handle = ui.button("", () => {
        if (connecting && draft) {
          const next = connectNodes(draft, connecting.from, connecting.port, node.id, snapshot.registry);
          if (next) { connecting = null; mutate(next); } else say(label("invalidConnection"), true);
        } else { selected = node.id; canvas.querySelectorAll(".automations-node").forEach(el => el.classList.toggle("selected", (el as HTMLElement).dataset.nodeId === node.id)); renderInspector(); }
      }, "ghost");
      handle.classList.add("automations-node-handle"); handle.setAttribute("aria-label", node.label); handle.title = t("automations.nodeMove", { name: node.label });
      const top = h("span", "automations-node-top"); const mark = h("span", "automations-kind"); mark.innerHTML = icon(glyphs[node.config.type], 14); top.append(mark, h("span", "", label(node.config.type)), h("span", "automations-grip", "⠿"));
      handle.append(top, h("strong", "", node.label), h("small", "", nodeDetail(node)));
      let drag: { x: number; y: number; start: { x: number; y: number }; before: Workflow; moved: boolean } | null = null;
      handle.onpointerdown = event => { if (event.button !== 0 || !draft || connecting) return; drag = { x: event.clientX, y: event.clientY, start: { ...node.position }, before: structuredClone(draft), moved: false }; handle.setPointerCapture(event.pointerId); };
      handle.onpointermove = event => {
        if (!drag) return;
        const dx = (event.clientX - drag.x) / zoom, dy = (event.clientY - drag.y) / zoom;
        if (Math.abs(dx) + Math.abs(dy) < 4 && !drag.moved) return;
        drag.moved = true; node.position = { x: Math.max(0, Math.round(drag.start.x + dx)), y: Math.max(0, Math.round(drag.start.y + dy)) };
        card.style.left = `${node.position.x}px`; card.style.top = `${node.position.y}px`; drawEdges();
      };
      const endDrag = () => { if (drag?.moved) { undo.push(drag.before); issues = null; simulation = null; renderHeader(); renderValidation(); renderTabs(); } drag = null; };
      handle.onpointerup = endDrag; handle.onpointercancel = endDrag;
      handle.onkeydown = event => {
        const delta: Record<string, [number, number]> = { ArrowLeft: [-20, 0], ArrowRight: [20, 0], ArrowUp: [0, -20], ArrowDown: [0, 20] };
        if (!delta[event.key] || !draft) return; event.preventDefault();
        const [x, y] = delta[event.key]; const next = structuredClone(draft); const n = next.nodes.find(n => n.id === node.id)!; n.position = { x: Math.max(0, n.position.x + x), y: Math.max(0, n.position.y + y) }; mutate(next); content.querySelector<HTMLButtonElement>(`[data-node-id="${CSS.escape(node.id)}"] .automations-node-handle`)?.focus();
      };
      const ports = h("div", "automations-ports");
      for (const port of outputPorts(node, snapshot.registry)) {
        const b = ui.button(portLabel(port), () => { connecting = { from: node.id, port }; say(t("automations.connecting", { port: portLabel(port) })); renderContent(); }, "ghost");
        b.setAttribute("aria-label", t("automations.connect", { port: portLabel(port) })); b.classList.toggle("connecting", connecting?.from === node.id && connecting.port === port); ports.append(b);
      }
      card.append(handle, ports); canvas.append(card);
    }
    plane.append(canvas); viewport.append(plane); content.append(viewport); viewport.scrollLeft = scroll.x; viewport.scrollTop = scroll.y;
    const tools = toolbar([act("zoomOut", () => { zoom = Math.max(.4, zoom - .1); renderContent(); }, "ghost"), h("span", "", `${Math.round(zoom * 100)}%`), act("zoomIn", () => { zoom = Math.min(1.6, zoom + .1); renderContent(); }, "ghost"), act("fit", () => { zoom = Math.max(.4, Math.min(1, viewport.clientWidth / width, viewport.clientHeight / height)); renderContent(); }, "ghost")]); tools.classList.add("automations-zoom"); content.append(tools);
  }
  function nodeDetail(node: WorkflowNode) {
    const c = node.config;
    if (c.type === "trigger") return c.event;
    if (c.type === "condition") return `${c.path} · ${label(c.operator)}`;
    if (c.type === "query" || c.type === "action" || c.type === "jev") return `${c.operation} · v${c.version}`;
    if (c.type === "agent") return c.provider || label("agent");
    if (c.type === "wait") return `${c.seconds}s`;
    return c.type === "approval" ? c.message : "";
  }

  function renderInspector() {
    inspector.replaceChildren();
    const node = draft?.nodes.find(n => n.id === selected); inspector.hidden = !node || view !== "graph"; body.classList.toggle("has-inspector", !inspector.hidden);
    if (!node || !draft) return;
    inspector.append(sectionHeader({ title: label(node.config.type), description: label(`${node.config.type}Help`), actions: [act("close", () => { selected = null; renderInspector(); renderContent(); }, "ghost")] }));
    const form = h("form", "automations-node-form") as HTMLFormElement; const changed = structuredClone(node); const c = changed.config;
    const getters: (() => void)[] = [];
    const textField = (name: string, value: string, save: (value: string) => void, multiline = false) => {
      const input = ui.input(value, multiline); form.append(ui.field(label(name), input)); getters.push(() => save(input.value)); return input;
    };
    const numberField = (name: string, value: number, min: number, save: (value: number) => void) => {
      const input = textField(name, String(value), value => save(Number(value))); input.setAttribute("type", "number"); input.setAttribute("min", String(min)); input.setAttribute("step", "1"); input.required = true;
    };
    const choiceField = (name: string, value: string, options: [string, string][], save: (value: string) => void) => {
      const select = ui.select(value, options); form.append(ui.field(label(name), select.control)); getters.push(() => save(select.value)); return select;
    };
    const jsonField = (name: string, value: unknown, save: (value: unknown) => void) => {
      const input = textField(name, json(value), value => { try { save(JSON.parse(value)); } catch { throw new Error(label("invalidJson")); } }, true); input.classList.add("automations-json"); return input;
    };
    textField("nodeLabel", node.label, value => { changed.label = value; }).required = true;
    switch (c.type) {
      case "trigger":
        choiceField("event", c.event, [...new Map([["manual", label("manual")], ["github.authored_pr", label("githubPr")], ["linear.assigned_issue", label("linearIssue")], [c.event, c.event]] as [string, string][]).entries()], value => { c.event = value; });
        numberField("interval", c.intervalSeconds ?? 60, 60, value => { c.intervalSeconds = value; });
        choiceField("baseline", c.baseline, [["ignoreExisting", label("ignoreExisting")], ["includeExisting", label("includeExisting")]], value => { c.baseline = value as typeof c.baseline; }); break;
      case "condition":
        textField("path", c.path, value => { c.path = value; }).required = true;
        choiceField("operator", c.operator, ["equals", "notEquals", "exists", "truthy"].map(value => [value, label(value)]), value => { c.operator = value as typeof c.operator; });
        jsonField("value", c.value ?? null, value => { c.value = value; }); break;
      case "agent":
        choiceField("provider", c.provider ?? "", [["", label("defaultProvider")], ...[...new Set(["codex", "claude", ...(c.provider ? [c.provider] : [])])].map(value => [value, value] as [string, string])], value => { if (value) c.provider = value; else delete c.provider; });
        textField("prompt", c.prompt, value => { c.prompt = value; }, true).required = true;
        jsonField("context", c.context ?? {}, value => { c.context = value; });
        jsonField("outputSchema", c.outputSchema ?? {}, value => { c.outputSchema = value; });
        jsonField("checks", c.checks ?? [], value => {
          if (!Array.isArray(value) || value.some(check => !check || typeof check !== "object" || typeof check.executable !== "string" || !Array.isArray(check.args) || check.args.some((arg: unknown) => typeof arg !== "string"))) throw new Error(label("invalidChecks"));
          c.checks = value;
        });
        form.append(h("p", "ui-hint", label("checksHelp")));
        for (const tool of ["read_file", "list_files", "write_file", "run_checks"]) { const toggle = ui.checkbox(tool, c.tools?.includes(tool) ?? false); form.append(toggle.label); getters.push(() => { c.tools = (c.tools ?? []).filter(t => t !== tool); if (toggle.control.checked) c.tools.push(tool); }); }
        break;
      case "approval": textField("message", c.message, value => { c.message = value; }, true).required = true; break;
      case "wait": numberField("seconds", c.seconds, 1, value => { c.seconds = value; }); break;
      default: {
        const entries = snapshot.registry.filter(r => r.kind === c.type);
        const operation = choiceField("operation", c.operation, [...new Map([...entries.map(r => [r.id, r.title] as [string, string]), ...(!entries.some(r => r.id === c.operation) ? [[c.operation, c.operation] as [string, string]] : [])]).entries()], value => { if (c.operation !== value) { c.operation = value; c.version = snapshot.registry.find(r => r.id === value)?.version ?? c.version; } });
        const inputs = h("div", "automations-operation-inputs"); form.append(inputs);
        let readInputs: () => Record<string, unknown> = () => c.inputs;
        const renderInputs = () => {
          inputs.replaceChildren(); const entry = entries.find(r => r.id === operation.value); const reads: (() => [string, unknown])[] = [];
          for (const name of Object.keys(entry?.inputSchema ?? Object.fromEntries((entry?.requiredInputs ?? []).map(key => [key, "any"])))) {
            const type = entry?.inputSchema[name] ?? "any"; const current = c.inputs[name]; const control = ui.input(current === undefined ? "" : typeof current === "string" ? current : JSON.stringify(current), type === "object" || type === "array");
            control.required = entry?.requiredInputs.includes(name) ?? false;
            inputs.append(ui.field(name, control, type)); reads.push(() => {
              if (!control.value) return [name, undefined];
              if (type === "string" || control.value.startsWith("$")) return [name, control.value];
              try { return [name, JSON.parse(control.value)]; } catch { if (type === "any") return [name, control.value]; throw new Error(`${name}: ${label("invalidJson")}`); }
            });
          }
          const extra = ui.input(json(Object.fromEntries(Object.entries(c.inputs).filter(([name]) => !(name in (entry?.inputSchema ?? {}))))), true); inputs.append(ui.field(label("inputs"), extra, label("inputsHelp")));
          readInputs = () => { const result = JSON.parse(extra.value); if (!result || typeof result !== "object" || Array.isArray(result)) throw new Error(label("invalidJson")); for (const read of reads) { const [name, value] = read(); if (value !== undefined) result[name] = value; } return result; };
        }; operation.onchange = renderInputs; renderInputs(); getters.push(() => { c.inputs = readInputs(); });
      }
    }
    const error = h("p", "automations-form-error"); error.setAttribute("role", "alert");
    const apply = act("applyStep", () => { form.requestSubmit(); }, "pri");
    form.onsubmit = event => {
      event.preventDefault(); if (!draft) return;
      try {
        getters.forEach(get => get());
        const next = { ...draft, nodes: draft.nodes.map(n => n.id === changed.id ? changed : n), edges: draft.edges.filter(e => e.from !== changed.id || outputPorts(changed, snapshot.registry).includes(e.port)) };
        mutate(next);
      } catch (e) { error.textContent = fromBack(e); }
    };
    form.append(error, apply); inspector.append(form);
    const connections = h("div", "automations-connections"); connections.append(h("h3", "", label("connections")));
    for (const port of outputPorts(node, snapshot.registry)) {
      const current = draft.edges.find(e => e.from === node.id && e.port === port);
      const picker = ui.select(current?.to ?? "", [["", label("noConnection")], ...draft.nodes.filter(n => n.id !== node.id && n.config.type !== "trigger").map(n => [n.id, n.label] as [string, string])]);
      picker.onchange = () => {
        if (!draft) return;
        if (!picker.value) mutate({ ...draft, edges: draft.edges.filter(e => e.from !== node.id || e.port !== port) });
        else { const next = connectNodes(draft, node.id, port, picker.value, snapshot.registry); if (next) mutate(next); else { say(label("invalidConnection"), true); renderInspector(); } }
      }; connections.append(ui.field(`${portLabel(port)} · ${label("destination")}`, picker.control));
    }
    inspector.append(connections, act("remove", () => { if (draft) { selected = null; mutate(removeNode(draft, node.id)); } }, "danger"));
  }

  function policyDialog() {
    if (!draft) return;
    const next = structuredClone(draft); const get: (() => void)[] = [];
    const dialog = ui.formDialog({ error: fromBack, title: label("policy"), save: label("applyStep"), cancel: label("cancel"), submit: async () => { get.forEach(f => f()); mutate(next); } });
    const name = ui.input(next.name); name.required = true;
    const repository = ui.input(next.scope.repository ?? "");
    const identity = ui.input(next.scope.identity ?? "");
    const project = ui.select(next.scope.projectId ?? "", [["", label("selectProject")], ...projects.map(p => [p.id, p.name] as [string, string]), ...(!projects.some(p => p.id === next.scope.projectId) && next.scope.projectId ? [[next.scope.projectId, next.scope.projectId] as [string, string]] : [])]);
    dialog.body.append(ui.field(label("name"), name), ui.field(label("localProject"), project.control), ui.field(label("repository"), repository), ui.field(label("identity"), identity));
    get.push(() => { next.name = name.value; next.scope.projectId = project.value || undefined; next.scope.repository = repository.value.trim() || undefined; next.scope.identity = identity.value.trim() || undefined; });
    const targetFields: { projectId: string; repository: HTMLInputElement; identity: HTMLInputElement }[] = [];
    const connectionError = h("p", "automations-form-error"); connectionError.setAttribute("role", "alert");
    const connected = act("useAccount", () => {
      connected.disabled = true; dialog.save.disabled = true; connectionError.textContent = "";
      void (async () => {
        try {
          const linear = next.nodes.some(n => n.config.type === "trigger" && n.config.event.startsWith("linear."));
          if (linear) {
            const status = await invoke("linear_status");
            if (!status.connected || !status.who) throw new Error(label("connectLinear"));
            if (dialog.root.isConnected) identity.value = status.who.id;
          } else {
            const [login, mappings] = await Promise.all([invoke("github_identity"), invoke("github_projects")]);
            if (dialog.root.isConnected) {
              identity.value = login; const match = mappings.find(p => p.project === project.value); if (match) repository.value = match.repository;
              for (const target of targetFields) { const mapping = mappings.find(p => p.project === target.projectId); if (mapping && !target.repository.value) target.repository.value = mapping.repository; if (!target.identity.value) target.identity.value = login; }
            }
          }
        } catch (error) { if (dialog.root.isConnected) connectionError.textContent = fromBack(error); }
        finally { connected.disabled = false; dialog.save.disabled = false; }
      })();
    });
    dialog.body.append(connected, connectionError);
    const linearProject = ui.input(next.scope.linearProjectId ?? ""); dialog.body.append(ui.field(label("linearProject"), linearProject)); get.push(() => { next.scope.linearProjectId = linearProject.value.trim() || undefined; });
    const targetSection = ui.disclosure(label("additionalProjects"));
    const targetReads: (() => { projectId: string; repository?: string; identity?: string } | null)[] = [];
    const availableProjects = [...projects, ...(next.scope.targets ?? []).filter(target => !projects.some(p => p.id === target.projectId)).map(target => ({ id: target.projectId, name: target.projectId, path: "" }))];
    for (const available of availableProjects) {
      const existing = next.scope.targets?.find(target => target.projectId === available.id);
      const checked = ui.checkbox(available.name, !!existing); const repo = ui.input(existing?.repository ?? ""); const targetIdentity = ui.input(existing?.identity ?? "");
      targetFields.push({ projectId: available.id, repository: repo, identity: targetIdentity });
      const row = h("div", "automations-target"); row.append(checked.label, ui.field(label("repository"), repo), ui.field(label("identity"), targetIdentity));
      const toggle = () => { repo.disabled = targetIdentity.disabled = !checked.control.checked; }; checked.control.onchange = toggle; toggle(); targetSection.append(row);
      targetReads.push(() => checked.control.checked ? { projectId: available.id, repository: repo.value.trim() || undefined, identity: targetIdentity.value.trim() || identity.value.trim() || undefined } : null);
    }
    dialog.body.append(targetSection);
    get.push(() => { next.scope.targets = targetReads.map(read => read()).filter((target): target is NonNullable<typeof target> => target !== null); });
    for (const [name, prop] of [["allowWrites", "allowWrites"], ["allowCommit", "allowCommit"], ["allowPush", "allowPush"], ["requireApproval", "requireMergeApproval"], ["requirePublishApproval", "requirePublishApproval"]] as const) { const check = ui.checkbox(label(name), next.policy[prop] ?? prop === "requirePublishApproval"); dialog.body.append(check.label); get.push(() => { next.policy[prop] = check.control.checked; }); }
    for (const [name, prop, min] of [["concurrency", "maxConcurrentRuns", 1], ["retries", "maxRetries", 0], ["turns", "maxAgentTurns", 1]] as const) { const input = ui.input(String(next.policy[prop])); input.type = "number"; input.min = String(min); input.required = true; dialog.body.append(ui.field(label(name), input)); get.push(() => { next.policy[prop] = Number(input.value); }); }
    const budget = ui.input(next.policy.maxCostUsd === undefined ? "" : String(next.policy.maxCostUsd)); budget.type = "number"; budget.min = "0.01"; budget.step = "any";
    dialog.body.append(ui.field(label("budget"), budget, label("budgetHelp"))); get.push(() => { if (budget.value) next.policy.maxCostUsd = Number(budget.value); else delete next.policy.maxCostUsd; });
    dialog.open();
  }

  function renderSimulation() {
    footer.replaceChildren(); footer.hidden = !draft || view !== "graph"; if (!draft) return;
    const details = h("details", "automations-fixture"); details.append(h("summary", "", label("simulation")));
    const input = ui.input(fixtureText, true); input.classList.add("automations-json"); input.oninput = () => { fixtureText = input.value; }; details.append(ui.field(label("fixture"), input));
    footer.append(toolbar([details, h("span", "ui-hint", label("effectsSuppressed"))], [act("simulate", () => void task(async () => {
      if (!draft) return;
      let fixture; try { fixture = JSON.parse(fixtureText); if (!fixture || typeof fixture !== "object") throw new Error(); } catch { throw new Error(label("invalidJson")); }
      const base = JSON.stringify(draft); const result = await invoke("automations_simulate", { workflow: structuredClone(draft), fixture });
      if (alive && JSON.stringify(draft) === base) { simulation = result; renderContent(); renderSimulation(); }
    }), "pri")]));
    if (simulation) {
      const evidence = h("details", "automations-evidence") as HTMLDetailsElement; evidence.open = true; evidence.append(h("summary", "", label("evidence")));
      for (const step of simulation.steps) {
        const row = h("details", `automations-step ${step.status}`); row.append(h("summary", "", `${draft.nodes.find(n => n.id === step.nodeId)?.label ?? step.nodeId} · ${step.port ? portLabel(step.port) : label(step.status)}`));
        row.append(h("p", "ui-hint", step.message), h("small", "", label("input")), h("pre", "automations-json", json(step.input)), h("small", "", label("output")), h("pre", "automations-json", json(step.output))); evidence.append(row);
      }
      footer.append(evidence);
    }
  }
  function renderRuns() {
    if (!draft) return;
    const section = h("div", "automations-records"); const runOnce = act("run", () => {
      if (!draft) return;
      const workflow = draft;
      const event = ui.input("{}", true);
      const dialog = ui.formDialog({ error: fromBack, title: label("run"), save: label("run"), cancel: label("cancel"), submit: async () => {
        let value; try { value = JSON.parse(event.value); } catch { throw new Error(label("invalidJson")); }
        await invoke("automations_run", { id: workflow.id, event: value }); await load(); if (alive) renderContent();
      } });
      dialog.body.append(h("p", "ui-hint", t("automations.runSaved", { revision: workflow.revision })), ui.field(label("runEvent"), event)); dialog.open();
    }, "pri"); runOnce.disabled = dirty() || !draft.revision;
    section.append(toolbar([h("h3", "", label("runs"))], [runOnce, act("refresh", () => void task(async () => { await load(); if (alive) renderContent(); }))]));
    const runs = snapshot.runs.filter(r => r.workflow.id === draft!.id).sort((a, b) => b.createdAt - a.createdAt);
    if (!runs.length) section.append(listState({ kind: "empty", text: label("noRuns") }));
    for (const run of runs) {
      const item = h("details", "automations-run"); item.append(h("summary", "", `${new Date(run.createdAt * (run.createdAt < 1e12 ? 1000 : 1)).toLocaleString()} · ${label(run.status === "succeeded" ? "completed" : run.status === "awaitingApproval" ? "approval" : run.status === "queued" ? "waiting" : run.status)} · v${run.workflow.revision}`));
      const history = h("ol", "");
      for (const event of run.history) history.append(h("li", "", `${event.nodeId ? `${run.workflow.nodes.find(n => n.id === event.nodeId)?.label ?? event.nodeId}: ` : ""}${event.message}`));
      item.append(history, h("pre", "automations-json", json(run.outputs)), act("definition", () => jsonDialog(label("definition"), run.workflow), "ghost"));
      if (run.measuredCostUsd !== undefined) item.append(h("p", "ui-hint", t("automations.measuredCost", { amount: new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", maximumFractionDigits: 4 }).format(run.measuredCostUsd) })));
      if (run.costUnknown) item.append(h("p", "ui-hint", label("unknownCost")));
      if (run.status === "awaitingApproval") {
        const waiting = [...run.history].reverse().find(e => e.nodeId && run.workflow.nodes.some(n => n.id === e.nodeId && n.config.type === "approval"));
        if (waiting?.nodeId) { const nodeId = waiting.nodeId; item.append(act("approval", () => {
          const dialog = ui.formDialog({ error: fromBack, title: label("approval"), save: label("approve"), cancel: label("cancel"), submit: async () => { await invoke("automations_approve", { runId: run.id, nodeId, headSha: sha.value || undefined }); await load(); if (alive) renderContent(); } });
          const event = run.event && typeof run.event === "object" ? run.event as Record<string, unknown> : {};
          const sha = ui.input(typeof event.headSha === "string" ? event.headSha : ""); sha.readOnly = !!sha.value;
          dialog.body.append(h("p", "ui-hint", label("approvalBinding")), ui.field(label("remoteHeadSha"), sha), h("pre", "automations-json", json(run.event)));
          const evidence = publicationEvidence(run);
          if (evidence.length) {
            dialog.body.append(h("h3", "", label("publicationEvidence")), h("p", "ui-hint", label("publicationEvidenceHelp")));
            for (const entry of evidence) {
              const output = entry.value && typeof entry.value === "object" ? entry.value as Record<string, unknown> : {};
              dialog.body.append(h("b", "", run.workflow.nodes.find(n => n.id === entry.nodeId)?.label ?? entry.operation));
              if (typeof output.headSha === "string") dialog.body.append(h("small", "", label("localCommitSha")), h("pre", "automations-json", output.headSha));
              dialog.body.append(h("pre", "automations-json", json(entry.value)));
            }
          }
          dialog.open();
        }, "pri")); }
      }
      if (run.status === "paused") item.append(act("resume", () => void task(async () => { await invoke("automations_resume", { runId: run.id }); await load(); if (alive) renderContent(); })));
      if (!["succeeded", "failed", "cancelled"].includes(run.status)) item.append(act("cancelRun", () => void task(async () => { await invoke("automations_cancel", { runId: run.id }); await load(); if (alive) renderContent(); }), "danger"));
      section.append(item);
    }
    for (const diagnostic of snapshot.diagnostics ?? []) if (diagnostic.workflowId === draft.id && diagnostic.error) section.append(listState({ kind: "error", text: diagnostic.error }));
    content.append(section);
  }
  function renderHistory() {
    if (!draft) return;
    const section = h("div", "automations-records"); section.append(sectionHeader({ title: label("history"), description: label("revisionHelp") }));
    const revisions = snapshot.revisions.filter(w => w.id === draft!.id).sort((a, b) => b.revision - a.revision);
    if (!revisions.length) section.append(listState({ kind: "empty", text: label("noHistory") }));
    for (const revision of revisions) {
      const item = h("div", "automations-revision"); item.append(h("b", "", t("automations.runRevision", { revision: revision.revision })), h("span", "ui-hint", revision.name), act("definition", () => jsonDialog(label("definition"), revision), "ghost"), act("restore", () => { if (draft) mutate({ ...structuredClone(revision), revision: draft.revision, enabled: draft.enabled }); })); section.append(item);
    }
    if (draft.revision) section.append(act("delete", () => {
      const dialog = ui.formDialog({ error: fromBack, title: label("delete"), save: label("delete"), cancel: label("cancel"), submit: async () => { if (!draft) return; await invoke("automations_delete", { id: draft.id }); await load(); if (snapshot.workflows[0]) choose(snapshot.workflows[0]); else { draft = null; saved = ""; render(); } } }); dialog.body.append(h("p", "", label("deleteConfirm"))); dialog.open();
    }, "danger"));
    content.append(section);
  }
  function jsonDialog(title: string, value: unknown) {
    const dialog = ui.formDialog({ error: fromBack, title, save: label("close"), cancel: label("cancel"), submit: async () => {} }); dialog.body.append(h("pre", "automations-json", json(value))); dialog.open();
  }
  const escape = (event: KeyboardEvent) => { if (event.key === "Escape" && connecting) { connecting = null; say(""); renderContent(); } };
  root.addEventListener("keydown", escape);
  content.append(listState({ kind: "loading", text: label("loading") }));
  const initialize = async () => {
    try { await load(); if (alive) { if (snapshot.workflows[0]) choose(snapshot.workflows[0]); else render(); } }
    catch (error) {
      if (!alive) return;
      unavailable = automationTransportUnavailable(error); root.classList.toggle("unavailable", unavailable);
      if (unavailable) { render(); content.replaceChildren(listState({ kind: "error", text: label("unsupported") })); }
      else content.replaceChildren(listState({ kind: "error", text: fromBack(error), retry: { label: label("retry"), run: () => { void task(initialize); } } }));
    }
  };
  void task(initialize);
  return () => { alive = false; root.removeEventListener("keydown", escape); root.remove(); };
}
