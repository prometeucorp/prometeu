import { invoke } from "./ipc";
import { fromBack, t, type Key } from "./i18n";
import { h } from "./util";
import { choiceLabel, defaultChoice } from "./model-choice";
import { openModelPicker } from "./model-picker";
import type { Project } from "./types";
import type { IconName } from "./icons";
import { automationDiagram } from "./automations-diagram";
import { composer } from "./components/chat/composer";
import { conversationBlock, errorCard, noticeCard } from "./components/chat/blocks";
import { renderUserMessage } from "./components/chat/content";
import * as ui from "./ui";
import { sectionHeader, toolbar, itemRow, listState } from "./components/compositions";
import { localizeBuiltinTemplate, automationTransportUnavailable, pendingApprovalNode, publicationEvidence, blankWorkflow, outputPorts, workflowDiff, workflowReadingOrder } from "./automations-model";
import type { AutomationMessage, AutomationProposal, AutomationSnapshot, NodeKind, SimulationResult, ValidationIssue, Workflow } from "./automations-model";
import "./automations.css";

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
  let view: "conversation" | "graph" | "runs" | "history" = "conversation";
  let graphView: "steps" | "diagram" = "diagram";
  let home = true;
  let diagramCleanup: (() => void) | null = null;
  let issues: ValidationIssue[] | null = null;
  let simulation: SimulationResult | null = null;
  let proposal: { result: AutomationProposal; base: string } | null = null;
  let messages: { role: "user" | "assistant" | "validation" | "error" | "notice"; text: string }[] = [];
  let sending = false;
  let slowReply = false;
  let failedRequest: { request: string; history: AutomationMessage[] } | null = null;
  let busy = false;
  let unavailable = false;
  let requestText = "";
  const preferredModel = defaultChoice();
  let proposalModel: { agent: "claude" | "codex"; model: string } = preferredModel && (preferredModel.agent === "codex" || preferredModel.agent === "claude") ? { agent: preferredModel.agent, model: preferredModel.model } : { agent: "claude", model: "" };
  let fixtureText = '{\n  "event": { "ready": true },\n  "outputs": {}\n}';
  const root = h("section", "automations"); root.setAttribute("aria-label", label("title"));
  const header = h("div", "automations-header");
  const library = h("div", "automations-library");
  library.setAttribute("role", "region"); library.setAttribute("aria-label", label("library"));
  const bar = h("div", "automations-bar");
  const body = h("div", "automations-workbench");
  const assistant = h("aside", "automations-assistant chatwrap"); assistant.hidden = true;
  const thread = h("div", "feed");
  thread.setAttribute("role", "log"); thread.setAttribute("aria-label", label("conversation"));
  const input = composer({ send: () => void sendMessage() });
  const prompt = input.area;
  prompt.placeholder = label("request"); prompt.setAttribute("aria-label", label("request"));
  input.withModel.hidden = false; input.effort.hidden = true;
  const resizePrompt = () => { prompt.style.height = "auto"; prompt.style.height = `${Math.min(prompt.scrollHeight, window.innerHeight * .25)}px`; };
  prompt.oninput = () => { requestText = prompt.value; input.send.disabled = sending || busy || !requestText.trim(); resizePrompt(); };
  prompt.onkeydown = event => { if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); void sendMessage(); } };
  input.model.onclick = () => openModelPicker(input.model, { current: proposalModel, only: ["claude", "codex"], select: choice => {
    if (choice.agent === "claude" || choice.agent === "codex") { proposalModel = { agent: choice.agent, model: choice.model }; renderAssistant(); }
  } });
  assistant.append(thread, input.root);
  const main = h("div", "automations-main");
  const tabs = h("div", "automations-tabs");
  const validation = h("div", "automations-validation");
  const content = h("div", "automations-content");
  const footer = h("div", "automations-simulation");
  const notice = h("div", "automations-notice"); notice.setAttribute("role", "status"); notice.setAttribute("aria-live", "polite");
  main.append(content, footer); body.append(assistant, main);
  root.append(header, library, bar, tabs, validation, body, notice); host.replaceChildren(root);
  const dirty = () => !!draft && JSON.stringify(draft) !== saved;
  const say = (message: string, error = false) => { notice.textContent = message; notice.dataset.error = String(error); notice.setAttribute("role", error ? "alert" : "status"); };
  const act = (name: string, run: () => void, variant: ui.ButtonVariant = "outline") => { const control = ui.button(label(name), run, variant); control.dataset.focus = `automations-${name}`; return control; };
  const task = async (run: () => Promise<void>) => {
    if (busy) return;
    busy = true; root.setAttribute("aria-busy", "true"); renderHeader();
    const disabled = [...root.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLTextAreaElement>("button, input, textarea")].map(control => ({ control, disabled: control.disabled }));
    disabled.forEach(({ control }) => { control.disabled = true; });
    try { await run(); } catch (error) { if (alive) say(fromBack(error), true); }
    finally { disabled.forEach(item => { if (item.control.isConnected) item.control.disabled = item.disabled; }); busy = false; if (alive) { root.removeAttribute("aria-busy"); renderHeader(); renderAssistant(); renderValidation(); } }
  };
  const mutate = (next: Workflow, redraw = true) => {
    draft = next; issues = null; simulation = null;
    if (redraw) render(); else { renderHeader(); renderValidation(); }
  };
  const load = async () => { const [next, board] = await Promise.all([invoke("automations_snapshot"), invoke("load_board")]); snapshot = next; projects = board.projects; };
  const choose = (workflow: Workflow, isNew = false) => {
    home = false;
    if (draft?.id !== workflow.id) { messages = []; failedRequest = null; requestText = ""; view = isNew ? "conversation" : "graph"; graphView = "diagram"; }
    draft = structuredClone(workflow); saved = isNew ? "" : JSON.stringify(draft); selected = null; issues = null; simulation = null; proposal = null; render();
  };
  const switchTo = (workflow: Workflow, isNew = false) => {
    if (sending) return;
    if (draft?.id === workflow.id) { home = false; render(); return; }
    if (!dirty() && !proposal && !messages.length && !requestText.trim()) { choose(workflow, isNew); return; }
    const dialog = ui.formDialog({ error: fromBack, title: label("leaveDraft"), save: label("discard"), cancel: label("cancel"), submit: async () => choose(workflow, isNew) }); dialog.open();
  };

  function renderHeader() {
    const locked = busy || sending;
    const actions: HTMLElement[] = [];
    const newFlow = act("new", createMenu, home ? "pri" : "ghost"); newFlow.disabled = locked;
    if (home) {
      header.replaceChildren(sectionHeader({ title: label("title"), description: label("libraryHelp"), actions: [newFlow] }));
      return;
    }
    if (draft?.nodes.length) {
      const save = act("save", () => void task(async () => {
        if (!draft) return;
        const candidate = structuredClone(draft);
        issues = await invoke("automations_validate", { workflow: candidate }); renderValidation();
        if (issues.length) return;
        const result = await invoke("automations_save", { workflow: candidate, expectedRevision: candidate.revision || null });
        await load(); if (alive) { choose(result); say(label("saved")); }
      }), "pri"); save.disabled = locked || !!proposal;
      actions.push(save);
      if (draft.revision) {
        const activate = act(draft.enabled ? "pause" : "activate", () => void task(async () => {
          if (!draft) return;
          const result = await invoke("automations_pause", { id: draft.id, paused: draft.enabled }); await load(); if (alive) choose(result);
        })); activate.disabled = locked || dirty() || !!proposal; actions.push(activate);
      }
    }
    const back = act("library", openLibrary); back.disabled = locked;
    actions.unshift(back);
    if (view === "conversation") {
      const policy = act("policy", () => { if (!draft) choose(blankWorkflow(label("untitled")), true); policyDialog(); }, "ghost");
      policy.disabled = locked; actions.push(policy);
    }
    actions.push(newFlow);
    header.replaceChildren(sectionHeader({ title: draft?.name ?? label("title"), description: unavailable ? label("unsupportedHelp") : undefined, actions: unavailable ? [] : actions }));
    bar.replaceChildren(); bar.hidden = !draft || view === "conversation";
    if (!draft || view === "conversation") return;
    bar.append(h("span", "automations-state", label(draft.enabled ? "active" : "paused")), h("span", "automations-version", `v${draft.revision}`), h("span", "ui-hint", dirty() ? label("draft") : label("saved")));
    const tools = h("div", "automations-bar-tools");
    tools.append(act("policy", policyDialog, "ghost"), act("definition", () => jsonDialog(label("definition"), draft), "ghost")); bar.append(tools);
  }

  function openLibrary() {
    home = true; say(""); render();
    void task(async () => { await load(); if (alive) render(); });
  }

  function renderLibrary() {
    library.replaceChildren();
    if (!home) return;
    const workflows = [...snapshot.workflows];
    if (draft && !workflows.some(workflow => workflow.id === draft?.id)) workflows.unshift(draft);
    const savedSection = h("section", "automations-library-section");
    savedSection.append(sectionHeader({ title: label("library"), count: workflows.length }));
    const search = ui.input(""); search.type = "search"; search.placeholder = label("searchWorkflows"); search.setAttribute("aria-label", label("searchWorkflows"));
    const rows = h("div", "automations-library-rows");
    const draw = () => {
      rows.replaceChildren();
      const visible = workflows.filter(workflow => (workflow.id === draft?.id ? draft.name : workflow.name).toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase()));
      for (const workflow of visible) {
        const current = workflow.id === draft?.id;
        const unsaved = current && (dirty() || !!proposal);
        const row = itemRow({ title: current ? draft!.name : workflow.name, glyph: "git-branch",
          description: [workflow.revision ? label(workflow.enabled ? "active" : "paused") : label("unsavedWorkflow"),
            workflow.revision ? `v${workflow.revision}` : "", workflow.scope.repository ?? ""].filter(Boolean).join(" · "),
          status: unsaved ? label("draftSessionHelp") : undefined,
          actions: [act(current ? "continueWorkflow" : "openWorkflow", () => switchTo(workflow), "outline")] });
        rows.append(row.root);
      }
      if (!visible.length) rows.append(listState({ kind: "empty", text: label(workflows.length ? "noMatchingWorkflows" : "noSavedWorkflows") }));
    };
    search.oninput = draw; draw();
    if (workflows.length) savedSection.append(search);
    savedSection.append(rows);
    const templates = h("section", "automations-library-section");
    templates.append(sectionHeader({ title: label("templateLibrary"), description: label("templateLibraryHelp") }));
    const cards = h("div", "automations-template-grid");
    for (const original of snapshot.templates) {
      const template = localizeBuiltinTemplate(original, key => t(key as Key));
      const row = itemRow({ title: template.name, glyph: "file", description: label(`templateSummary.${template.id}`),
        actions: [act("useTemplate", () => createMenu(template.id))] });
      cards.append(row.root);
    }
    templates.append(cards); library.append(savedSection, templates);
  }

  function createMenu(templateId?: string) {
    const templates = snapshot.templates.map(template => localizeBuiltinTemplate(template, key => t(key as Key)));
    const dialog = ui.formDialog({ error: fromBack, title: label("new"), save: label("new"), cancel: label("cancel"), submit: async () => {
      const template = templates.find(w => w.id === pick.value);
      const workflow = template ? structuredClone(template) : blankWorkflow(name.value.trim() || label("untitled"));
      workflow.id = crypto.randomUUID(); workflow.revision = 0; workflow.enabled = false; workflow.name = name.value.trim() || template?.name || label("untitled"); switchTo(workflow, true);
    } });
    const name = ui.input(templates.find(template => template.id === templateId)?.name ?? ""); name.placeholder = label("untitled");
    const pick = ui.select(templateId ?? "", [["", label("blank")], ...templates.map(w => [w.id, w.name] as [string, string])]);
    let suggestedName = name.value;
    pick.onchange = () => { const nextName = templates.find(w => w.id === pick.value)?.name ?? ""; if (!name.value || name.value === suggestedName) name.value = nextName; suggestedName = nextName; };
    dialog.body.append(ui.field(label("name"), name), ui.field(label("templates"), pick.control)); dialog.open();
  }

  async function sendMessage(request = requestText.trim(), fromComposer = true, retryHistory?: AutomationMessage[]) {
    if (sending || busy || !request.trim()) return;
    if (!draft) choose(blankWorkflow(label("untitled")), true);
    if (!draft) return;
    const base = JSON.stringify(draft);
    const current = structuredClone(proposal?.base === base ? proposal.result.workflow : draft);
    const history: AutomationMessage[] = retryHistory ?? messages.flatMap(({ role, text }) => role === "user" || role === "assistant" || role === "validation" ? [{ role, text }] : []);
    if (!retryHistory) messages.push({ role: "user", text: request });
    if (fromComposer) requestText = ""; sending = true; failedRequest = null; slowReply = false;
    const slowTimer = window.setTimeout(() => { if (alive && sending) { slowReply = true; renderAssistant(true); } }, 30_000);
    renderHeader(); renderAssistant(true); renderValidation(); prompt.focus({ preventScroll: true });
    try {
      const result = await invoke("automations_propose", { prompt: request, history, projectId: current.scope.projectId, workflow: current, provider: proposalModel.agent, model: proposalModel.model });
      if (!alive) return;
      messages.push({ role: "assistant", text: result.summary });
      if (result.workflow) proposal = { result: { workflow: result.workflow, summary: result.summary }, base };
    } catch (error) {
      if (!alive) return;
      const cause = fromBack(error);
      const message = cause === "automation_conversation_limit" ? label("conversationLimit")
        : cause === "automation_proposal_timeout" ? label("proposalTimeout")
        : cause === "automation_proposal_output_limit" ? label("proposalOutputLimit")
        : cause === "automation_proposal_cli_missing" ? label("proposalCliMissing")
        : cause.startsWith("automation_proposal_transport:") ? `${label("proposalInterrupted")}\n${cause.slice("automation_proposal_transport: ".length)}`
        : cause === "automation_proposal_unavailable" ? label("proposalInterrupted")
        : cause.startsWith("automation_proposal_account:") ? `${label("proposalAccount")}\n${cause}`
        : cause.startsWith("automation_proposal_format:") ? `${label("proposalFormatRejected")}\n${cause.slice("automation_proposal_format: ".length)}`
        : cause.startsWith("automation_proposal_failed") ? `${label("proposalProviderFailed")}${cause.startsWith("automation_proposal_failed: ") ? ` ${cause.slice("automation_proposal_failed: ".length)}` : ""}`
          : cause.startsWith("automation_proposal_unexpected_tool: ") ? t("automations.proposalToolBlocked", { kind: cause.slice("automation_proposal_unexpected_tool: ".length) })
            : /^automation_proposal_(schema|invalid|response)/.test(cause) ? `${label("proposalInvalid")}\n${cause}`
              : `${label("proposalUnavailable")} ${cause}`;
      messages.push({ role: /^automation_proposal_(schema|invalid|response)/.test(cause) ? "validation" : "error", text: message });
      failedRequest = { request, history: /^automation_proposal_(schema|invalid|response)/.test(cause)
        ? messages.flatMap(({ role, text }) => role === "user" || role === "assistant" || role === "validation" ? [{ role, text }] : []) : history };
      if (fromComposer && !requestText) requestText = request;
    } finally {
      window.clearTimeout(slowTimer);
      sending = false;
      if (alive) { renderHeader(); renderAssistant(true); renderValidation(); }
    }
  }

  function renderAssistant(latest = false) {
    const scroll = thread.scrollTop;
    thread.replaceChildren();
    for (const message of messages) {
      let turn: HTMLElement;
      if (message.role === "assistant") {
        turn = h("div", "turn bot");
        turn.append(conversationBlock({ kind: "text", text: message.text }, false));
      } else if (message.role === "user") {
        turn = h("div", "turn user");
        const bubble = h("div", "bubble"); renderUserMessage(bubble, message.text); turn.append(bubble);
      } else if (message.role === "error" || message.role === "validation") {
        turn = errorCard(message.text); turn.setAttribute("role", "alert");
      } else turn = noticeCard(message.text, t("chat.notice.title"));
      thread.append(turn);
    }
    if (!sending && failedRequest) {
      const retry = failedRequest;
      const control = act("retry", () => void sendMessage(retry.request, requestText.trim() === retry.request, retry.history));
      control.disabled = busy; thread.append(toolbar([control]));
    }
    if (sending) {
      const pending = conversationBlock({ kind: "thinking", text: "" }, true);
      pending.setAttribute("role", "status"); pending.setAttribute("aria-label", label("responding")); thread.append(pending);
      if (slowReply) pending.append(h("p", "ui-hint", label("respondingSlow")));
    } else if (proposal && draft) {
      const review = h("div", "automations-proposal-review");
      review.append(h("strong", "", proposal.result.workflow.name));
      const outline = h("details", "automations-proposal-outline");
      outline.append(h("summary", "", t("automations.reviewSteps", { count: proposal.result.workflow.nodes.length })));
      const steps = h("ul", "");
      for (const node of proposal.result.workflow.nodes) steps.append(h("li", "", `${node.label} · ${label(node.config.type)}`));
      outline.append(steps); review.append(outline);
      const changes = workflowDiff(draft, proposal.result.workflow);
      const details = h("details", "automations-proposal-details");
      details.append(h("summary", "", label("technicalChanges")));
      for (const change of changes) {
        const item = h("details", "automations-change"); item.append(h("summary", "", change.path));
        item.append(h("small", "", label("before")), h("pre", "automations-before", json(change.before)), h("small", "", label("after")), h("pre", "automations-after", json(change.after))); details.append(item);
      }
      if (changes.length) review.append(details);
      const apply = act("applyProposal", () => {
        if (!proposal || !draft) return;
        if (JSON.stringify(draft) !== proposal.base) { say(label("staleProposal"), true); return; }
        const next = { ...proposal.result.workflow, id: draft.id, revision: draft.revision, enabled: draft.enabled };
        proposal = null; messages.push({ role: "notice", text: label("appliedToDraft") }); mutate(next);
      }, "pri"); apply.disabled = JSON.stringify(draft) !== proposal.base;
      review.append(toolbar([apply, act("discard", () => { proposal = null; renderAssistant(); }, "ghost")]));
      if (apply.disabled) review.append(h("p", "ui-hint", label("staleProposal")));
      thread.append(review);
    } else if (!messages.length) {
      const welcome = h("div", "automations-welcome");
      welcome.append(h("h2", "", label("conversationStart")), h("p", "", label("conversationHelp")));
      thread.append(welcome);
    }
    if (prompt.value !== requestText) prompt.value = requestText;
    input.send.disabled = sending || busy || !requestText.trim();
    input.model.disabled = sending || busy;
    input.model.replaceChildren(h("span", "", choiceLabel(proposalModel)));
    input.model.setAttribute("aria-label", `${label("proposalModel")}: ${choiceLabel(proposalModel)}`);
    input.hint.textContent = t("chat.input.hint");
    resizePrompt();
    thread.scrollTop = scroll;
    if (latest) {
      if (sending) thread.scrollTop = thread.scrollHeight;
      else (proposal ? thread.lastElementChild?.previousElementSibling : thread.lastElementChild)?.scrollIntoView({ block: "start" });
    }
  }

  function issueLabel(issue: ValidationIssue) {
    const known: Record<string, string> = { agent_schema: "fixAgentSchema", target: "fixTarget", scope: "fixScope", agent_tools: "fixAgentTools" };
    const text = known[issue.code] ? label(known[issue.code]) : label("fixStep");
    const node = draft?.nodes.find(node => node.id === issue.nodeId);
    return node ? `${node.label}: ${text}` : text;
  }

  async function connectAccount() {
    if (!draft) return;
    const next = structuredClone(draft);
    if (next.nodes.some(node => node.config.type === "trigger" && node.config.event.startsWith("linear."))) {
      const status = await invoke("linear_status");
      if (!status.connected || !status.who) throw new Error(label("connectLinear"));
      next.scope.identity ||= status.who.id;
    } else {
      const [login, mappings] = await Promise.all([invoke("github_identity"), invoke("github_projects")]);
      next.scope.identity ||= login;
      next.scope.repository ||= mappings.find(mapping => mapping.project === next.scope.projectId)?.repository;
      for (const target of next.scope.targets ?? []) {
        target.identity ||= login;
        target.repository ||= mappings.find(mapping => mapping.project === target.projectId)?.repository;
      }
    }
    if (!alive) return;
    mutate(next);
    issues = await invoke("automations_validate", { workflow: next });
    if (alive) renderValidation();
  }

  function renderValidation() {
    validation.replaceChildren();
    validation.hidden = home || !draft || (view !== "graph" && !issues?.length);
    if (!draft) return;
    if (issues?.length) {
      const details = ui.disclosure(t("automations.saveBlocked", { n: issues.length }));
      details.classList.add("automations-errors");
      for (const issue of issues) {
        const item = h("div", "automations-issue");
        item.append(h("p", "", issueLabel(issue)));
        const technical = ui.disclosure(label("technicalChanges"), h("pre", "", issue.message));
        item.append(technical);
        if (issue.nodeId) item.append(ui.button(label("editWithAssistant"), () => askForChange(issue.nodeId), "ghost"));
        details.append(item);
      }
      const repair = act("repairWithAssistant", () => { showView("conversation"); void sendMessage(label("repairRequest"), false); });
      repair.disabled = busy || sending;
      const actions = [repair];
      if (issues.some(issue => ["target", "scope"].includes(issue.code))) {
        const account = act("useAccount", () => void task(connectAccount), "ghost"); account.disabled = busy || sending; actions.push(account);
      }
      validation.append(details, toolbar(actions));
    } else {
      validation.append(h("span", issues ? "automations-valid" : "ui-hint", issues ? t("automations.valid", { nodes: draft.nodes.length, edges: draft.edges.length }) : label("unvalidated")));
      const check = act("validate", () => void task(async () => {
        if (!draft) return;
        const base = JSON.stringify(draft), result = await invoke("automations_validate", { workflow: structuredClone(draft) });
        if (alive && JSON.stringify(draft) === base) { issues = result; renderValidation(); }
      }), "ghost"); check.disabled = busy || sending; validation.append(check);
    }
  }

  function render() {
    if (!alive) return;
    library.hidden = !home; body.hidden = home; tabs.hidden = home; bar.hidden = home;
    root.classList.toggle("library-open", home);
    if (home) { diagramCleanup?.(); diagramCleanup = null; validation.hidden = true; renderHeader(); renderLibrary(); return; }
    assistant.hidden = unavailable || view !== "conversation";
    main.hidden = !unavailable && view === "conversation";
    body.classList.toggle("conversation", view === "conversation");
    root.classList.toggle("building", view === "conversation");
    renderHeader(); renderAssistant(); renderValidation(); renderTabs(); renderContent(); renderSimulation();
  }
  function renderTabs() {
    tabs.replaceChildren();
    const names: (typeof view)[] = draft?.revision ? ["conversation", "graph", "runs", "history"] : ["conversation", "graph"];
    const buttons = names.map(name => {
      const b = act(name, () => showView(name), "ghost"); b.classList.toggle("selected", name === view); b.setAttribute("aria-pressed", String(name === view)); return b;
    });
    const display = (["steps", "diagram"] as const).map(mode => {
      const button = act(mode, () => { graphView = mode; showView("graph"); }, "ghost");
      button.setAttribute("aria-pressed", String(graphView === mode)); button.classList.toggle("selected", graphView === mode); return button;
    });
    tabs.append(toolbar(buttons, view === "graph" ? [...display, act("editWithAssistant", () => askForChange())] : []));
  }
  function askForChange(nodeId?: string) {
    const node = draft?.nodes.find(node => node.id === nodeId);
    if (node) requestText = [requestText.trim(), t("automations.editStepRequest", { name: node.label })].filter(Boolean).join("\n\n");
    showView("conversation"); prompt.focus({ preventScroll: true });
  }

  function showView(next: typeof view) {
    view = next; render();
    if (view === "graph" && selected) {
      const node = [...content.querySelectorAll<HTMLElement>("[data-node-id]")].find(node => node.dataset.nodeId === selected);
      node?.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
  }
  function renderContent() {
    diagramCleanup?.(); diagramCleanup = null;
    const scroll = content.querySelector(".automations-outline")?.scrollTop ?? 0;
    content.replaceChildren();
    if (view === "conversation") return;
    if (!draft || !draft.nodes.length) { content.append(listState({ kind: "empty", text: label("emptyGraph") }), act("editWithAssistant", () => askForChange(), "pri")); return; }
    if (view === "runs") { renderRuns(); return; }
    if (view === "history") { renderHistory(); return; }
    if (graphView === "steps") { const steps = renderSteps(); if (steps) steps.scrollTop = scroll; return; }
    diagramCleanup = automationDiagram(content, draft, portLabel);
  }
  function renderSteps() {
    if (!draft) return;
    const section = h("div", "automations-records automations-outline");
    section.append(sectionHeader({ title: label("stepsTitle"), description: label("stepsHelp") }));
    const nodes = workflowReadingOrder(draft);
    const names = new Map(nodes.map((node, i) => [node.id, `${i + 1}. ${node.label}`]));
    const list = h("ol", "automations-outline-list");
    const focusStep = (id: string) => {
      selected = id;
      for (const item of list.querySelectorAll<HTMLElement>("[data-node-id]")) {
        item.classList.toggle("selected", item.dataset.nodeId === id);
        if (item.dataset.nodeId === id) { item.focus({ preventScroll: true }); item.scrollIntoView({ block: "nearest" }); }
      }
    };
    for (const node of nodes) {
      const item = h("li", "automations-outline-item"); item.dataset.nodeId = node.id; item.tabIndex = -1;
      item.setAttribute("aria-label", names.get(node.id)!); item.classList.toggle("selected", selected === node.id);
      item.classList.toggle("traversed", !!simulation?.steps.some(step => step.nodeId === node.id && step.status === "simulated"));
      const edit = act("editWithAssistant", () => askForChange(node.id), "ghost");
      const row = itemRow({ title: names.get(node.id)!, glyph: glyphs[node.config.type],
        description: node.config.type === "approval" ? node.config.message : label(`${node.config.type}Help`), actions: [edit] });
      const routes = h("ul", "automations-outline-routes");
      const ports = new Set([...outputPorts(node, snapshot.registry), ...draft.edges.filter(edge => edge.from === node.id).map(edge => edge.port)]);
      for (const port of ports) {
        const route = h("li", "");
        const caption = ["next", "true", "false", "error", "uncertain"].includes(port) ? label(`route.${port}`) : port;
        route.append(h("span", "", `${caption}: `));
        const edges = draft.edges.filter(edge => edge.from === node.id && edge.port === port);
        if (!edges.length) route.append(h("span", "ui-hint", label(["error", "uncertain"].includes(port) ? "unhandledRoute" : "noConnection")));
        for (const edge of edges) {
          const target = ui.button(names.get(edge.to) ?? edge.to, () => focusStep(edge.to), "ghost");
          target.disabled = !names.has(edge.to); route.append(target);
        }
        routes.append(route);
      }
      const details = ui.disclosure(label("stepDetails"), h("pre", "automations-json", json(node.config)));
      row.copy.append(routes, details); item.append(row.root); list.append(item);
    }
    if (!nodes.length) section.append(listState({ kind: "empty", text: label("emptyHelp") }));
    section.append(list); content.append(section); return section;
  }

  function policyDialog() {
    if (!draft) return;
    const next = structuredClone(draft); const get: (() => void)[] = [];
    const dialog = ui.formDialog({ error: fromBack, title: label("policy"), save: label("applyStep"), cancel: label("cancel"), submit: async () => { get.forEach(f => f()); mutate(next); } });
    const permissions = h("div", "automations-permissions");
    const controls = new Map<string, HTMLInputElement>();
    permissions.append(h("p", "ui-hint", label("autonomousRepairHelp")), act("autonomousRepair", () => {
      for (const prop of ["allowWrites", "allowCommit", "allowPush", "requireMergeApproval"]) controls.get(prop)!.checked = true;
      for (const prop of ["requirePublishApproval", "requireLocalChecks"]) controls.get(prop)!.checked = false;
    }));
    for (const [name, prop] of [["allowWrites", "allowWrites"], ["allowCommit", "allowCommit"], ["allowPush", "allowPush"], ["requireApproval", "requireMergeApproval"], ["requirePublishApproval", "requirePublishApproval"], ["requireLocalChecks", "requireLocalChecks"]] as const) {
      const check = ui.checkbox(label(name), next.policy[prop] ?? prop.startsWith("require"));
      controls.set(prop, check.control); permissions.append(check.label);
      get.push(() => { next.policy[prop] = check.control.checked; });
    }
    permissions.append(h("p", "ui-hint", label("localChecksHelp")));
    dialog.body.append(permissions);
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
        const waiting = pendingApprovalNode(run);
        if (waiting) { const nodeId = waiting.id; item.append(act("approval", () => {
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
      const dialog = ui.formDialog({ error: fromBack, title: label("delete"), save: label("delete"), cancel: label("cancel"), submit: async () => { if (!draft) return; await invoke("automations_delete", { id: draft.id }); await load(); draft = null; saved = ""; messages = []; proposal = null; requestText = ""; home = true; render(); } }); dialog.body.append(h("p", "", label("deleteConfirm"))); dialog.open();
    }, "danger"));
    content.append(section);
  }
  function jsonDialog(title: string, value: unknown) {
    const dialog = ui.formDialog({ error: fromBack, title, save: label("close"), cancel: label("cancel"), submit: async () => {} }); dialog.body.append(h("pre", "automations-json", json(value))); dialog.open();
  }
  library.append(listState({ kind: "loading", text: label("loading") }));
  const initialize = async () => {
    try { await load(); if (alive) render(); }
    catch (error) {
      if (!alive) return;
      unavailable = automationTransportUnavailable(error); root.classList.toggle("unavailable", unavailable);
      if (unavailable) { home = false; render(); content.replaceChildren(listState({ kind: "error", text: label("unsupported") })); }
      else { render(); library.replaceChildren(listState({ kind: "error", text: fromBack(error), retry: { label: label("retry"), run: () => { void task(initialize); } } })); }
    }
  };
  void task(initialize);
  return () => { alive = false; diagramCleanup?.(); root.remove(); };
}
