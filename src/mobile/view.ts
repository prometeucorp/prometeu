import type { Note, Shared, Status } from "../../relay/src/protocol";
import { t } from "../i18n";
import { icon } from "../icons";
import { md } from "../markdown";
import * as comments from "../team-comments";
import * as member from "../team-member";
import * as viewer from "../team-viewer";
import { renderUserMessage, toolLabel } from "../chat-presentation";
import { Timeline, summary, type Block, type Item } from "../timeline";
import { h, template } from "../util";
import { button as uiButton, input as uiInput } from "../ui";
import type { Config, Organization } from "./shell";

/// Phone interface over the collaboration core: shared conversations, one attached transcript, input to the
/// owner's Mac, comments and the personal inbox. No local agent, no board.

type Screen = { kind: "list" } | { kind: "chat"; id: string; ws: string; tab: string } | { kind: "inbox" };

export type Actions = { selectOrganization(id: string): void };

export class MobileView {
  private screen: Screen = { kind: "list" };
  private organization: Organization | null = null;
  private tl = new Timeline();
  private nodes: HTMLElement[] = [];
  private transcript: HTMLElement | null = null;
  private thread: HTMLElement | null = null;
  private decoder = new TextDecoder();
  private partial = "";
  private loading = false;
  private sending = false;

  constructor(private root: HTMLElement, private config: Config, private actions: Actions) {
    root.classList.add("ui-comfortable");
  }

  setOrganization(organization: Organization | null) {
    this.organization = organization;
    this.screen = { kind: "list" };
    this.render();
  }

  /* Sink for the viewer feature. */

  live(tab: string, bytes: Uint8Array) {
    if (this.screen.kind !== "chat" || this.screen.tab !== tab || this.loading) return;
    this.partial += this.decoder.decode(bytes, { stream: true });
    const lines = this.partial.split("\n");
    this.partial = lines.pop() ?? "";
    for (const line of lines) if (line.trim()) this.paint(this.tl.push(line));
  }

  reset(tab: string, bytes: Uint8Array) {
    if (this.screen.kind === "chat" && this.screen.tab === tab) this.load(bytes);
  }

  /* Directory or comment changes. */

  changed() {
    if (this.screen.kind === "chat") {
      this.paintThread();
      this.paintComposer();
    } else this.render();
  }

  toast(text: string) {
    const box = h("div", "m-toast", text);
    box.setAttribute("role", "alert");
    this.root.append(box);
    setTimeout(() => box.remove(), 4000);
  }

  /* Screens. */

  render() {
    this.root.replaceChildren();
    if (this.screen.kind === "list") this.renderList();
    else if (this.screen.kind === "inbox") this.renderInbox();
    else this.renderChat(this.screen);
  }

  private header(title: string, back: (() => void) | null): HTMLElement {
    const bar = h("header", "m-bar");
    if (back) {
      const button = template("button", "ui-button ghost m-back", icon("x", 18));
      button.setAttribute("aria-label", t("mobile.back"));
      button.addEventListener("click", back);
      bar.append(button);
    }
    bar.append(h("h1", "", title));
    return bar;
  }

  private renderList() {
    const bar = this.header(this.organization?.name ?? "Prometeu", null);
    if (this.config.organizations.length > 1) {
      const select = h("select", "ui-select m-org") as HTMLSelectElement;
      select.setAttribute("aria-label", t("mobile.organization"));
      if (!this.organization) select.append(new Option(t("mobile.chooseOrganization"), "", true, true));
      for (const org of this.config.organizations) select.append(new Option(org.name, org.id, false, org.id === this.organization?.id));
      select.addEventListener("change", () => this.actions.selectOrganization(select.value));
      bar.append(select);
    }
    const phase = member.current().phase;
    bar.append(h("span", `m-status ${phase}`, t(`mobile.status.${phase}`)));
    const inbox = template("button", "ui-button ghost m-inbox", `${icon("inbox", 18)}<b></b>`);
    inbox.setAttribute("aria-label", t("mobile.inbox"));
    inbox.querySelector("b")!.textContent = comments.inboxCount() ? String(comments.inboxCount()) : "";
    inbox.addEventListener("click", () => { this.screen = { kind: "inbox" }; this.render(); });
    bar.append(inbox);
    this.root.append(bar);

    const main = h("main", "m-main");
    if (!this.config.organizations.length) main.append(h("p", "ui-hint m-empty", t("mobile.noOrganizations")));
    else if (!this.organization) main.append(h("p", "ui-hint m-empty", t("mobile.chooseOrganization")));
    else {
      const remotes = viewer.remotes();
      if (!remotes.length) main.append(h("p", "ui-hint m-empty", t("mobile.empty")));
      for (const { id, share } of remotes) main.append(this.card(id, share));
    }
    this.root.append(main);
  }

