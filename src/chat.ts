import * as actions from "./actions";
import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { capabilitiesOf } from "./agents";
import { encodeBrowserContext, type BrowserContext } from "./browser-context";
import type { ConversationCommandV1, RequestResponse } from "./conversation";
import { icon } from "./icons";
import { fromBack, t, tn } from "./i18n";
import { diffHtml, isDiff } from "./highlight";
import { kilo } from "./context";
import {
  capError,
  capLines,
  browserContextChip,
  contextPanel,
  countTools,
  errorPeek,
  inputView,
  peek,
  renderBrowserMessage,
  tallyText,
  took,
  toolIcon,
  toolLabel,
  wantsCard,
} from "./chat-presentation";
import { effortStep, fitsEffort, modelGroups, modelLabel, nextEffort } from "./launcher";
import { md } from "./markdown";
import * as mcp from "./mcp";
import * as menu from "./menu";
import * as plugins from "./plugins";
import * as skills from "./skills";
import * as trust from "./trust";
import * as commands from "./commands";
import * as notes from "./notes";
import * as paths from "./paths";
import { pasteFiles } from "./paste";
import * as team from "./team";
import { pieces, summary, Timeline, touched, type Ask, type Block, type Command, type Item, type Piece, type ToolBlock } from "./timeline";
import type { Choice, ProviderId, Selection, Status } from "./types";
import { button } from "./ui";
import { h, template } from "./util";

/// Render Timeline updates and composer interaction from canonical conversation events. Send prompts, responses, and interrupts through ConversationCommandV1. Remote conversations reuse this view with relay transport. Side-panel comments anchor to stable Piece.key values without entering the transcript.

/// Tab metadata needed by presentation but absent from conversation events.
export type Info = {
  task?: actions.TaskRun | null;
  /// Frontend workspace ID; remote IDs are prefixed.
  workspace: string | null;
  status: Status | null;
  /// A prompt waiting for worktree setup.
  pending: string | null;
  /// Remote owner name and online state.
  remote: { name: string; online: boolean } | null;
  /// Agent working directory and default file-picker location; internal attachments use relative paths.
  worktree: string | null;
  /// Comments require an active local or remote share, not merely configured team membership.
  team: boolean;
  /// Owner companion devices may view and control this workspace.
  remoteControl: boolean;
  /// Resolved tab or workspace model/effort; empty values mean CLI defaults.
  agent: ProviderId;
  model: string;
  effort: string;
  /// Workspace MCP layer; null inherits the global and project layers.
  mcp: Selection | null;
  /// Workspace plugin layer follows the same inheritance rules.
  plugins: Selection | null;
  /// Workspace standalone-skill layer, its own axis since ADR 0043.
  skills: Selection | null;
};

export type Ctx = {
  say: (text: string, isError?: boolean) => void;
  info: () => Info;
  comment?: (target: notes.Target) => void;
  thread?: (id: string) => void;
};

/* Conversation-owned transient state. */

/// Share drafts and attachments across ChatView instances so desk-to-workspace navigation preserves input.
const drafts = {
  says: new Map<string, string>(),
  files: new Map<string, string[]>(),
  contexts: new Map<string, BrowserContext[]>(),
  pending: new Map<string, number>(),
};
const draftListeners = new Set<(key: string) => void>();
const draftChanged = (key: string) => draftListeners.forEach(listener => listener(key));

export class ChatView {
  private feed!: HTMLElement;
  private box!: HTMLElement;
  private area!: HTMLTextAreaElement;
  private ctx!: Ctx;
  private key: string | null = null;
  private remote = false;
  /// Use attachment versions to distinguish stale snapshots even after leaving and returning to the same key.
  private attachVersion = 0;
  private disposed = false;
  private cleanup: (() => void)[] = [];
  private tl = new Timeline();
  /// Rendered pieces and their DOM nodes in display order.
  private shown: Piece[] = [];
  private drawn: HTMLElement[] = [];
  /// Persist expanded work cards across redraws and tab navigation.
  private opened = new Set<string>();
  /// Buffer partial JSON lines from chunked remote bytes.
  private partial = "";
  private decoder = new TextDecoder("utf-8");
  private feedback: string | null = null;
  /// Coalesce per-token updates into one animation frame to avoid excessive rendering.
  private dirty = new Set<number>();
  private raf = 0;
  private working = template("div", "working", "<i></i><i></i><i></i><span class=\"wlabel\"></span>");
  /// Hold live events during snapshot loading to remove overlap by sequence number.
  private held: { seq: number; line: string }[] | null = null;
  /// Display queued setup prompts with their waiting state.
  private waiting = template("div", "turn user wait", `<div class="bubble"></div><div class="working"><i></i><i></i><i></i><span class="wlabel"></span></div>`);

  open(host: HTMLElement, ctx: Ctx) {
    this.ctx = ctx;
    this.disposed = false;
    this.feed = h("div", "feed");
    this.box = h("div", "composer");
    host.append(this.feed, this.box);
    this.buildComposer();
    const draftChangedHere = (key: string) => {
      if (this.key !== key) return;
      if (this.area.value !== this.stashed()) this.restore();
      this.paintComposer();
    };
    draftListeners.add(draftChangedHere);
    this.cleanup.push(() => draftListeners.delete(draftChangedHere));

    void listen<[string, string, number]>("chat", ({ payload: [session, line, seq] }) => {
      if (session !== this.key || this.remote) return;
      if (this.held) this.held.push({ seq, line });
      else this.absorb(line);
    }).then((unlisten) => {
      // The tab may disappear before IPC listener registration completes.
      if (this.disposed) unlisten();
      else this.cleanup.push(unlisten);
    });
    // Comment changes update only transcript markers.
    const teamChanged = () => {
      if (this.key) this.paintCommentPins();
      this.paintComposer();
    };
    this.cleanup.push(team.onChange(teamChanged));
    const selectionChanged = () => this.paintQuoteButton();
    document.addEventListener("selectionchange", selectionChanged);
    this.cleanup.push(() => document.removeEventListener("selectionchange", selectionChanged));
  }

  /* Attachment lifecycle. */

  /// Attach a local transcript snapshot, followed by live events.
  async attach(key: string) {
    if (this.disposed) return;
    const version = ++this.attachVersion;
    this.stash();
    this.key = key;
    this.remote = false;
    this.reset();
    this.restore();
    this.held = [];
    const snapshot = await invoke("chat_snapshot", { session: key });
    if (this.disposed || version !== this.attachVersion || this.key !== key) return;
    const held = this.held ?? [];
    this.held = null;
    this.tl.load(snapshot.text);
    // Discard held events already included in the snapshot sequence, including initial prompts emitted while the view opens.
    for (const { seq, line } of held) if (seq > snapshot.seq) this.tl.push(line);
    this.renderAll();
  }

  /// Attach a remote snapshot; subsequent bytes arrive through remoteWrite.
  attachRemote(key: string, bytes: Uint8Array) {
    if (this.disposed) return;
    this.attachVersion++;
    this.stash();
    this.key = key;
    this.remote = true;
    this.reset();
    this.restore();
    this.tl.load(new TextDecoder("utf-8").decode(bytes));
    this.renderAll();
  }

  /// Ignore live bytes for other tabs; their cached mirrors restore them when selected.
  remoteWrite(key: string, bytes: Uint8Array) {
    if (key !== this.key || !this.remote) return;
    this.partial += this.decoder.decode(bytes, { stream: true });
    const lines = this.partial.split("\n");
    this.partial = lines.pop() ?? "";
    for (const line of lines) if (line.trim()) this.absorb(line);
  }

  detach() {
    this.attachVersion++;
    this.stash();
    this.key = null;
    this.remote = false;
    this.reset();
    this.restore();
    this.paintComposer();
  }

