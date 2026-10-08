import { decodeBinary, encodeSnapshot, encryptedBinary, isEncryptedUp, parseDown, parseEncrypted, parseShare, parseUp,
  SNAPSHOT, TEXT_FRAME_MAX, DOWN_FRAME_MAX, NOTE_TEXT_MAX, NOTE_QUOTE_MAX, MENTIONS_MAX,
  type Down, type Encrypted, type Inbox, type Member, type Note, type Share, type Shared, type Up } from "../relay/src/protocol";
import { decodeBase64Url, encodeBase64Url, open, seal, signIdentity } from "./team-crypto";
import { NO_RIGHTS, parseRights } from "./team-rights";
import { TeamSecurity } from "./team-security";
import type { ShareRights } from "./types";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
/// `rights` travels only inside an authenticated share announcement; older clients ignore it (ADR 0090).
type Payload = { frame: Up; expires?: number; revision?: number; rights?: unknown } | { binary: string };

/// The person a member acts for: a companion device maps to its primary member, anyone else to itself.
export const personOf = (members: ReadonlyArray<Pick<Member, "id" | "person">>, id: string): string =>
  members.find(m => m.id === id)?.person ?? id;

/** Content boundary shared by the desktop and the browser's simulated peers. */
export class TeamChannel {
  members: Member[] = [];
  readonly shares = new Map<string, Shared>();
  private owned = new Map<string, Shared>();
  private remoteControl = new Set<string>();
  /// Rights announced with owned shares, and those read from other owners' authenticated announcements.
  private ownedRights = new Map<string, ShareRights>();
  private announcedRights = new Map<string, ShareRights>();
  private attached: { ws: string; tab: string } | null = null;
  private pendingWelcome: Extract<Down, { t: "welcome" }> | null = null;
  ready = false;

  constructor(readonly security: TeamSecurity, readonly scope: string, readonly self: string) {}

  own(share: Share, remoteControl = false, rights: ShareRights = NO_RIGHTS) {
    const current = { ...share, owner: this.self, online: true };
    this.owned.set(share.id, current); this.shares.set(share.id, current);
    this.ownedRights.set(share.id, rights);
    if (remoteControl) this.remoteControl.add(share.id);
    else this.remoteControl.delete(share.id);
  }

  /// The rights another owner announced for a share; undefined when that owner predates rights.
  rightsOf(ws: string): ShareRights | undefined {
    return this.announcedRights.get(ws);
  }

  async identity(challenge: string): Promise<Up> {
    return { t: "identity", key: this.security.identity.publicKey,
      proof: await signIdentity(this.security.identity, [this.self, challenge]) };
  }

  private async pack(payload: Payload, recipients: string[]): Promise<Encrypted> {
    const id = crypto.randomUUID();
    const boxes: Encrypted["boxes"] = Object.create(null);
    const bytes = encoder.encode(JSON.stringify(payload));
    for (const member of new Set(recipients)) {
      const key = this.security.key(member);
      if (!key) throw new Error("Missing or changed identity");
      boxes[member] = await seal(this.security.identity, key, [this.scope, this.self, member, id], bytes);
    }
    const result = parseEncrypted({ id, boxes });
    if (!result) throw new Error("Encrypted frame too large");
    return result;
  }

  private async unpack(encrypted: Encrypted | undefined, author: string): Promise<Payload> {
    const key = this.security.key(author);
    const box = encrypted?.boxes[this.self];
    if (!encrypted || !key || !box) throw new Error("Unknown encrypted sender");
    const bytes = await open(this.security.identity, key, [this.scope, author, this.self, encrypted.id], box);
    const value = JSON.parse(decoder.decode(bytes));
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid encrypted payload");
    return value;
  }

  /// Audiences and mentions name people; every companion device of a person receives its own box.
  private devices(ids: Iterable<string>): string[] {
    const out = new Set<string>();
    for (const id of ids) {
      out.add(id);
      for (const m of this.members) if (m.person === id) out.add(m.id);
    }
    return [...out];
  }