  private card(id: string, share: Shared): HTMLElement {
    const card = h("section", `m-card${share.online ? "" : " off"}`);
    const owner = h("div", "m-owner");
    owner.append(h("i", "dot"), h("span", "", member.nameOf(share.owner)));
    if (!share.online) owner.append(h("small", "ui-hint", t("team.offline")));
    card.append(owner, h("h2", "", share.title));
    const meta = [share.repo_name, share.branch].filter(Boolean).join(" · ");
    if (meta) card.append(h("p", "ui-hint", meta));
    const tabs = h("div", "m-tabs");
    share.tabs.forEach((tab, index) => {
      const button = h("button", `ui-button outline m-tab ${tab.status}`);
      button.append(h("span", "", tab.title || t("mobile.conversation", { n: index + 1 })), h("small", "", statusLabel(tab.status)));
      button.addEventListener("click", () => void this.open(id, share.id, tab.id));
      tabs.append(button);
    });
    card.append(tabs);
    return card;
  }

  private renderInbox() {
    this.root.append(this.header(t("mobile.inbox"), () => { this.screen = { kind: "list" }; this.render(); }));
    const main = h("main", "m-main");
    const items = comments.inboxList();
    if (!items.length) main.append(h("p", "ui-hint m-empty", t("mobile.inboxEmpty")));
    for (const item of items) {
      const row = h("button", "m-row");
      row.append(h("b", "", item.author), h("span", "", item.title), h("p", "", item.text));
      row.addEventListener("click", () => {
        const remote = viewer.remotes().find((r) => r.share.id === item.ws);
        if (!remote) return;
        const target = comments.readInbox(item.id);
        const tab = target?.tab ?? remote.share.active ?? remote.share.tabs[0]?.id;
        if (tab) void this.open(remote.id, remote.share.id, tab);
      });
      main.append(row);
    }
    this.root.append(main);
  }

  /* Conversation. */

  private async open(id: string, ws: string, tab: string) {
    this.screen = { kind: "chat", id, ws, tab };
    this.loading = true;
    this.tl = new Timeline();
    this.nodes = [];
    this.partial = "";
    this.render();
    let result: { bytes: Uint8Array } | null = null;
    try {
      result = await viewer.attach(id, tab);
    } catch (error) {
      this.toast(String(error));
    }
    if (this.screen.kind !== "chat" || this.screen.tab !== tab) return;
    this.loading = false;
    if (result) this.load(result.bytes);
    else this.paintAll();
  }

  private close() {
    viewer.detach();
    this.screen = { kind: "list" };
    this.render();
  }

