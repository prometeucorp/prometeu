import { terminalView } from "./terminal-view";
import type { TerminalPort } from "./terminal";
import { button, field, input } from "../ui";
import { h } from "../util";
import { t } from "../i18n";
import { sectionHeader } from "../components/compositions";
import { conversationBlock, errorCard } from "../components/chat/blocks";
import { requestCard } from "../components/chat/requests";
import { Session, type SessionPort, type Target } from "./session";
import type { Item } from "../timeline";
import "./view.css";
import { workspaceView } from "./workspace-view";
import type { WorkspacePort } from "./workspaces";

export function mount(host: HTMLElement, port: SessionPort, terminalPort: TerminalPort, workspacePort: WorkspacePort) {
  const terminal = terminalView(terminalPort);
  const session = new Session(port, paint);
  const fields = {
    distribution: input("Ubuntu"), executable: input("/home/user/bin/prometeu-runtime"),
    root: input("/home/user/.local/share/prometeu-wsl-preview"), workdir: input("/home/user/project"), codex: input("/home/user/.local/bin/codex"),
  };
  const setup = h("form", "wsl-setup") as HTMLFormElement;
  for (const key of Object.keys(fields) as (keyof Target)[]) {
    fields[key].required = true;
    setup.append(field(t(`wsl.${key}`), fields[key]));
  }
  const connect = button(t("wsl.connect"), () => {
    if (setup.reportValidity()) void session.connect(Object.fromEntries(Object.entries(fields).map(([key, value]) => [key, value.value])) as Target);
  }, "pri");
  setup.addEventListener("submit", event => { event.preventDefault(); connect.click(); });
  setup.append(connect);
  const start = button(t("wsl.start"), () => void session.start());
  const stop = button(t("chat.stop"), () => void session.stop());
  const shutdown = button(t("wsl.shutdown"), () => void session.shutdown(), "ghost");
  const disconnect = button(t("wsl.disconnect"), () => void session.disconnect(), "ghost");
  const status = h("p", "wsl-status"); status.setAttribute("role", "status");
  const error = h("div", "wsl-error"); error.setAttribute("role", "alert");
  const transcript = h("div", "wsl-transcript");
  const draft = input("", true); draft.rows = 3; draft.setAttribute("aria-label", t("wsl.message"));
  const workspaces = workspaceView(workspacePort, session, draft, paint);
  const send = button(t("chat.send"), () => void submit(), "pri");
  draft.addEventListener("input", () => { send.disabled = !session.canSend || workspaces.pending || !draft.value.trim(); });
  draft.addEventListener("keydown", event => {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); void submit(); }
  });
  const compose = h("div", "wsl-compose"); compose.append(draft, send);
  host.classList.add("wsl-shell");
  host.append(sectionHeader({ title: t("wsl.title"), description: t("wsl.description"), actions: [start, stop, disconnect, shutdown] }), setup, workspaces.root, status, error, transcript, compose, terminal.root);
  let previous: Item[] = [];
  const nodes = new Map<Item, { node: HTMLElement; signature: string }>();
  paint();

  async function submit() {
    const text = draft.value;
    if (await session.send(text)) { if (draft.value === text) draft.value = ""; }
    paint();
  }
  function render(item: Item): HTMLElement {
    switch (item.kind) {
      case "assistant": {
        const node = h("div", "turn bot");
        node.append(...item.blocks.map(block => conversationBlock(block, item.streaming)));
        return node;
      }
      case "ask": {
        const wrapper = h("fieldset", "wsl-request");
        wrapper.append(requestCard(item, {
          respond: response => void session.respond(item.id, response),
          feedbackOpen: false, feedbackChanged: () => {},
        }));
        return wrapper;
      }
      case "user": return h("div", "turn me", item.text);
      case "system": case "result": return item.error ? errorCard(item.text) : h("p", "sys", item.text);
      case "context": return h("p", "sys", t("wsl.context"));
    }
  }
  function paint() {
    if (!host.isConnected) return;
    terminal.update(session.connected && !session.switching);
    workspaces.update(session.connected);
    setup.hidden = session.connected;
    connect.disabled = session.pending;
    start.disabled = !session.connected || session.running || session.pending;
    stop.disabled = !session.running || session.pending;
    disconnect.disabled = session.pending;
    shutdown.disabled = !session.connected || session.pending;
    draft.disabled = !session.ready || workspaces.pending;
    send.disabled = !session.canSend || workspaces.pending || !draft.value.trim();
    status.textContent = t(session.pending ? "wsl.pending" : session.ready ? (session.timeline.working ? "wsl.working" : "wsl.ready") : session.running ? "wsl.starting" : session.connected ? "wsl.connected" : "wsl.offline");
    error.textContent = session.error === "sequence_gap" ? t("wsl.sequenceGap") : session.error;
    const nearBottom = transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight < 80;
    if (previous !== session.timeline.items) { nodes.clear(); transcript.replaceChildren(); previous = session.timeline.items; }
    for (const item of session.timeline.items) {
      const signature = JSON.stringify(item);
      const old = nodes.get(item);
      if (old?.signature === signature) continue;
      const node = render(item);
      if (old) old.node.replaceWith(node); else transcript.append(node);
      nodes.set(item, { node, signature });
    }
    for (const control of transcript.querySelectorAll<HTMLFieldSetElement>(".wsl-request")) control.disabled = !session.ready || session.pending;
    if (nearBottom) transcript.scrollTop = transcript.scrollHeight;
  }
  return session;
}