  private recipients(share: Share & { owner?: string }): string[] {
    const owner = share.owner ?? this.self;
    const person = personOf(this.members, owner);
    // A local audience names people and may include the owner's own person (whole organization or a
    // self-mention); the owner's devices only join through remote control. Received audiences are already
    // expanded to devices and keep the owner's other devices as listed.
    const excluded = owner === this.self ? (id: string) => personOf(this.members, id) === person : (id: string) => id === owner;
    const audience = (share.audience ?? this.members.filter(m => !m.person).map(m => m.id)).filter(id => !excluded(id));
    const recipients = new Set([owner, ...this.devices(audience)]);
    if (this.remoteControl.has(share.id)) {
      for (const member of this.members) if (member.id !== owner && personOf(this.members, member.id) === person) recipients.add(member.id);
    }
    return [...recipients];
  }

  private allowed(ws: string, member: string): Shared {
    const share = this.shares.get(ws);
    if (!share || !this.recipients(share).includes(member) || !this.security.key(member)) throw new Error("Outside encrypted audience");
    return share;
  }

  async outgoing(frame: Up): Promise<Up> {
    if (!this.ready) throw new Error("Encryption not ready");
    let out = frame;
    switch (frame.t) {
      case "share": {
        // Directory members without a published key receive no content. A later
        // presence update reannounces the share after that member is ready.
        const audience = this.recipients(frame.share).filter(id => !!this.security.key(id));
        const plain: Share = { ...frame.share, audience: audience.filter(id => id !== this.self) };
        const encrypted = await this.pack({ frame: { t: "share", share: plain }, revision: await this.security.nextRevision(),
          rights: this.ownedRights.get(plain.id) ?? NO_RIGHTS }, audience);
        const wire: Share = { ...plain, title: "", repo_name: "", branch: "", stage: "", issue: null,
          tabs: plain.tabs.map(tab => ({ ...tab, title: "", note: null, tokens: null, status: "desligada" })), encrypted };
        this.shares.set(plain.id, { ...plain, owner: this.self, online: true });
        this.owned.set(plain.id, this.shares.get(plain.id)!);
        out = { t: "share", share: wire };
        break;
      }
      case "unshare": this.shares.delete(frame.ws); this.owned.delete(frame.ws); this.remoteControl.delete(frame.ws);
        this.ownedRights.delete(frame.ws); break;
      case "attach": this.allowed(frame.ws, this.self); this.attached = frame; break;
      case "detach": this.attached = null; break;
      case "write": {
        const share = this.allowed(frame.ws, this.self);
        if (!share.tabs.some(tab => tab.id === frame.tab)) throw new Error("Unknown encrypted tab");
        const encrypted = await this.pack({ frame, expires: Date.now() + 120_000 }, [share.owner]);
        out = { ...frame, data: "", encrypted }; break;
      }
      case "note":
      case "note_reply":
      case "note_resolve": {
        const share = this.allowed(frame.ws, this.self);
        if (frame.t !== "note_resolve" && (!frame.text.trim() || frame.text.length > NOTE_TEXT_MAX)) throw new Error("Invalid comment");
        if (frame.t === "note" && (frame.quote?.length ?? 0) > NOTE_QUOTE_MAX) throw new Error("Invalid quote");
        const plain: Up = frame.t === "note_resolve" ? frame : { ...frame, mentions: this.devices(frame.mentions) };
        if (plain.t !== "note_resolve" && plain.mentions.length > MENTIONS_MAX) throw new Error("Too many mentions");
        const encrypted = await this.pack({ frame: plain }, this.recipients(share));
        out = plain.t === "note" ? { ...plain, text: "", quote: null, anchor: null, encrypted }
          : plain.t === "note_reply" ? { ...plain, text: "", encrypted } : { ...plain, encrypted };
        break;
      }
    }
    if (!isEncryptedUp(out) || encoder.encode(JSON.stringify(out)).length > TEXT_FRAME_MAX) throw new Error("Invalid encrypted frame");
    return out;
  }