  private renderChat(screen: Extract<Screen, { kind: "chat" }>) {
    const share = viewer.remotes().find((r) => r.id === screen.id)?.share;
    this.root.append(this.header(share?.title ?? "", () => this.close()));
    this.transcript = h("main", "m-main m-transcript");
    if (this.loading) this.transcript.append(h("p", "ui-hint m-empty", t("mobile.loading")));
    this.root.append(this.transcript);
    this.paintAll();

    const details = template("details", "m-comments", `<summary></summary>`);
    details.querySelector("summary")!.textContent = t("mobile.comments");
    this.thread = h("div", "m-thread");
    const form = h("form", "m-form");
    const input = uiInput("", true);
    input.placeholder = t("mobile.commentPlaceholder");
    input.setAttribute("aria-label", input.placeholder);
    input.rows = 2;
    const send = uiButton(t("mobile.comment"), undefined, "pri");
    send.type = "submit";
    form.append(input, send);
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const text = input.value.trim();
      if (!text) return;
      void comments.addNote(screen.ws, screen.tab, null, text, [], null).then((sent) => { if (sent) input.value = ""; });
    });
    details.append(this.thread, form);
    this.root.append(details);
    this.paintThread();

    const composer = h("form", "m-form m-composer");
    const box = uiInput("", true);
    box.rows = 1;
    box.placeholder = t("mobile.placeholder");
    box.setAttribute("aria-label", box.placeholder);
    const button = uiButton("", undefined, "pri");
    button.type = "submit";
    button.onclick = null;
    button.classList.add("m-send");
    button.innerHTML = icon("arrow-up", 20);
    button.setAttribute("aria-label", t("mobile.send"));
    button.title = t("mobile.send");
    const status = h("div", "m-connection");
    status.setAttribute("role", "status");
    composer.append(box, button, status);
    const viewOnly = h("p", "ui-hint m-viewonly", t("mobile.viewOnly"));
    const resize = () => {
      box.style.height = "auto";
      box.style.height = `${box.scrollHeight + 2}px`;
    };
    box.addEventListener("input", () => { resize(); this.paintComposer(); });
    composer.addEventListener("submit", async (event) => {
      event.preventDefault();
      const draft = box.value, text = draft.trim();
      if (!text || button.disabled || this.sending) return;
      box.focus({ preventScroll: true });
      this.sending = true;
      composer.setAttribute("aria-busy", "true");
      this.paintComposer();
      try {
        if (await viewer.write(text) && box.value === draft) box.value = "";
      } catch {
        this.toast(t("err.team.encryption"));
      } finally {
        this.sending = false;
        composer.removeAttribute("aria-busy");
        resize();
        this.paintComposer();
      }
    });
    this.root.append(composer, viewOnly);
    this.paintComposer();
  }

  private paintComposer() {
    if (this.screen.kind !== "chat") return;
    const id = this.screen.id;
    const share = viewer.remotes().find((r) => r.id === id)?.share;
    const box = this.root.querySelector<HTMLTextAreaElement>(".m-composer textarea");
    const button = this.root.querySelector<HTMLButtonElement>(".m-composer button");
    if (!box || !button) return;
    // Only the composer reflects rights here; the phone has no request answers to hide (ADR 0090).
    const send = viewer.rightsIn(id).send;
    box.closest<HTMLElement>(".m-composer")!.hidden = !send;
    const viewOnly = this.root.querySelector<HTMLElement>(".m-viewonly");
    if (viewOnly) viewOnly.hidden = send;
    const phase = member.current().phase;
    const online = phase === "online" && !!share?.online;
    button.disabled = !online || this.sending || !box.value.trim();
    const status = this.root.querySelector<HTMLElement>(".m-connection");
    if (status) status.textContent = online ? "" : phase !== "online" ? t(`mobile.status.${phase}`) : t("mobile.ownerOffline");
  }

  private paintThread() {
    if (this.screen.kind !== "chat" || !this.thread) return;
    const notes = comments.notesOf(this.screen.ws);
    this.thread.replaceChildren(...notes.filter((note) => !note.parent).map((root) => this.noteNode(root, notes)));
  }

  private noteNode(note: Note, all: Note[]): HTMLElement {
    const box = h("div", `m-note${note.resolved ? " resolved" : ""}`);
    const head = h("div", "m-note-head");
    head.append(h("b", "", member.nameOf(note.author)), h("small", "ui-hint", new Date(note.ts).toLocaleString()));
    if (note.resolved) head.append(h("small", "ui-badge", t("mobile.resolved")));
    box.append(head);
    if (note.quote) box.append(h("blockquote", "", note.quote));
    box.append(h("p", "", note.text));
    for (const reply of all.filter((item) => item.parent === note.id)) {
      const child = h("div", "m-reply");
      child.append(h("b", "", member.nameOf(reply.author)), h("p", "", reply.text));
      box.append(child);
    }
    return box;
  }

  private load(bytes: Uint8Array) {
    this.tl = new Timeline();
    this.partial = "";
    this.tl.load(new TextDecoder("utf-8").decode(bytes));
    this.nodes = [];
    this.paintAll();
  }

  private paintAll() {
    if (!this.transcript) return;
    this.nodes = this.tl.items.map(itemNode);
    this.transcript.replaceChildren(...this.nodes);
    this.transcript.scrollTop = this.transcript.scrollHeight;
  }

  /// Replace only the items the reducer touched; a new item appends at the end.
  private paint(touched: number[]) {
    if (!this.transcript) return;
    const stick = this.transcript.scrollHeight - this.transcript.scrollTop - this.transcript.clientHeight < 80;
    for (const at of touched) {
      const item = this.tl.items[at];
      if (!item) continue;
      const node = itemNode(item);
      if (this.nodes[at]) this.nodes[at].replaceWith(node);
      else this.transcript.append(node);
      this.nodes[at] = node;
    }
    if (stick) this.transcript.scrollTop = this.transcript.scrollHeight;
  }
}

const statusLabel = (status: Status) => t(`status.${status}`);

function itemNode(item: Item): HTMLElement {
  switch (item.kind) {
    case "user": {
      const node = h("div", "m-item m-user");
      renderUserMessage(node, item.text);
      return node;
    }
    case "assistant": {
      const box = h("div", "m-item m-assistant");
      for (const block of item.blocks) box.append(blockNode(block));
      if (item.streaming && !item.blocks.length) box.append(h("span", "m-dots", "…"));
      return box;
    }
    case "ask":
      return h("div", "m-item m-ask", t("mobile.ask", { tool: toolLabel(item.tool) }));
    case "result":
      return h("div", `m-item m-result${item.error ? " err" : ""}`, item.text);
    case "system":
      return h("div", `m-item m-system${item.error ? " err" : ""}`, item.text);
    case "context":
      return h("div", "m-item m-hidden");
  }
}

function blockNode(block: Block): HTMLElement {
  if (block.kind === "text") return template("div", "m-md", md(block.text));
  if (block.kind === "thinking") {
    const details = template("details", "m-thinking", `<summary></summary>`);
    details.querySelector("summary")!.textContent = t("mobile.thinking");
    details.append(h("div", "", block.text));
    return details;
  }
  const tool = h("div", `m-tool${block.error ? " err" : ""}${block.done ? "" : " busy"}`);
  tool.append(h("b", "", toolLabel(block.name)), h("span", "", summary(block.name, block.input, block.json)));
  return tool;
}
