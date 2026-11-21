import { ChatView, type Ctx } from "./chat";
import { fromBack, t } from "./i18n";
import { icon } from "./icons";
import type { Item } from "./menu";
import { pasteFiles } from "./paste";
import { TerminalConversationView } from "./terminal-conversation";
import { h } from "./util";
import "./terminal-conversation.css";

/** Show the saved conversation interface with its shared product controls. */
export class ConversationView {
  private chat = new ChatView();
  private terminal: TerminalConversationView | null = null;
  private ctx!: Ctx;
  private root!: HTMLElement;
  private chatHost!: HTMLElement;
  private terminalHost!: HTMLElement;
  private toolsAttention = false;
  private key: string | null = null;
  private version = 0;
  private native = false;
  private disposed = false;
  private paste = (event: ClipboardEvent) => {
    if (!this.native) return;
    const target = this.chat.fileDropTarget();
    if (!target) return;
    pasteFiles(event, target.put, error => this.ctx.say(fromBack(error), true));
    if (event.defaultPrevented) event.stopPropagation();
  };

  open(host: HTMLElement, ctx: Ctx) {
    this.ctx = ctx;
    this.root = h("div", "conversation-surface");
    this.root.dataset.trackArea = "conversation";
    this.chatHost = h("div", "chatwrap conversation-product");
    this.terminalHost = h("div", "conversation-terminal");
    this.terminalHost.addEventListener("paste", this.paste, true);
    this.root.append(this.terminalHost, this.chatHost);
    host.append(this.root);
    this.chat.open(this.chatHost, {
      ...ctx,
      nativeInput: () => this.nativeInput(),
      nativeToolsChanged: attention => {
        this.toolsAttention = attention;
        ctx.nativeToolsChanged?.(attention);
      },
    });
    this.paint();
  }

  async attach(key: string) {
    if (this.disposed) return;
    this.version++;
    // Session metadata resolves through current(), so establish identity before reading it.
    this.key = key;
    this.native = this.ctx.info().conversationMode === "terminal";
    if (this.native && !this.terminal) {
      this.terminal = new TerminalConversationView();
      this.terminal.open(this.terminalHost, this.ctx, () => {
        this.chat.refresh();
        this.paint();
      });
    }
    if (!this.native) this.terminal?.detach();
    this.paint();
    await Promise.all([this.chat.attach(key), ...(this.native ? [this.terminal!.attach(key)] : [])]);
  }

  attachRemote(key: string, bytes: Uint8Array) {
    if (this.disposed) return;
    this.version++;
    this.key = key;
    this.native = false;
    this.terminal?.detach();
    this.chat.attachRemote(key, bytes);
    this.paint();
  }

  remoteWrite(key: string, bytes: Uint8Array) { this.chat.remoteWrite(key, bytes); }

  detach() {
    this.version++;
    this.key = null;
    this.native = false;
    this.terminal?.detach();
    this.chat.detach();
    this.paint();
  }

  dispose(forget = false) {
    if (this.disposed) return;
    this.disposed = true;
    this.key = null;
    this.terminal?.dispose();
    this.terminalHost.removeEventListener("paste", this.paste, true);
    this.chat.dispose(forget);
    this.root.remove();
  }

  current() { return this.key; }
  nativeInput() { return this.native && !this.disposed ? this.terminal?.inputTarget() ?? null : null; }
  /** The attached native process exited and is not being resumed. */
  nativeStopped() { return this.native && !this.disposed && !!this.terminal && !this.terminal.busy() && !this.terminal.running(); }
  nativeToolsPending() { return !!this.key && this.native && this.toolsAttention; }
  showNativeTools() { if (this.native && this.key && !this.disposed) this.chat.showNativeTools(); }

  /** Hosts place these actions in their existing tab or workspace menus, outside native output. */
  nativeActions(): Item[] {
    const key = this.key;
    const version = this.version;
    if (!this.native || !key || this.disposed) return [];
    const run = (action: () => void) => () => {
      if (this.key === key && this.version === version && this.native && !this.disposed) action();
    };
    const items: Item[] = [
      { label: t("terminalConversation.tools"), track: "conversation.terminal.tools.open", glyph: icon("settings"), run: run(() => this.showNativeTools()) },
    ];
    if (this.chat.canAttachFiles()) items.push({ label: t("chat.addFile"), track: "chat.addFile", glyph: icon("paperclip"), run: run(() => this.chat.pickFile()) });
    if (!this.terminal?.busy() && !this.terminal?.running()) items.push({
      label: t("terminalConversation.resume"), track: "conversation.terminal.resume", glyph: icon("play"), run: run(() => { void this.terminal?.resume(); }),
    });
    if (this.ctx.info().team && this.ctx.comment && this.terminal?.selection().trim()) items.push({
      label: t("notes.quoteSelection"), track: "notes.quoteSelection", glyph: icon("message-square"), run: run(() => { this.quoteSelection(); }),
    });
    return items;
  }
  focus() { this.native ? this.terminal!.focus() : this.chat.focus(); }
  refresh() {
    this.chat.refresh();
    if (this.native) this.terminal?.refresh();
    this.paint();
  }
  forget(alive: Set<string>) { this.chat.forget(alive); }
  selection() { return this.native ? this.terminal!.selection() : this.chat.selection(); }
  canAttachFiles() { return this.chat.canAttachFiles(); }
  attachFiles(paths: string[]) {
    const target = this.fileDropTarget();
    if (!target) return false;
    target.put(paths);
    return true;
  }

  fileDropTarget(): ReturnType<ChatView["fileDropTarget"]> {
    return this.chat.fileDropTarget();
  }

  contextTarget(): ReturnType<ChatView["contextTarget"]> {
    return this.chat.contextTarget();
  }

  quoteSelection() {
    if (!this.native) return this.chat.quoteSelection();
    const quote = this.terminal!.selection().trim();
    if (!this.key || !quote || !this.ctx.info().team || !this.ctx.comment) return false;
    this.ctx.comment({ tab: this.key, anchor: null, quote });
    return true;
  }

  focusAnchor(anchor: string) {
    if (this.native) this.terminal!.focus();
    else this.chat.focusAnchor(anchor);
  }

  private paint() {
    if (!this.root || this.disposed) return;
    this.root.classList.toggle("conversation-native", this.native);
    this.terminalHost.hidden = !this.native;
  }
}