  async outgoingBinary(data: Uint8Array, recipients: string[]): Promise<Uint8Array[]> {
    const inner = decodeBinary(data);
    if (!inner) throw new Error("Invalid binary payload");
    const share = [...this.shares.values()].find(s => s.owner === this.self && s.tabs.some(t => t.id === inner.tab));
    if (!share) throw new Error("Unknown owned tab");
    const targets = inner.kind === SNAPSHOT ? [inner.to] : recipients;
    const frames: Uint8Array[] = [];
    for (const member of new Set(targets)) {
      this.allowed(share.id, member);
      const encrypted = await this.pack({ binary: encodeBase64Url(data) }, [member]);
      frames.push(encodeSnapshot(inner.tab, member, 0, encoder.encode(JSON.stringify(encrypted))));
    }
    return frames;
  }

  async incomingBinary(data: ArrayBuffer): Promise<Uint8Array> {
    const outer = encryptedBinary(data);
    if (!this.ready || !outer || outer.to !== this.self) throw new Error("Unencrypted binary frame");
    const share = [...this.shares.values()].find(s => s.tabs.some(t => t.id === outer.tab));
    if (!share || this.attached?.ws !== share.id || this.attached.tab !== outer.tab) throw new Error("Unattached encrypted tab");
    const plain = await this.unpack(outer.encrypted, share.owner);
    if (!("binary" in plain) || typeof plain.binary !== "string") throw new Error("Invalid binary content");
    const bytes = decodeBase64Url(plain.binary, 1024 * 1024);
    const inner = decodeBinary(bytes);
    if (!inner || inner.tab !== outer.tab || (inner.kind === SNAPSHOT && inner.to !== this.self)) throw new Error("Binary routing mismatch");
    return bytes;
  }

  private async readShare(wire: Shared): Promise<Shared> {
    if (wire.owner === this.self) {
      const own = this.owned.get(wire.id);
      if (!own) throw new Error("Unannounced own workspace");
      return own;
    }
    if (this.owned.has(wire.id)) throw new Error("Workspace owner changed");
    const payload = await this.unpack(wire.encrypted, wire.owner);
    const frame = "frame" in payload && parseUp(payload.frame);
    if (!frame || frame.t !== "share" || !parseShare(frame.share)) throw new Error("Invalid encrypted share");
    const plain = frame.share;
    if (plain.id !== wire.id || !this.recipients({ ...plain, owner: wire.owner }).includes(this.self) ||
      JSON.stringify(plain.tabs.map(t => t.id)) !== JSON.stringify(wire.tabs.map(t => t.id))) throw new Error("Share routing mismatch");
    const result = { ...plain, owner: wire.owner, online: wire.online };
    await this.security.observeShare(wire.id, wire.owner, this.security.key(wire.owner)!,
      "revision" in payload ? payload.revision! : 0, wire.encrypted!.id);
    this.shares.set(result.id, result);
    if ("rights" in payload) this.announcedRights.set(result.id, parseRights(payload.rights));
    else this.announcedRights.delete(result.id);
    return result;
  }

  private async readNote(wire: Note): Promise<Note> {
    this.allowed(wire.ws, wire.author);
    const payload = await this.unpack(wire.encrypted, wire.author);
    const frame = "frame" in payload && parseUp(payload.frame);
    if (!frame || (frame.t !== "note" && frame.t !== "note_reply") || frame.ws !== wire.ws ||
      (frame.t === "note_reply" ? frame.note !== wire.parent : !!wire.parent) || wire.id !== wire.encrypted?.id) throw new Error("Comment routing mismatch");
    if (frame.text.length > NOTE_TEXT_MAX || (frame.t === "note" && (frame.quote?.length ?? 0) > NOTE_QUOTE_MAX)) throw new Error("Comment too large");
    let resolved = false;
    if (wire.resolution) {
      this.allowed(wire.ws, wire.resolution.author);
      const resolution = await this.unpack(wire.resolution.encrypted, wire.resolution.author);
      const action = "frame" in resolution && parseUp(resolution.frame);
      resolved = !!action && action.t === "note_resolve" && action.ws === wire.ws && action.note === wire.id;
    }
    return { ...wire, text: frame.text, mentions: frame.mentions, quote: frame.t === "note" ? frame.quote : null,
      anchor: frame.t === "note" ? frame.anchor : null, tab: frame.t === "note" ? frame.tab : wire.tab, resolved };
  }