  /// Detach temporarily hidden views; dispose permanently removed views and release their listeners.
  dispose(forget = false) {
    if (this.disposed) return;
    const key = this.key;
    this.detach();
    // Archiving or cleaning hides desk panels, while closing tabs also discards their drafts. The collection owner chooses that behavior.
    if (forget && key) {
      drafts.says.delete(key);
      drafts.files.delete(key);
      drafts.contexts.delete(key);
    }
    this.disposed = true;
    for (const stop of this.cleanup.splice(0)) stop();
  }

  private reset() {
    this.tl = new Timeline();
    this.held = null;
    this.shown = [];
    this.drawn = [];
    this.partial = "";
    this.decoder = new TextDecoder("utf-8");
    this.feedback = null;
    this.dirty.clear();
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.feed.replaceChildren();
  }

  current() {
    return this.key;
  }

  focus() {
    this.area.focus();
  }

  /// Refresh composer metadata and queued prompts after external board changes.
  refresh() {
    const stick = this.stuck();
    this.paintWorking();
    this.paintComposer();
    if (stick) this.feed.scrollTop = this.feed.scrollHeight;
  }

  /// Read selected transcript text for a comment quotation.
  selection(): string {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.anchorNode || !this.feed.contains(sel.anchorNode)) return "";
    return sel.toString();
  }

  /// Only local conversations can attach files from this Mac.
  canAttachFiles(): boolean {
    const info = this.ctx.info();
    return (
      !!this.key &&
      !this.remote &&
      !!info.workspace &&
      capabilitiesOf(info.agent).attachments
    );
  }

  /// File-picker and drop attachments update the shared draft without changing typed text.
  attachFiles(paths: string[]): boolean {
    const target = this.fileDropTarget();
    if (!target) return false;
    target.put(paths);
    return true;
  }

  /** Keep selected elements separate from the person's editable text. */
  contextTarget(): ((context: BrowserContext) => void) | null {
    if (!this.canAttachFiles() || !this.key) return null;
    const key = this.key;
    return context => {
      drafts.contexts.set(key, [...(drafts.contexts.get(key) ?? []), context]);
      draftChanged(key);
      if (!this.disposed && this.key === key) this.area.focus();
    };
  }

  /// Delayed captures retain the draft chosen at drop time, even if this view has since unmounted.
  fileDropTarget(): { put: (paths: string[]) => void; wait: () => () => void } | null {
    if (!this.canAttachFiles() || !this.key) return null;
    const key = this.key;
    return {
      put: (paths) => {
        const files = (drafts.files.get(key) ?? []).slice();
        for (const path of paths) if (path && !files.includes(path)) files.push(path);
        drafts.files.set(key, files);
        draftChanged(key);
        if (!this.disposed && this.key === key && this.box.getClientRects().length) this.area.focus();
      },
      wait: () => {
        drafts.pending.set(key, (drafts.pending.get(key) ?? 0) + 1);
        draftChanged(key);
        return () => {
          const remaining = (drafts.pending.get(key) ?? 1) - 1;
          if (remaining) drafts.pending.set(key, remaining);
          else drafts.pending.delete(key);
          draftChanged(key);
        };
      },
    };
  }

  /* Incoming lines. */

  private absorb(line: string) {
    for (const i of this.tl.push(line)) this.dirty.add(i);
    if (!this.raf) this.raf = requestAnimationFrame(() => this.flush());
  }

  /// Apply accumulated changes once per frame. Follow output only when the user was already at the bottom.
  private flush() {
    this.raf = 0;
    const stick = this.stuck();
    const changed = this.dirty.size > 0;
    this.sync(this.dirty);
    this.dirty.clear();
    if (changed) this.paintCommentPins();
    this.paintWorking();
    this.paintComposer();
    if (stick) this.feed.scrollTop = this.feed.scrollHeight;
  }

  /// Show activity dots between streamed content, with labels for compaction or background work that may outlive the turn.
  private paintWorking() {
    this.paintWaiting();
    const last = this.tl.items[this.tl.items.length - 1];
    const typing = last?.kind === "assistant" && last.streaming && last.blocks[last.blocks.length - 1]?.kind === "text";
    const tasks = [...this.tl.tasks.values()];
    const show = this.tl.compacting || tasks.length > 0 || (this.tl.busy && !typing && !this.tl.pending.length);
    if (!show) {
      this.working.remove();
      return;
    }
    const label = this.tl.compacting
      ? t("chat.compacting")
      : tasks.length
        ? `${tn(tasks.length, "chat.bg")}: ${tasks.map((k) => k.description || "…").join(" · ")}`
        : "";
    this.working.querySelector(".wlabel")!.textContent = label;
    this.feed.append(this.working);
  }

  /// Display the initial prompt while setup runs so waiting does not look like lost input.
  private paintWaiting() {
    const info = this.ctx.info();
    const text = !this.remote && this.key ? info.pending : null;
    if (!text) {
      this.waiting.remove();
      return;
    }
    renderBrowserMessage(this.waiting.querySelector<HTMLElement>(".bubble")!, text);
    this.waiting.querySelector(".wlabel")!.textContent = t("chat.waiting");
    this.feed.querySelector(".nohint")?.remove();
    this.feed.append(this.waiting);
  }

  private stuck() {
    return this.feed.scrollTop + this.feed.clientHeight >= this.feed.scrollHeight - 48;
  }

  private renderAll() {
    this.shown = [];
    this.drawn = [];
    this.feed.replaceChildren();
    if (!this.tl.items.length) this.feed.append(h("div", "nohint", t("chat.empty")));
    this.sync(null);
    this.paintCommentPins();
    this.paintWorking();
    this.paintComposer();
    this.feed.scrollTop = this.feed.scrollHeight;
  }

  /// Update pieces touching changed items, append new pieces, and preserve other nodes, selection, and expansion. Null dirty state requests a full update.
  private sync(dirty: Set<number> | null) {
    const next = pieces(this.tl.items);
    // Find the unchanged prefix; append-only timelines usually retain all existing pieces.
    let same = 0;
    while (same < next.length && same < this.shown.length && next[same].key === this.shown[same].key) same++;
    for (const gone of this.drawn.splice(same)) gone.remove();
    for (let i = 0; i < same; i++) if (this.touches(next[i], dirty)) this.draw(i, next[i]);
    for (let i = same; i < next.length; i++) this.draw(i, next[i]);
    this.shown = next;
  }

  private touches(piece: Piece, dirty: Set<number> | null): boolean {
    if (!dirty) return true;
    return piece.kind === "work" ? piece.refs.some((r) => dirty.has(r.at)) : dirty.has(piece.at);
  }

  private draw(i: number, piece: Piece) {
    const old = this.drawn[i];
    // Update streaming nodes in place to preserve selection and avoid visual jitter.
    if (old && this.repaint(old, piece)) return;
    const node = this.node(piece);
    node.dataset.key = piece.key;
    if (old) {
      // Keep expanded tool cards open after redraw.
      for (const open of old.querySelectorAll<HTMLElement>(".tool.open")) {
        const id = open.dataset.tool;
        node.querySelector<HTMLElement>(`.tool[data-tool="${CSS.escape(id ?? "")}"]`)?.classList.add("open");
      }
      old.replaceWith(node);
    } else {
      this.feed.querySelector(".nohint")?.remove();
      this.feed.append(node);
    }
    this.drawn[i] = node;
  }

  private node(piece: Piece): HTMLElement {
    if (piece.kind === "item") {
      const item = this.tl.items[piece.at];
      // Assistant messages produce say/work pieces rather than item pieces.
      return item.kind === "assistant" ? h("div", "turn bot") : this.render(item, piece.at);
    }
    if (piece.kind === "say") {
      const el = h("div", "turn bot");
      const at = this.blockAt(piece);
      if (at) el.append(this.block(at.block, at.live));
      this.paintMeta(el, piece);
      return el;
    }
    return this.workCard(piece);
  }

  /// Show duration and copy controls only beneath the final speech of a completed turn.
  private paintMeta(el: HTMLElement, piece: Extract<Piece, { kind: "say" }>) {
    const ms = this.turnMs(piece);
    const old = el.querySelector(".meta");
    if (ms === null) return void old?.remove();
    // Durations below a tenth of a second lack meaningful historical timing; show only copy.
    const label = ms < 100 ? "" : took(ms);
    if (old) {
      old.querySelector(".took")!.textContent = label;
      this.paintCommentAction(old, piece);
      return;
    }
    const meta = template("div", "meta", `<span class="took"></span><button class="ico sm cp"></button><button class="ghost sm cm"></button>`);
    meta.querySelector(".took")!.textContent = label;
    const cp = meta.querySelector<HTMLElement>(".cp")!;
    cp.innerHTML = icon("copy", 13);
    cp.title = t("chat.copy");
    cp.addEventListener("click", () => {
      const at = this.blockAt(piece);
      if (at?.block.kind !== "text") return;
      void navigator.clipboard.writeText(at.block.text);
      cp.innerHTML = icon("check", 13);
      setTimeout(() => (cp.innerHTML = icon("copy", 13)), 1200);
    });
    this.paintCommentAction(meta, piece);
    el.append(meta);
  }

  private paintCommentAction(meta: Element, piece: Extract<Piece, { kind: "say" }>) {
    const button = meta.querySelector<HTMLButtonElement>(".cm");
    if (!button) return;
    button.hidden = !this.ctx.comment || !this.ctx.info().team;
    if (button.hidden) return;
    button.innerHTML = `${icon("message-square", 12)}<span></span>`;
    button.querySelector("span")!.textContent = t("notes.comment");
    button.title = t("notes.comment.turn");
    button.onclick = () => {
      const at = this.blockAt(piece);
      const quote = at?.block.kind === "text" ? at.block.text : null;
      if (this.key) this.ctx.comment?.({ tab: this.key, anchor: piece.key, quote });
    };
  }

  /// Find duration only for the final completed assistant block before control returns to the user.
  private turnMs(piece: Extract<Piece, { kind: "say" }>): number | null {
    const item = this.tl.items[piece.at];
    if (item?.kind !== "assistant" || item.streaming) return null;
    if (piece.block !== item.blocks.length - 1) return null;
    for (let i = piece.at + 1; i < this.tl.items.length; i++) {
      const next = this.tl.items[i];
      if (next.kind === "assistant") return null;
      if (next.kind === "user") break;
    }
    for (let i = piece.at - 1; i >= 0; i--) {
      const before = this.tl.items[i];
      if (before.kind === "user") return Math.max(0, item.ts - before.ts);
    }
    return null;
  }

  /// Only the final block of a streaming message is still receiving content.
  private blockAt(ref: { at: number; block: number }): { block: Block; live: boolean } | null {
    const item = this.tl.items[ref.at];
    if (item?.kind !== "assistant") return null;
    const block = item.blocks[ref.block];
    if (!block) return null;
    return { block, live: item.streaming && ref.block === item.blocks.length - 1 };
  }

  /// Render non-assistant items such as prompts, requests, turn endings, and system notices.
  private render(item: Exclude<Item, { kind: "assistant" }>, i: number): HTMLElement {
    switch (item.kind) {
      case "user": {
        const el = template("div", "turn user", `<div class="bubble"></div>`);
        renderBrowserMessage(el.firstElementChild as HTMLElement, item.text);
        return el;
      }
      case "ask":
        return this.askCard(item, i);
      case "result": {
        return this.errorCard(item.text || t("chat.result.error"));
      }
      case "context":
        return contextPanel(item.report);
      case "system": {
        if (item.what === "summary") {
          // Compaction summaries belong to the agent and collapse like reasoning content.
          const el = template("details", "think summary", `<summary></summary><div class="md"></div>`);
          el.querySelector("summary")!.textContent = t("chat.summary");
          (el.lastElementChild as HTMLElement).innerHTML = md(item.text);
          return el;
        }
        if (item.error) return this.errorCard(item.text);
        const el = h("div", "sys");
        el.textContent =
          item.what === "compacted"
            ? item.tokens
              ? t("chat.compacted.tokens", { pre: kilo(item.tokens[0]), post: kilo(item.tokens[1]) })
              : t("chat.compacted")
            : item.text;
        return el;
      }
    }
  }

  /// Present technical failures concisely while retaining full diagnostic output.
  private errorCard(text: string): HTMLElement {
    const value = text.trim() || t("chat.result.error");
    if (!value.includes("\n") && value.length <= 180) {
      const el = h("div", "sys err");
      el.textContent = value;
      return el;
    }
    const el = template(
      "details",
      "syserr",
      `<summary><span class="eic">${icon("x", 12)}</span><b></b><span class="prev"></span></summary><pre></pre>`,
    );
    el.querySelector("b")!.textContent = t("chat.error.title");
    el.querySelector(".prev")!.textContent = errorPeek(value);
    el.querySelector("pre")!.textContent = capError(value);
    return el;
  }

  /// Update compatible blocks in place and append new ones; return false when the whole piece must be replaced.
  private repaint(el: HTMLElement, piece: Piece): boolean {
    if (el.dataset.key !== piece.key) return false;
    // Rebuilding user items and notices is cheap; request cards retain their selected answers.
    if (piece.kind === "item") return false;
    if (piece.kind === "say") {
      const at = this.blockAt(piece);
      const node = el.firstElementChild as HTMLElement | null;
      if (!at || at.block.kind !== "text" || node?.dataset.kind !== "text") return false;
      node.innerHTML = md(at.block.text);
      node.classList.toggle("typing", at.live);
      // Add duration/copy controls when an already-rendered turn completes.
      this.paintMeta(el, piece);
      return true;
    }
    // A reasoning-only piece becomes a work card when its first tool arrives, requiring another node type.
    const parts = piece.refs.map((r) => this.blockAt(r)).filter((p) => !!p);
    const card = el.classList.contains("work");
    if (wantsCard(parts) !== card) return false;
    const body = card ? el.querySelector<HTMLElement>(".wbody") : el;
    if (!body) return false;
    // Refresh the collapsed header when its underlying blocks change.
    if (card) this.paintWorkHead(el, piece);
    return this.patch(body, parts);
  }

  /// Update blocks within the existing piece node.
  private patch(el: HTMLElement, blocks: ({ block: Block; live: boolean } | null)[]): boolean {
    for (let k = 0; k < blocks.length; k++) {
      const at = blocks[k];
      if (!at) continue;
      const node = el.children[k] as HTMLElement | undefined;
      if (!node) {
        el.append(this.block(at.block, at.live));
        continue;
      }
      // If a block unexpectedly changes type, rebuild instead of displaying stale structure.
      if (node.dataset.kind !== at.block.kind) return false;
      if (at.block.kind === "text") {
        node.innerHTML = md(at.block.text);
        node.classList.toggle("typing", at.live);
      } else if (at.block.kind === "thinking") {
        node.querySelector("b")!.textContent = t(at.live ? "chat.thinking" : "chat.thought");
        node.querySelector(".prev")!.textContent = peek(at.block.text);
        (node.lastElementChild as HTMLElement).textContent = at.block.text;
        node.classList.toggle("live", at.live);
        node.classList.toggle("bare", !at.block.text);
      } else {
        // Rebuild changed tool states while preserving expansion.
        const fresh = this.block(at.block, at.live);
        if (node.classList.contains("open")) fresh.classList.add("open");
        node.replaceWith(fresh);
      }
    }
    return true;
  }

  /// Group consecutive agent work into one expandable card so intermediate steps do not bury readable speech.
  private workCard(piece: Extract<Piece, { kind: "work" }>): HTMLElement {
    const parts = piece.refs.map((r) => this.blockAt(r)).filter((p) => !!p);
    if (!wantsCard(parts)) {
      const el = h("div", "turn bot");
      for (const p of parts) el.append(this.block(p.block, p.live));
      return el;
    }
    const el = h("div", "work" + (this.opened.has(piece.key) ? " open" : ""));
    const head = template("button", "whead", `<span class="wic"></span><b></b><span class="sum"></span><span class="st"></span>`);
    head.addEventListener("click", () => {
      const open = el.classList.toggle("open");
      if (open) this.opened.add(piece.key);
      else this.opened.delete(piece.key);
    });
    const body = h("div", "wbody");
    for (const p of parts) body.append(this.block(p.block, p.live));
    el.append(head, body);
    this.paintWorkHead(el, piece);
    return el;
  }

  /// While running, the card header shows current activity; after completion, summarize step counts and affected work.
  private paintWorkHead(el: HTMLElement, piece: Extract<Piece, { kind: "work" }>) {
    const parts = piece.refs.map((r) => this.blockAt(r)).filter((p) => !!p);
    const tools = parts.map((p) => p.block).filter((b): b is ToolBlock => b.kind === "tool");
    const last = parts[parts.length - 1];
    const running = parts.some((p) => p.live) || tools.some((b) => !b.done || b.background);
    const bad = tools.some((b) => b.error);
    el.classList.toggle("going", running);
    el.classList.toggle("bad", !running && bad);
    el.classList.toggle("ok", !running && !bad);

    const now = running && last.block.kind === "tool" ? last.block : null;
    const tally = countTools(tools);
    const name = now ? now.name : tally[0]?.[0] ?? "";
    const q = (sel: string) => el.querySelector(sel)!;
    q(".wic").innerHTML = icon(running && !now ? "sparkles" : toolIcon(name), 14);
    q("b").textContent = now ? toolLabel(now.name) : running ? t("chat.thinking") : tn(tools.length, "chat.work");
    q(".sum").textContent = now ? summary(now.name, now.input, now.json) : running ? "" : tallyText(tally);
    q(".st").innerHTML = running ? `<span class="spin"></span>` : icon(bad ? "x" : "check", 12);
  }

  /// The final streaming block distinguishes ongoing reasoning from completed reasoning.
  private block(block: Block, live: boolean): HTMLElement {
    if (block.kind === "text") {
      const el = h("div", "md" + (live ? " typing" : ""));
      el.dataset.kind = "text";
      el.innerHTML = md(block.text);
      return el;
    }
    if (block.kind === "thinking") {
      // Without retained reasoning text, show only its label and no expansion control.
      const el = template(
        "details",
        "think" + (live ? " live" : "") + (block.text ? "" : " bare"),
        `<summary><span class="tic">${icon("brain", 14)}</span><b></b><span class="prev"></span></summary><div></div>`,
      );
      el.dataset.kind = "thinking";
      el.querySelector("b")!.textContent = t(live ? "chat.thinking" : "chat.thought");
      el.querySelector(".prev")!.textContent = peek(block.text);
      (el.lastElementChild as HTMLElement).textContent = block.text;
      return el;
    }
    // Collapsed tool rows reveal input and output when opened.
    const running = !block.done || block.background;
    const el = h("div", "tool" + (running ? " run" : block.error ? " bad" : " ok"));
    el.dataset.kind = "tool";
    el.dataset.tool = block.id;
    const head = template("button", "thead", `<span class="tic">${icon(toolIcon(block.name), 14)}</span><b></b><span class="sum"></span><span class="bgtag"></span><span class="st"></span>`);
    head.querySelector("b")!.textContent = toolLabel(block.name);
    head.querySelector(".sum")!.textContent = block.name === "ExitPlanMode" ? "" : summary(block.name, block.input, block.json);
    head.querySelector(".bgtag")!.textContent = block.background ? t("chat.bg.tag") : "";
    head.querySelector(".st")!.innerHTML = running ? `<span class="spin"></span>` : icon(block.error ? "x" : "check", 12);
    head.addEventListener("click", () => el.classList.toggle("open"));
    el.append(head);
    const body = h("div", "tbody");
    if (block.name === "ExitPlanMode") {
      // Render plans directly as readable markdown.
      el.classList.add("open", "plan");
      const plan = h("div", "md");
      plan.innerHTML = md(String((block.input as { plan?: string })?.plan ?? ""));
      body.append(plan);
    } else if (block.error) {
      const failed = template("div", "tfail", `<span class="tic">${icon("x", 12)}</span><span></span>`);
      failed.lastElementChild!.textContent = t("chat.tool.failed");
      body.append(failed);

      const technical = h("div", "ttech");
      const input = inputView(block.name, block.input);
      if (input.childElementCount) technical.append(input);
      if (block.result !== null) {
        const out = h("pre", "tout");
        out.textContent = capError(block.result);
        technical.append(out);
      }
      if (technical.childElementCount) {
        const details = template("details", "ttechnical", `<summary></summary>`);
        details.querySelector("summary")!.textContent = t("chat.tool.details");
        details.append(technical);
        body.append(details);
      }
    } else if (block.name === "Skill" && block.result) {
      // Collapse skill instructions while preserving readable markdown on expansion.
      const what = h("div", "md");
      what.innerHTML = md(capLines(block.result));
      body.append(what);
    } else {
      body.append(inputView(block.name, block.input));
      if (block.result !== null) {
        const out = h("pre", "tout");
        // Highlight unified diffs returned by tools like other edit diffs.
        if (isDiff(block.result)) {
          out.classList.add("tdiff");
          out.innerHTML = diffHtml(capLines(block.result));
        } else out.textContent = capLines(block.result);
        body.append(out);
      }
    }
    el.append(body);
    return el;
  }

  /* Response requests. */

  private askCard(ask: Ask, i: number): HTMLElement {
    if (ask.answered) {
      const el = template("div", "sys done", `${icon("check", 12)}<span></span>`);
      el.querySelector("span")!.textContent = t("chat.answered", { what: toolLabel(ask.tool) });
      return el;
    }
    const el = h("div", "ask");
    if (ask.tool === "ExitPlanMode") return this.planCard(el, ask);
    if (ask.tool === "AskUserQuestion") return this.questionCard(el, ask);
    return this.permCard(el, ask, i);
  }

  private planCard(el: HTMLElement, ask: Ask): HTMLElement {
    el.classList.add("plan");
    el.append(h("h4", "", t("chat.plan.title")));
    const row = h("div", "row");
    const go = h("button", "pri md", t("chat.plan.go"));
    go.title = t("chat.plan.go.title");
    go.addEventListener("click", () => {
      // Approving a plan also enables bypass before resuming so its first tool does not immediately ask again.
      this.control({
        v: 1,
        type: "permission.mode.set",
        mode: "bypass",
      });
      this.respond(ask, { outcome: "allow" });
    });
    const asking = h("button", "outline md", t("chat.plan.ask"));
    asking.title = t("chat.plan.ask.title");
    asking.addEventListener("click", () => this.respond(ask, { outcome: "allow" }));
    const no = h("button", "ghost md", t("chat.plan.no"));
    row.append(go, asking, no);
    el.append(row);
    // Send requested plan changes as denial feedback to the agent.
    const fb = template("div", "fb", `<textarea rows="3"></textarea><div class="row"><span class="spacer"></span><button class="pri md"></button></div>`);
    const area = fb.querySelector("textarea")!;
    area.placeholder = t("chat.plan.feedback");
    fb.querySelector("button")!.textContent = t("chat.plan.send");
    fb.hidden = this.feedback !== ask.id;
    no.addEventListener("click", () => {
      this.feedback = ask.id;
      fb.hidden = false;
      area.focus();
    });
    const send = () => {
      const text = area.value.trim();
      if (!text) return;
      this.feedback = null;
      this.respond(ask, { outcome: "deny", message: text });
    };
    fb.querySelector("button")!.addEventListener("click", send);
    area.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        send();
      }
    });
    el.append(fb);
    return el;
  }

  /// Show one question per tab, advance to unanswered questions, and submit all answers together.
  private questionCard(el: HTMLElement, ask: Ask): HTMLElement {
    el.classList.add("question");
    type Q = { question: string; header?: string; multiSelect?: boolean; options: { label: string; description?: string }[] };
    const questions = (ask.input.questions as Q[]) ?? [];
    const answers: Record<string, string[]> = {};
    const other: Record<string, string> = {};
    let active = 0;
    const has = (q: Q) => (answers[q.question]?.length ?? 0) > 0 || !!other[q.question]?.trim();
    // Advance to the next unanswered question, wrap if needed, or remain when all are answered.
    const next = () => {
      const after = questions.findIndex((q, i) => i > active && !has(q));
      const any = questions.findIndex((q) => !has(q));
      active = after !== -1 ? after : any !== -1 ? any : active;
    };
    const tabs = h("div", "qtabs");
    const body = h("div", "qbody");
    const row = h("div", "row");
    const go = h("button", "pri md", t("chat.ask.go")) as HTMLButtonElement;
    go.addEventListener("click", () => {
      const out: Record<string, string> = {};
      for (const q of questions) {
        const typed = other[q.question]?.trim();
        const picked = answers[q.question] ?? [];
        out[q.question] = [...picked, ...(typed ? [typed] : [])].join(", ");
      }
      this.respond(ask, { outcome: "answer", answers: out });
    });
    row.append(h("span", "spacer"), go);

    const paint = () => {
      tabs.replaceChildren(
        ...questions.map((q, i) => {
          const b = template("button", "qtab" + (i === active ? " on" : "") + (has(q) ? " done" : ""), `<span></span>${icon("check", 11)}`);
          b.querySelector("span")!.textContent = q.header || t("chat.ask.n", { n: i + 1 });
          b.addEventListener("click", () => {
            active = i;
            paint();
          });
          return b;
        }),
      );
      tabs.hidden = questions.length < 2;
      const q = questions[active];
      body.replaceChildren();
      if (!q) return;
      body.append(h("p", "", q.question));
      const opts = h("div", "opts");
      for (const o of q.options ?? []) {
        const on = (answers[q.question] ?? []).includes(o.label);
        const b = template("button", "opt" + (on ? " on" : ""), `<b></b><span></span>`);
        b.querySelector("b")!.textContent = o.label;
        b.querySelector("span")!.textContent = o.description ?? "";
        b.addEventListener("click", () => {
          const list = answers[q.question] ?? [];
          if (q.multiSelect) {
            answers[q.question] = on ? list.filter((x) => x !== o.label) : [...list, o.label];
          } else {
            answers[q.question] = [o.label];
            next();
          }
          paint();
        });
        opts.append(b);
      }
      body.append(opts);
      const free = document.createElement("input");
      free.className = "field";
      free.placeholder = t("chat.ask.other");
      free.value = other[q.question] ?? "";
      free.addEventListener("input", () => {
        other[q.question] = free.value;
        paintGo();
        paintTabs();
      });
      free.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && free.value.trim()) {
          e.preventDefault();
          if (questions.every(has)) go.click();
          else {
            next();
            paint();
          }
        }
      });
      body.append(free);
      paintGo();
    };
    const paintGo = () => void (go.disabled = !questions.every(has));
    const paintTabs = () => {
      for (const [i, b] of [...tabs.children].entries()) b.classList.toggle("done", has(questions[i]));
    };
    el.append(tabs, body, row);
    paint();
    return el;
  }

  private permCard(el: HTMLElement, ask: Ask, _i: number): HTMLElement {
    el.append(h("h4", "", t("chat.perm.title", { tool: toolLabel(ask.tool) })));
    el.append(inputView(ask.tool, ask.input));
    const row = h("div", "row");
    const yes = h("button", "pri md", t("chat.perm.yes"));
    yes.addEventListener("click", () => this.respond(ask, { outcome: "allow" }));
    const always = h("button", "outline md", t("chat.perm.always"));
    always.addEventListener("click", () => {
      this.control({
        v: 1,
        type: "permission.mode.set",
        mode: "bypass",
      });
      this.respond(ask, { outcome: "allow" });
    });
    const no = h("button", "ghost md", t("chat.perm.no"));
    no.addEventListener("click", () => this.respond(ask, { outcome: "deny", message: t("chat.perm.denied") }));
    row.append(yes, always, no);
    el.append(row);
    return el;
  }

  private respond(ask: Ask, response: RequestResponse) {
    this.control({ v: 1, type: "request.respond", requestId: ask.id, response });
    this.sync(new Set(this.tl.answer(ask.id)));
    this.paintComposer();
  }

  /// Send canonical control locally or through the relay to the owner's Mac.
  private control(frame: ConversationCommandV1) {
    if (!this.key) return;
    if (this.remote) team.write(JSON.stringify(frame));
    else invoke("chat_control", { session: this.key, frame }).catch((e) => this.ctx.say(fromBack(e), true));
  }

  private interrupt() {
    this.control({ v: 1, type: "turn.interrupt" });
  }

  /* Composer. */

  private buildComposer() {
    this.box.innerHTML = `
      <!-- Keep attachments visible above the prompt, matching the launcher. -->
      <div class="cfiles" hidden></div>
      <textarea rows="1" spellcheck="true"></textarea>
      <div class="crow composer-meta">
        <!-- Model and effort changes restart the process on the next prompt while preserving the conversation. -->
        <span class="with" hidden>
          <button class="ghost mdl"></button>
          <button class="ghost effort"><span class="bars"><i></i><i></i><i></i><i></i><i></i></span><span class="el"></span></button>
        </span>
        <button class="ghost sm taskwatch" hidden></button>
        <span class="hint" role="status"></span>
      </div>
      <div class="crow composer-toolbar">
        <div class="composer-tools">
          <!-- Tool selection resumes the same transcript with updated MCP and plugin settings. -->
          <button class="ico sm addfile" hidden></button>
          <button class="ghost sm actionsbtn"></button>
          <button class="ghost sm mcpbtn" hidden><span></span></button>
          <button class="ghost sm plugbtn" hidden><span></span></button>
          <button class="ghost sm skillbtn" hidden><span></span></button>
        </div>
        <div class="composer-controls"></div>
      </div>
      <button class="outline md quotesel" hidden></button>`;
    this.area = this.box.querySelector("textarea")!;
    this.area.title = t("chat.input.hint");
    const q = (sel: string) => this.box.querySelector<HTMLElement>(sel)!;
    const remote = button(t("remoteControl.label"), () => {}, "ghost");
    remote.classList.add("remotebtn");
    remote.hidden = true;
    remote.innerHTML = `${icon("globe", 14)}<span></span>`;
    remote.setAttribute("aria-label", t("remoteControl.label"));
    const stop = button(t("chat.stop"), () => this.interrupt(), "ghost");
    stop.classList.add("stop");
    stop.hidden = true;
    stop.innerHTML = icon("square", 14);
    stop.setAttribute("aria-label", t("chat.stop"));
    stop.title = t("chat.stop.title");
    const send = button(t("chat.send"), () => this.send(), "pri");
    send.classList.add("send", "round");
    send.setAttribute("aria-label", t("chat.send"));
    q(".composer-controls").append(remote, stop, send);
    q(".addfile").innerHTML = icon("plus", 14);
    q(".addfile").title = t("chat.addFile");
    q(".addfile").setAttribute("aria-label", t("chat.addFile"));
    q(".quotesel").innerHTML = `${icon("message-square", 12)}<span></span>`;
    q(".quotesel span").textContent = t("notes.quoteSelection");
    q(".quotesel").title = t("notes.quoteSelection.title");
    q(".actionsbtn").innerHTML = `${icon("play", 13)}<span></span>`;
    q(".actionsbtn span").textContent = t("actions.title");
    q(".actionsbtn").title = t("actions.title");
    q(".actionsbtn").setAttribute("aria-label", t("actions.title"));
    q(".actionsbtn").addEventListener("click", () => this.actionMenu());
    this.cleanup.push(actions.onChange(() => this.paintComposer()));
    // The CLI-inherited base arrives asynchronously and can unhide the MCP button (ADR 0044).
    this.cleanup.push(mcp.onChange(() => this.paintComposer()));
    q(".addfile").addEventListener("click", () => void this.addFile());
    q(".quotesel").addEventListener("click", () => this.quoteSelection());

    this.area.addEventListener("input", () => {
      this.keep();
      this.grow();
      // Slash commands own completion at prompt start; otherwise @ completes workspace file paths.
      if (!commands.typed(this.area, this.commands(), () => { this.keep(); this.grow(); }, name => this.selectAction(name))) this.typedPath();
    });
    this.area.addEventListener("keydown", (e) => {
      const pick = e.key === "Enter" || e.key === "Tab";
      if (pick && !e.shiftKey && !e.isComposing && (commands.accept(e.key === "Tab") || paths.accept())) {
        e.preventDefault();
      } else if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        this.send();
      } else if (e.key === "Escape" && this.tl.busy && !this.area.value) {
        e.preventDefault();
        this.interrupt();
      }
    });
    // Pasted screenshots and copied files become attachments, like a drop on this conversation.
    this.area.addEventListener("paste", (e) => {
      const target = this.fileDropTarget();
      if (target) pasteFiles(e, target.put, (error) => this.ctx.say(fromBack(error), true));
    });
  }

  /// The file picker starts in the worktree but allows other local files. Show attachments as removable chips and convert them to mentions on send without altering typed text.
  private async addFile() {
    const root = this.ctx.info().worktree;
    const picked = await open({ multiple: true, title: t("chat.addFile.dialog"), defaultPath: root ?? undefined });
    const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
    this.attachFiles(list);
  }

  /// Attachments require a tab-owned draft.
  private attached(): string[] {
    return (this.key && drafts.files.get(this.key)) || [];
  }

  private contexts(): BrowserContext[] {
    return (this.key && drafts.contexts.get(this.key)) || [];
  }

  /// Offer path completion only locally because remote agents use another Mac's files.
  private typedPath() {
    const ws = this.ctx.info().workspace;
    if (this.remote || !ws) return paths.dismiss();
    void paths.typed(this.area, ws, touched(this.tl.items), () => { this.keep(); this.grow(); });
  }

  /// Persist the tab draft on every keystroke.
  private keep() {
    if (this.key) drafts.says.set(this.key, this.area.value);
  }

  /// Stash input before attaching another conversation.
  private stash() {
    this.keep();
  }

  private stashed(): string {
    return (this.key && drafts.says.get(this.key)) || "";
  }

  /// Closing a tab removes its unfinished prompt and attachments.
  forget(alive: Set<string>) {
    for (const key of drafts.says.keys()) if (!alive.has(key)) drafts.says.delete(key);
    for (const key of drafts.files.keys()) if (!alive.has(key)) drafts.files.delete(key);
    for (const key of drafts.contexts.keys()) if (!alive.has(key)) drafts.contexts.delete(key);
  }

  private grow() {
    const a = this.area;
    a.style.height = "0";
    a.style.height = `${Math.min(a.scrollHeight, window.innerHeight * 0.4)}px`;
  }

  /// Restore the selected tab's draft on attachment.
  private restore() {
    this.area.value = this.stashed();
    this.grow();
  }

  /// Open comment composition beside the transcript without replacing the agent composer.
  quoteSelection(): boolean {
    const sel = this.selection().trim();
    if (!this.key || !sel || !this.ctx.info().team || !this.ctx.comment) return false;
    const selection = window.getSelection();
    const node = selection?.anchorNode;
    const element = node instanceof Element ? node : node?.parentElement;
    const anchor = element?.closest<HTMLElement>("[data-key]")?.dataset.key ?? null;
    this.ctx.comment({ tab: this.key, anchor, quote: sel });
    return true;
  }

  private send() {
    if (this.key && drafts.pending.has(this.key)) return;
    const text = this.area.value.trim();
    if (this.selectAction()) return;
    const files = this.attached();
    const contexts = this.contexts();
    if ((!text && !files.length && !contexts.length) || !this.key) return;
    const info = this.ctx.info();
    if (info.remote && !info.remote.online) return this.ctx.say(t("err.team.offline"), true);
    // Prepend attachments as mentions, matching the launcher's initial prompt format.
    const said = [paths.mentions(files, info.worktree), ...contexts.map(encodeBrowserContext), text].filter(Boolean).join("\n\n");
    const key = this.key;
    if (this.remote) team.write(said);
    else invoke("chat_send", { session: key, text: said }).catch(e => this.ctx.say(fromBack(e), true));
    drafts.says.delete(key);
    drafts.files.delete(key);
    drafts.contexts.delete(key);
    this.area.value = "";
    commands.dismiss();
    paths.dismiss();
    draftChanged(key);
    this.grow();
    this.paintComposer();
  }

  /// Use commands discovered at process startup. Detached transcripts lack this metadata, so reuse the latest model-specific list or application-known defaults.
  private commands(): commands.Suggestion[] {
    const provider = this.providerCommands();
    if (this.remote) return provider;
    return [...actions.commandNames(actions.catalog().commands, provider).map(c => ({
      name: c.name, hint: "", badge: t("actions.origin"),
      description: [t(c.kind === "prompt" ? "actions.prompt" : "actions.agent"), c.description].filter(Boolean).join(" · "),
    })), ...provider];
  }

  private actionMenu() {
    if (this.remote || !this.ctx.info().workspace) return;
    const button = this.box.querySelector<HTMLElement>(".actionsbtn")!;
    const box = button.getBoundingClientRect();
    menu.openAt({ x: box.left, y: box.top - 4, above: true }, actions.catalog().commands.length
      ? actions.catalog().commands.map(action => ({ label: `/${action.name}`, hint: t(action.kind === "prompt" ? "actions.prompt" : "actions.agent"), run: () => this.useAction(action, this.area.value) }))
      : [{ label: t("actions.empty"), disabled: true }]);
  }

  private selectAction(name?: string): boolean {
    if (this.remote || !this.ctx.info().workspace) return false;
    const text = name ? `/${name} ${this.area.value.slice(this.area.selectionStart)}` : this.area.value;
    const found = actions.findCommand(text.trim(), actions.catalog().commands, this.providerCommands());
    if (!found) return false;
    this.useAction(found.action, found.rest);
    return true;
  }

  private startingAction = false;
  private useAction(action: actions.Action, rest: string) {
    commands.dismiss();
    if (action.kind === "prompt") {
      this.area.value = actions.expand(action.prompt, rest);
      this.keep(); this.grow(); this.area.focus();
      return;
    }
    const workspace = this.ctx.info().workspace;
    if (!workspace || this.startingAction) return;
    this.startingAction = true;
    const key = this.key;
    const draft = this.area.value;
    const contexts = this.contexts();
    const context = [actions.expand(paths.mentions(this.attached(), this.ctx.info().worktree), rest), ...contexts.map(encodeBrowserContext)].filter(Boolean).join("\n\n");
    // Preserve the draft if the backend rejects execution.
    void actions.start(workspace, action, context).then(() => {
      if (key && drafts.says.get(key) === draft) drafts.says.delete(key);
      if (key) drafts.files.delete(key);
      if (key) drafts.contexts.set(key, (drafts.contexts.get(key) ?? []).filter(context => !contexts.includes(context)));
      if (this.key === key) { this.area.value = ""; this.grow(); }
      if (key) draftChanged(key);
    }).catch(e => this.ctx.say(fromBack(e), true)).finally(() => { this.startingAction = false; });
  }

  private providerCommands(): Command[] {
    const info = this.ctx.info();
    const capabilities = capabilitiesOf(info.agent);
    const supported = (command: Command) =>
      (command.name !== "compact" || capabilities.compact) &&
      (command.name !== "context" || capabilities.contextReport);
    const key = `prometeu:comandos:${info.agent}:${info.model}`;
    const live = this.tl.commands;
    if (live.length) {
      localStorage.setItem(key, JSON.stringify(live));
      return live.filter(supported);
    }
    try {
      const seen: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
      if (Array.isArray(seen) && seen.length) {
        return seen
          .filter((c): c is Command => !!c && typeof c.name === "string" && typeof c.description === "string")
          .filter(supported);
      }
    } catch {
      /* Ignore unreadable cached command lists. */
    }
    return [
      ...(capabilities.compact ? [{ name: "compact", description: t("chat.cmd.compact"), hint: "" }] : []),
      ...(capabilities.contextReport ? [{ name: "context", description: t("chat.cmd.context"), hint: "" }] : []),
    ];
  }

  private paintComposer() {
    const info = this.ctx.info();
    const actionButton = this.box.querySelector<HTMLButtonElement>(".actionsbtn")!;
    actionButton.hidden = !!info.remote || !info.workspace;
    const watch = this.box.querySelector<HTMLButtonElement>(".taskwatch")!;
    const run = info.task;
    watch.hidden = !run;
    if (run) {
      watch.textContent = t(run.error ? "actions.attention" : run.done ? "actions.done" : run.paused ? "actions.paused" : info.status === "rodando" ? "actions.running" : run.profile.watch ? "actions.watching" : "actions.running");
      watch.title = run.error ? fromBack(run.error) : t(run.paused ? "actions.resume" : "actions.pause");
      watch.disabled = !!info.remote || run.done || !run.profile.watch;
      watch.onclick = () => {
        if (this.key) void invoke("action_pause", { session: this.key, paused: !run.paused }).catch(e => this.ctx.say(fromBack(e), true));
      };
    }

    const q = (sel: string) => this.box.querySelector<HTMLElement>(sel)!;
    const hasKey = !!this.key;
    this.box.hidden = !hasKey;
    if (!hasKey) return;
    // Local file attachment is unavailable for agents running on another Mac.
    q(".addfile").hidden =
      this.remote || !info.workspace || !capabilitiesOf(info.agent).attachments;
    q(".stop").hidden = !this.tl.busy;
    this.box.classList.toggle("busy", this.tl.busy);

    const off = info.remote ? !info.remote.online : false;
    this.area.disabled = off;
    this.area.placeholder = off
        ? t("chat.placeholder.remoteOff", { name: info.remote?.name ?? "" })
        : info.remote
          ? t("chat.placeholder.remote", { name: info.remote.name })
          : info.status === "desligada"
            ? t("chat.placeholder.off")
            : t("chat.placeholder");
    this.area.setAttribute("aria-label", this.area.placeholder);
    const receiving = !!this.key && drafts.pending.has(this.key);
    const hint = receiving ? t("chat.drop.receiving") : this.tl.compacting ? t("chat.compacting") : this.tl.busy ? t("chat.busy") : "";
    if (q(".hint").textContent !== hint) q(".hint").textContent = hint;
    this.paintWith(info);
    this.paintMcp(info);
    this.paintPlugins(info);
    this.paintSkills(info);
    this.paintRemoteControl(info);
    const send = q(".send");
    (send as HTMLButtonElement).disabled = receiving;
    send.title = t("chat.send");
    send.innerHTML = icon("arrow-up", 16);
    this.paintQuoteButton();
    this.paintFiles();
  }

  /// Show one removable chip per attachment with its full path in the tooltip.
  private paintFiles() {
    const row = this.box.querySelector<HTMLElement>(".cfiles")!;
    const list = this.attached();
    const contexts = this.contexts();
    row.hidden = !list.length && !contexts.length;
    row.replaceChildren(
      ...list.map((path, i) => {
        const chip = template("span", "injchip", `<span></span><button class="ico sm">${icon("x", 12)}</button>`);
        chip.children[0].textContent = path.split("/").pop() ?? path;
        chip.children[0].setAttribute("title", paths.short(path, this.ctx.info().worktree));
        chip.children[1].addEventListener("click", () => {
          const files = this.attached().slice();
          files.splice(i, 1);
          if (this.key) drafts.files.set(this.key, files);
          this.paintComposer();
        });
        return chip;
      }),
      ...contexts.map(context => browserContextChip(context, () => {
        if (!this.key) return;
        drafts.contexts.set(this.key, this.contexts().filter(item => item !== context));
        draftChanged(this.key);
        this.area.focus();
      })),
    );
  }

  /// Present resolved model and effort beneath the composer. Local idle tabs can change within their provider, persisting the choice and restarting on the next prompt. Remote views show labels only; changing providers requires another tab because resume identities differ.
  private paintWith(info: Info) {
    const el = this.box.querySelector<HTMLElement>(".with")!;
    const label = info.model ? modelLabel(info.model, info.agent) : "";
    el.hidden = !label;
    if (el.hidden) return;
    const working = info.status === "rodando" || info.status === "querendo";
    // No workspace or remote ownership means this view cannot change the process configuration.
    const fixed = !!info.remote || !info.workspace || !!info.task;
    el.classList.toggle("ro", fixed);
    el.title = fixed ? "" : working ? t("chat.with.busy") : t("chat.with.pick");

    const model = el.querySelector<HTMLButtonElement>(".mdl")!;
    model.innerHTML = `${icon("sparkles", 13)}<span></span>`;
    model.querySelector("span")!.textContent = label;
    model.title = [label, el.title].filter(Boolean).join("\n");
    model.disabled = fixed || working;
    model.onclick = () => this.pickModel(model, info);

    const step = effortStep(info.model, info.effort, info.agent);
    const bars = el.querySelector<HTMLButtonElement>(".effort")!;
    bars.hidden = !step;
    if (!step) return;
    bars.classList.toggle("ultra", info.effort === "ultracode");
    bars.querySelector<HTMLElement>(".el")!.textContent = step.label;
    bars.querySelectorAll(".bars i").forEach((bar, n) => bar.classList.toggle("lit", n <= step.step));
    bars.disabled = fixed || working;
    bars.onclick = () =>
      this.retune(info, {
        agent: info.agent,
        model: info.model,
        effort: nextEffort(info.model, info.effort, info.agent),
      });
  }

  /// Restrict model choices to the conversation's provider and clamp effort to the new model's ladder.
  private pickModel(at: HTMLElement, info: Info) {
    const box = at.getBoundingClientRect();
    const blocks = modelGroups(info.agent);
    const items: menu.Item[] = [];
    blocks.forEach((block, n) => {
      if (n) items.push("sep");
      if (block.head && blocks.length > 1) items.push({ label: block.head, disabled: true });
      for (const [id, name] of block.items) {
        items.push({
          label: name,
          checked: id === info.model,
          run: () =>
            this.retune(info, {
              agent: info.agent,
              model: id,
              effort: fitsEffort(id, info.effort, info.agent),
            }),
        });
      }
    });
    menu.openAt({ x: box.left, y: box.bottom + 4 }, items);
  }

  /// Persist changed tab choices and stop their process; identical choices require no restart.
  private retune(info: Info, choice: Choice) {
    if (choice.model === info.model && choice.effort === info.effort) return;
    if (!info.workspace || !this.key) return;
    void invoke("set_tab_choice", { id: info.workspace, tab: this.key, choice }).catch((e) =>
      this.ctx.say(fromBack(e), true),
    );
  }

  /// Tool selection applies at the next spawn or resume; a running session keeps the set it was born
  /// with (ADR 0043), so the picker is available even mid-turn and only states that the change lands on
  /// the next message. Hide controls without registry entries, a CLI-inherited base, or an existing
  /// layer (ADR 0044).
  private paintMcp(info: Info) {
    const btn = this.box.querySelector<HTMLButtonElement>(".mcpbtn")!;
    if (info.task) { btn.hidden = true; return; }
    if (info.workspace) mcp.loadInherited(info.workspace);
    const has =
      mcp.list().length > 0 ||
      info.mcp !== null ||
      (!!info.workspace && mcp.inheritedOf(info.workspace).length > 0);
    btn.hidden =
      !!info.remote ||
      !info.workspace ||
      !capabilitiesOf(info.agent).workspaceMcpSelection ||
      !has;
    if (btn.hidden) return;
    // Read the status when the picker opens, not when this paint ran, so a session that started
    // working in between still gets the "applies on the next message" notice.
    const working = () => {
      const status = this.ctx.info().status;
      return status === "rodando" || status === "querendo";
    };
    btn.innerHTML = `${icon("plug", 13)}<span></span>`;
    btn.querySelector("span")!.textContent = mcp.label(info.mcp);
    btn.classList.toggle("on", !!info.mcp);
    btn.title = `${t("mcp.title")}: ${mcp.label(info.mcp)}`;
    btn.setAttribute("aria-label", btn.title);
    btn.onclick = () => {
      const at = btn.getBoundingClientRect();
      const workspace = info.workspace!;
      void mcp.openPicker({
        workspace,
        current: () => this.ctx.info().mcp,
        set: (sel) =>
          invoke("set_workspace_mcp", { id: workspace, mcp: sel }).catch((e) =>
            this.ctx.say(fromBack(e), true),
          ),
        at: () => ({ x: at.left, y: at.bottom + 4 }),
        working,
        trust: () => void trust.open(workspace, this.ctx.say),
      });
    };
  }

  /// Plugin selection shares MCP's ownership and next-spawn application rule.
  private paintPlugins(info: Info) {
    const btn = this.box.querySelector<HTMLButtonElement>(".plugbtn")!;
    if (info.task) { btn.hidden = true; return; }
    const has = plugins.list().some((p) => !skills.packageIds().has(p.id)) || info.plugins !== null;
    btn.hidden =
      !!info.remote ||
      !info.workspace ||
      !has ||
      !capabilitiesOf(info.agent).workspacePluginSelection;
    if (btn.hidden) return;
    const working = () => {
      const status = this.ctx.info().status;
      return status === "rodando" || status === "querendo";
    };
    btn.innerHTML = `${icon("puzzle", 13)}<span></span>`;
    btn.querySelector("span")!.textContent = plugins.label(info.plugins);
    btn.classList.toggle("on", !!info.plugins);
    btn.title = `${t("plugin.title")}: ${plugins.label(info.plugins)}`;
    btn.setAttribute("aria-label", btn.title);
    btn.onclick = () => {
      const at = btn.getBoundingClientRect();
      const workspace = info.workspace!;
      plugins.openPicker({
        workspace,
        current: () => this.ctx.info().plugins,
        set: (sel) =>
          invoke("set_workspace_plugins", { id: workspace, plugins: sel }).catch((e) =>
            this.ctx.say(fromBack(e), true),
          ),
        at: () => ({ x: at.left, y: at.bottom + 4 }),
        working,
        trust: () => void trust.open(workspace, this.ctx.say),
      });
    };
  }

  /// Standalone skills are their own axis (ADR 0043) but ride the plugin pipeline, so they share the
  /// plugin capability gate and next-spawn rule.
  private paintSkills(info: Info) {
    const btn = this.box.querySelector<HTMLButtonElement>(".skillbtn")!;
    if (info.task) { btn.hidden = true; return; }
    const has = skills.packageIds().size > 0 || info.skills !== null;
    btn.hidden =
      !!info.remote ||
      !info.workspace ||
      !has ||
      !capabilitiesOf(info.agent).workspacePluginSelection;
    if (btn.hidden) return;
    const working = () => {
      const status = this.ctx.info().status;
      return status === "rodando" || status === "querendo";
    };
    btn.innerHTML = `${icon("sparkles", 13)}<span></span>`;
    btn.querySelector("span")!.textContent = plugins.label(info.skills, plugins.SKILL_WORDS);
    btn.classList.toggle("on", !!info.skills);
    btn.title = `${t("skill.title")}: ${plugins.label(info.skills, plugins.SKILL_WORDS)}`;
    btn.setAttribute("aria-label", btn.title);
    btn.onclick = () => {
      const at = btn.getBoundingClientRect();
      const workspace = info.workspace!;
      plugins.openSkillPicker({
        workspace,
        current: () => this.ctx.info().skills,
        set: (sel) =>
          invoke("set_workspace_skills", { id: workspace, skills: sel }).catch((e) =>
            this.ctx.say(fromBack(e), true),
          ),
        at: () => ({ x: at.left, y: at.bottom + 4 }),
        working,
        trust: () => void trust.open(workspace, this.ctx.say),
      });
    };
  }

  /// Keep personal companion access beside conversation controls and independent from team sharing.
  private paintRemoteControl(info: Info) {
    const btn = this.box.querySelector<HTMLButtonElement>(".remotebtn")!;
    btn.hidden = !!info.remote || !info.workspace || !team.status().config?.cloud;
    if (btn.hidden) return;
    btn.querySelector("span")!.textContent = t("remoteControl.label");
    btn.title = `${t("remoteControl.label")} — ${t(info.remoteControl ? "remoteControl.on" : "remoteControl.off")}\n${t("remoteControl.title")}`;
    btn.classList.toggle("on", info.remoteControl);
    btn.setAttribute("aria-pressed", String(info.remoteControl));
    btn.onclick = () => {
      btn.disabled = true;
      void team.remoteControl(info.workspace!, !info.remoteControl)
        .catch(error => this.ctx.say(fromBack(error), true))
        .finally(() => { btn.disabled = false; });
    };
  }

  /// Offer selection comments only when collaboration and selected text are available.
  private paintQuoteButton() {
    if (!this.key) return;
    const b = this.box.querySelector<HTMLElement>(".quotesel");
    if (b) b.hidden = !this.ctx.comment || !this.ctx.info().team || !this.selection().trim();
  }

  /* Team comments anchored to transcript pieces. */

  private paintCommentPins() {
    const ws = this.ctx.info().workspace;
    for (const old of this.feed.querySelectorAll(".commentpin")) old.remove();
    if (!ws || !this.key || !this.ctx.info().team || !this.ctx.thread) return;
    const grouped = new Map<string, ReturnType<typeof notes.rootsOf>>();
    for (const note of notes.rootsOf(team.notesOf(ws), this.key)) {
      if (note.resolved || !note.anchor) continue;
      const list = grouped.get(note.anchor) ?? [];
      list.push(note);
      grouped.set(note.anchor, list);
    }
    for (const [anchor, list] of grouped) {
      const node = this.feed.querySelector<HTMLElement>(`[data-key="${CSS.escape(anchor)}"]`);
      if (!node) continue;
      const pin = h("button", "commentpin", `${list.length}`);
      pin.title = tn(list.length, "notes.onTurn");
      pin.onclick = () => this.ctx.thread?.(list[0].id);
      node.append(pin);
    }
  }

  focusAnchor(anchor: string) {
    const el = this.feed.querySelector<HTMLElement>(`[data-key="${CSS.escape(anchor)}"]`);
    if (!el) return;
    for (const old of this.feed.querySelectorAll(".commentfocus")) old.classList.remove("commentfocus");
    el.classList.add("commentfocus");
    el.scrollIntoView({ block: "center" });
    setTimeout(() => el.classList.remove("commentfocus"), 1800);
  }
}