  private async readInbox(wire: Inbox): Promise<Inbox> {
    this.allowed(wire.ws, wire.author);
    const payload = await this.unpack(wire.encrypted, wire.author);
    const frame = "frame" in payload && parseUp(payload.frame);
    if (!frame || (frame.t !== "note" && frame.t !== "note_reply") || frame.ws !== wire.ws ||
      (frame.t === "note" ? wire.encrypted?.id !== wire.id : frame.note !== wire.id)) throw new Error("Inbox routing mismatch");
    return { ...wire, text: frame.text, tab: frame.t === "note" ? frame.tab : wire.tab };
  }

  private async readable<T, R>(items: T[], read: (item: T) => Promise<R>): Promise<R[]> {
    const result: R[] = [];
    for (const item of items) {
      try { result.push(await read(item)); } catch { /* Unreadable history stays opaque. */ }
    }
    return result;
  }

  async incoming(raw: unknown): Promise<Down | null> {
    if (JSON.stringify(raw).length > DOWN_FRAME_MAX) throw new Error("Oversized relay response");
    const frame = parseDown(raw);
    if (!frame) throw new Error("Invalid relay response");
    if (frame.t === "welcome") {
      if (frame.e2ee !== 1 || !frame.challenge || frame.you !== this.self) throw new Error("Relay lacks encryption");
      this.pendingWelcome = frame;
      this.members = frame.members;
      return null;
    }
    if (frame.t === "presence") {
      await this.security.observe(frame.members, this.self);
      this.members = frame.members;
      if (this.pendingWelcome && frame.members.find(m => m.id === this.self)?.key === this.security.identity.publicKey) {
        const welcome = this.pendingWelcome; this.pendingWelcome = null; this.ready = true;
        return { ...welcome, members: frame.members,
          shares: await this.readable(welcome.shares, s => this.readShare(s)),
          inbox: await this.readable(welcome.inbox, i => this.readInbox(i)) };
      }
      return this.ready ? frame : null;
    }
    if (!this.ready) return null;
    switch (frame.t) {
      case "share": return { ...frame, share: await this.readShare(frame.share) };
      case "unshare": if (!this.owned.has(frame.ws)) { this.shares.delete(frame.ws); this.announcedRights.delete(frame.ws); } return frame;
      case "watch": {
        const share = this.shares.get(frame.ws);
        if (!share || share.owner !== this.self || !share.tabs.some(t => t.id === frame.tab)) return null;
        const allowed = (member: string) => this.recipients(share).includes(member) && !!this.security.key(member);
        return { ...frame, members: frame.members.filter(allowed), added: frame.added.filter(allowed) };
      }
      case "write": {
        const share = this.allowed(frame.ws, frame.from);
        if (share.owner !== this.self || !share.tabs.some(t => t.id === frame.tab)) throw new Error("Input audience mismatch");
        const payload = await this.unpack(frame.encrypted, frame.from);
        const plain = "frame" in payload && parseUp(payload.frame);
        if (!plain || plain.t !== "write" || plain.ws !== frame.ws || plain.tab !== frame.tab) throw new Error("Input routing mismatch");
        await this.security.consume(frame.encrypted!.id, "expires" in payload ? payload.expires! : 0);
        return { ...frame, data: plain.data };
      }
      case "note": return { ...frame, note: await this.readNote(frame.note) };
      case "notes": return { ...frame, items: await this.readable(frame.items, n => this.readNote(n)) };
      case "inbox": return { ...frame, items: await this.readable(frame.items, i => this.readInbox(i)) };
      default: return frame;
    }
  }
}
