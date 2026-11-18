/// Shared relay contract for the Worker and desktop; depend only on APIs available in both environments. Control and encrypted envelopes use JSON; binary envelopes retain a unicast routing header.

// Protocol v4 requires E2EE and identity proof on each connection. Existing v3 enrollment credentials remain valid; plaintext v3 content is never a fallback.
export const PROTO = 4;

/* control limits and validation */

export const NAME_MAX = 80;
export const TITLE_MAX = 256;
export const URL_MAX = 2 * 1024;
export const TAB_NOTE_MAX = 4 * 1024;
export const TABS_MAX = 32;
export const AUDIENCE_MAX = 64;
export const MENTIONS_MAX = 32;
export const SHARES_MAX = 64;
export const NOTES_PER_WORKSPACE_MAX = 200;
export const NOTES_TOTAL_MAX = 2_000;
export const INBOX_MAX = 500;
export const NOTE_TTL_MS = 90 * 24 * 60 * 60 * 1_000;
export const WRITE_MAX = 64 * 1024;
export const TEXT_FRAME_MAX = 2 * 1024 * 1024;
// Welcome aggregates previously accepted frames, so its bounded client limit must exceed one share frame.
export const DOWN_FRAME_MAX = 16 * 1024 * 1024;
export const BINARY_FRAME_MAX = 1024 * 1024;
export const BINARY_SEGMENTS_MAX = 256;
export const MEMBERS_MAX = 64;
export const SOCKETS_MAX = 128;
export const SOCKETS_PER_MEMBER_MAX = 4;
export const RATE_WINDOW_MS = 10_000;
export const FRAMES_PER_WINDOW_MAX = 500;
export const BYTES_PER_WINDOW_MAX = 16 * 1024 * 1024;
export const STORED_ENCRYPTED_MAX = 1536 * 1024;
export const SHARES_ENCRYPTED_MAX = 4 * 1024 * 1024;
export const NOTES_ENCRYPTED_MAX = 8 * 1024 * 1024;

const ID = /^[A-Za-z0-9_-]{1,64}$/;
const NOTE_ID = /^[A-Za-z0-9_-]{1,128}$/;
const SECRET = /^[A-Za-z0-9_-]{16,128}$/;
const CREDENTIAL = /^[A-Za-z0-9_-]{32,128}$/;
const RESERVED_KEYS = new Set(["__proto__", "prototype", "constructor"]);
const STATUSES = new Set<Status>(["rodando", "querendo", "pronta", "desligada"]);

const record = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const text = (v: unknown, max: number, empty = true): v is string =>
  typeof v === "string" && v.length <= max && (empty || v.trim().length > 0) && !v.includes("\0");
const id = (v: unknown): v is string => typeof v === "string" && ID.test(v) && !RESERVED_KEYS.has(v);
const noteId = (v: unknown): v is string => typeof v === "string" && NOTE_ID.test(v);
const integer = (v: unknown, min: number, max: number): v is number => Number.isSafeInteger(v) && Number(v) >= min && Number(v) <= max;

export const isId = id;
export const isInviteSecret = (v: unknown): v is string => typeof v === "string" && SECRET.test(v);
export const isCredential = (v: unknown): v is string => typeof v === "string" && CREDENTIAL.test(v);

export type Sealed = { enc: string; ct: string };
export type Encrypted = { id: string; boxes: Record<string, Sealed> };
export const isPublicKey = (v: unknown): v is string => typeof v === "string" && /^[A-Za-z0-9_-]{87}$/.test(v);
export function parseEncrypted(value: unknown): Encrypted | null {
  if (!record(value) || !id(value.id) || !record(value.boxes)) return null;
  const entries = Object.entries(value.boxes);
  if (!entries.length || entries.length > MEMBERS_MAX) return null;
  const boxes: Record<string, Sealed> = Object.create(null);
  let size = 0;
  for (const [member, box] of entries) {
    if (!id(member) || !record(box) || !isPublicKey(box.enc) || typeof box.ct !== "string" ||
      box.ct.length < 22 || box.ct.length > BINARY_FRAME_MAX || !/^[A-Za-z0-9_-]+$/.test(box.ct)) return null;
    size += box.ct.length + box.enc.length;
    if (size > TEXT_FRAME_MAX) return null;
    boxes[member] = { enc: box.enc, ct: box.ct };
  }
  return { id: value.id, boxes };
}

function encryption(value: Record<string, unknown>): { encrypted?: Encrypted } | null {
  if (value.encrypted === undefined) return {};
  const encrypted = parseEncrypted(value.encrypted);
  return encrypted ? { encrypted } : null;
}

export function normalizeName(value: unknown, fallback = ""): string {
  if (typeof value !== "string") return fallback;
  const clean = value.trim();
  return text(clean, NAME_MAX, false) ? clean : fallback;
}

export type Status = "rodando" | "querendo" | "pronta" | "desligada";

/// A tab advertisement containing the state required by a remote card.
export type ShareTab = {
  /// Native slash commands and permission modes belong to the owner's terminal.
  conversation_mode?: "terminal";
  /// Native decisions stay on the owner's terminal; omitted by legacy/chat announcements.
  decision_surface?: "terminal";
  id: string;
  title: string;
  status: Status;
  note: string | null;
  tokens: number | null;
};

/// The owner's latest shared workspace snapshot, updated when its board state changes.
export type Share = {
  encrypted?: Encrypted;
  id: string;
  title: string;
  repo_name: string;
  branch: string;
  stage: string;
  issue: { identifier: string; title: string; url: string } | null;
  active: string | null;
  tabs: ShareTab[];
  /// Terminal dimensions per tab, as `[cols, rows]`.
  sizes: Record<string, [number, number]>;
  /// Authorized member IDs, or null for the entire team. The relay filters routing; clients verify the audience and encrypt only for authorized recipients. The v4 wire audience is always explicit.
  audience: string[] | null;
};

/// A share plus relay-owned owner identity and presence. `left` lists the people who left the share; the relay keeps
/// them out of its audience until the owner readmits them, and omits the field when nobody left.
export type Shared = Share & { owner: string; online: boolean; left?: string[] };

/// A member is one device. `person` links a companion device to the membership it belongs to; absent on primary members and legacy teams.
export type Member = { id: string; name: string; online: boolean; key?: string; person?: string };

export type Note = {
  encrypted?: Encrypted;
  resolution?: { author: string; encrypted: Encrypted };
  id: string;
  ws: string;
  author: string;
  text: string;
  mentions: string[];
  /// Optional transcript excerpt quoted by a comment.
  quote: string | null;
  ts: number;
  /// Conversation and transcript location; absent on older standalone notes.
  tab?: string | null;
  anchor?: string | null;
  /// Replies are stored flat and reference their root comment.
  parent?: string | null;
  /// Only roots have resolution state. Missing legacy values mean open.
  resolved?: boolean;
};

/// An open comment awaiting this member. The ID identifies its root; older inbox entries may omit text and tab.
export type Inbox = { id: string; ws: string; author: string; ts: number; tab?: string | null; text?: string; encrypted?: Encrypted };

/// Viewers of the owner's workspaces, indexed by workspace, then tab.
export type Watching = Record<string, Record<string, string[]>>;

/// App-to-relay controls.
export type Up =
  | { t: "identity"; key: string; proof: string }
  | { t: "renew"; ticket: string }
  | { t: "me"; name: string }
  | { t: "share"; share: Share }
  | { t: "unshare"; ws: string }
  /// A viewer leaves another owner's share: every device of the viewer's person stops receiving it.
  | { t: "leave"; ws: string }
  /// The owner lets a person who left back into the share's audience.
  | { t: "readmit"; ws: string; person: string }
  | { t: "attach"; ws: string; tab: string }
  | { t: "detach" }
  | { t: "size"; ws: string; tab: string; cols: number; rows: number }
  | { t: "write"; ws: string; tab: string; data: string; encrypted?: Encrypted }
  | { t: "note"; ws: string; tab?: string | null; anchor?: string | null; text: string; mentions: string[]; quote: string | null; encrypted?: Encrypted }
  | { t: "note_reply"; ws: string; note: string; text: string; mentions: string[]; encrypted?: Encrypted }
  | { t: "note_resolve"; ws: string; note: string; encrypted?: Encrypted }
  | { t: "notes"; ws: string }
  | { t: "inbox_read"; id: string };

/// Relay-to-app controls.
export type Down =
  | { t: "welcome"; you: string; members: Member[]; shares: Shared[]; inbox: Inbox[]; watching: Watching; comments?: 1; e2ee?: 1; challenge?: string }
  | { t: "lease"; expires_in: number }
  | { t: "presence"; members: Member[] }
  | { t: "share"; share: Shared }
  | { t: "unshare"; ws: string }
  | { t: "watch"; ws: string; tab: string; members: string[]; added: string[] }
  | { t: "write"; ws: string; tab: string; data: string; from: string; encrypted?: Encrypted }
  | { t: "size"; ws: string; tab: string; cols: number; rows: number }
  | { t: "note"; note: Note }
  | { t: "notes"; ws: string; items: Note[] }
  | { t: "inbox"; items: Inbox[] }
  | { t: "error"; code: string };

/// Comment limits enforced before sending and again at the relay trust boundary.
export const NOTE_TEXT_MAX = 8 * 1024;
export const NOTE_QUOTE_MAX = 4 * 1024;
export const NOTE_ANCHOR_MAX = 128;

/// HTTP responses for team creation and enrollment. Invite `secret` values are shareable; individual
/// `credential` values stay private and authenticate WebSockets.
export type Membership = { member: string; credential: string };
export type CreatedTeam = Membership & { team: string; secret: string };
export type EnrollRequest = { secret: string };
export type EnrollResponse = Membership;

export function parseMembership(value: unknown): Membership | null {
  if (!record(value) || !id(value.member) || !isCredential(value.credential)) return null;
  return { member: value.member, credential: value.credential };
}

export function parseCreatedTeam(value: unknown): CreatedTeam | null {
  const membership = parseMembership(value);
  if (!membership || !record(value) || !id(value.team) || !isInviteSecret(value.secret)) return null;
  return { team: value.team, secret: value.secret, ...membership };
}

export function parseShare(value: unknown): Share | null {
  if (!record(value) || !id(value.id)) return null;
  const sealed = encryption(value);
  if (!sealed) return null;
  if (!text(value.title, TITLE_MAX) || !text(value.repo_name, TITLE_MAX) || !text(value.branch, TITLE_MAX) || !text(value.stage, TITLE_MAX)) return null;

  let issue: Share["issue"] = null;
  if (value.issue !== null) {
    if (!record(value.issue)) return null;
    if (
      !text(value.issue.identifier, TITLE_MAX, false) ||
      !text(value.issue.title, TITLE_MAX) ||
      !text(value.issue.url, URL_MAX) ||
      !/^https?:\/\/[^\s]+$/i.test(value.issue.url)
    ) return null;
    issue = { identifier: value.issue.identifier, title: value.issue.title, url: value.issue.url };
  }

  if (!Array.isArray(value.tabs) || value.tabs.length > TABS_MAX) return null;
  const tabs: ShareTab[] = [];
  const tabIds = new Set<string>();
  for (const raw of value.tabs) {
    if (!record(raw) || !id(raw.id) || tabIds.has(raw.id) || !text(raw.title, TITLE_MAX)) return null;
    if (typeof raw.status !== "string" || !STATUSES.has(raw.status as Status)) return null;
    if (raw.note !== null && !text(raw.note, TAB_NOTE_MAX)) return null;
    if (raw.tokens !== null && !integer(raw.tokens, 0, Number.MAX_SAFE_INTEGER)) return null;
    if (raw.decision_surface !== undefined && raw.decision_surface !== "terminal") return null;
    if (raw.conversation_mode !== undefined && raw.conversation_mode !== "terminal") return null;
    tabIds.add(raw.id);
    tabs.push({ id: raw.id, title: raw.title, status: raw.status as Status, note: raw.note as string | null, tokens: raw.tokens as number | null,
      ...(raw.decision_surface === "terminal" ? { decision_surface: "terminal" as const } : {}),
      ...(raw.conversation_mode === "terminal" ? { conversation_mode: "terminal" as const } : {}),
    });
  }

  if (value.active !== null && (!id(value.active) || !tabIds.has(value.active))) return null;
  if (!record(value.sizes) || Object.keys(value.sizes).length > tabs.length) return null;
  const sizes: Record<string, [number, number]> = Object.create(null) as Record<string, [number, number]>;
  for (const [tab, size] of Object.entries(value.sizes)) {
    if (!tabIds.has(tab) || !Array.isArray(size) || size.length !== 2 || !integer(size[0], 1, 1_000) || !integer(size[1], 1, 1_000)) return null;
    sizes[tab] = [size[0], size[1]];
  }

  let audience: string[] | null = null;
  if (value.audience !== null && value.audience !== undefined) {
    if (!Array.isArray(value.audience) || value.audience.length > AUDIENCE_MAX || !value.audience.every(id)) return null;
    audience = [...new Set(value.audience)];
  }

  return {
    ...sealed,
    id: value.id,
    title: value.title,
    repo_name: value.repo_name,
    branch: value.branch,
    stage: value.stage,
    issue,
    active: value.active as string | null,
    tabs,
    sizes,
    audience,
  };
}

/// Validate and copy control frames so unexpected properties, getters and unbounded values cannot cross
/// the relay boundary.
export function parseUp(value: unknown): Up | null {
  if (!record(value) || typeof value.t !== "string") return null;
  const sealed = encryption(value);
  if (!sealed) return null;
  switch (value.t) {
    case "identity":
      return isPublicKey(value.key) && typeof value.proof === "string" && /^[A-Za-z0-9_-]{86}$/.test(value.proof)
        ? { t: "identity", key: value.key, proof: value.proof } : null;
    case "renew":
      return typeof value.ticket === "string" && /^[A-Za-z0-9_-]{43}$/.test(value.ticket)
        ? { t: "renew", ticket: value.ticket } : null;
    case "me":
      return text(value.name, NAME_MAX) ? { t: "me", name: value.name } : null;
    case "share": {
      const share = parseShare(value.share);
      return share ? { t: "share", share } : null;
    }
    case "unshare":
    case "leave":
    case "notes":
      return id(value.ws) ? { t: value.t, ws: value.ws } : null;
    case "readmit":
      return id(value.ws) && id(value.person) ? { t: "readmit", ws: value.ws, person: value.person } : null;
    case "attach":
      return id(value.ws) && id(value.tab) ? { t: "attach", ws: value.ws, tab: value.tab } : null;
    case "detach":
      return { t: "detach" };
    case "size":
      return id(value.ws) && id(value.tab) && integer(value.cols, 1, 1_000) && integer(value.rows, 1, 1_000)
        ? { t: "size", ws: value.ws, tab: value.tab, cols: value.cols, rows: value.rows }
        : null;
    case "write":
      return id(value.ws) && id(value.tab) && text(value.data, WRITE_MAX)
        ? { t: "write", ws: value.ws, tab: value.tab, data: value.data, ...sealed }
        : null;
    case "note": {
      if (!id(value.ws) || !text(value.text, TEXT_FRAME_MAX) || (value.quote !== null && !text(value.quote, TEXT_FRAME_MAX))) return null;
      if (!Array.isArray(value.mentions) || value.mentions.length > MENTIONS_MAX) return null;
      if (value.tab !== null && value.tab !== undefined && !id(value.tab)) return null;
      if (value.anchor !== null && value.anchor !== undefined && !text(value.anchor, NOTE_ANCHOR_MAX, false)) return null;
      const mentions = value.mentions.filter(id);
      return {
        t: "note",
        ...sealed,
        ws: value.ws,
        tab: (value.tab as string | null | undefined) ?? null,
        anchor: (value.anchor as string | null | undefined) ?? null,
        text: value.text,
        mentions: [...new Set(mentions)],
        quote: value.quote as string | null,
      };
    }
    case "note_reply": {
      if (!id(value.ws) || !noteId(value.note) || !text(value.text, TEXT_FRAME_MAX)) return null;
      if (!Array.isArray(value.mentions) || value.mentions.length > MENTIONS_MAX) return null;
      const mentions = value.mentions.filter(id);
      return { t: "note_reply", ws: value.ws, note: value.note, text: value.text, mentions: [...new Set(mentions)], ...sealed };
    }
    case "note_resolve":
      return id(value.ws) && noteId(value.note) ? { t: "note_resolve", ws: value.ws, note: value.note, ...sealed } : null;
    case "inbox_read":
      return noteId(value.id) ? { t: "inbox_read", id: value.id } : null;
    default:
      return null;
  }
}

function parseMembers(value: unknown): Member[] | null {
  if (!Array.isArray(value) || value.length > MEMBERS_MAX) return null;
  const out: Member[] = [];
  const seen = new Set<string>();
  for (const raw of value) {
    if (!record(raw) || !id(raw.id) || seen.has(raw.id) || !text(raw.name, NAME_MAX, false) || typeof raw.online !== "boolean") return null;
    seen.add(raw.id);
    if (raw.key !== undefined && !isPublicKey(raw.key)) return null;
    if (raw.person !== undefined && (!id(raw.person) || raw.person === raw.id)) return null;
    out.push({ id: raw.id, name: raw.name, online: raw.online, ...(typeof raw.key === "string" ? { key: raw.key } : {}),
      ...(typeof raw.person === "string" ? { person: raw.person } : {}) });
  }
  return out;
}

function parseShared(value: unknown): Shared | null {
  const share = parseShare(value);
  if (!share || !record(value) || !id(value.owner) || typeof value.online !== "boolean") return null;
  if (value.left !== undefined && (!Array.isArray(value.left) || value.left.length > MEMBERS_MAX || !value.left.every(id))) return null;
  const left = Array.isArray(value.left) && value.left.length ? { left: [...new Set(value.left as string[])] } : {};
  return { ...share, owner: value.owner, online: value.online, ...left };
}

export function parseNote(value: unknown): Note | null {
  if (!record(value) || !noteId(value.id) || !id(value.ws) || !id(value.author)) return null;
  const sealed = encryption(value);
  if (!sealed) return null;
  let resolution: Note["resolution"];
  if (value.resolution !== undefined) {
    if (!record(value.resolution) || !id(value.resolution.author)) return null;
    const encrypted = parseEncrypted(value.resolution.encrypted);
    if (!encrypted) return null;
    resolution = { author: value.resolution.author, encrypted };
  }
  if (!text(value.text, NOTE_TEXT_MAX) || (value.quote !== null && !text(value.quote, NOTE_QUOTE_MAX))) return null;
  if (!Array.isArray(value.mentions) || value.mentions.length > MENTIONS_MAX || !value.mentions.every(id)) return null;
  if (!integer(value.ts, 0, Number.MAX_SAFE_INTEGER)) return null;
  if (value.tab !== undefined && value.tab !== null && !id(value.tab)) return null;
  if (value.anchor !== undefined && value.anchor !== null && !text(value.anchor, NOTE_ANCHOR_MAX, false)) return null;
  if (value.parent !== undefined && value.parent !== null && !noteId(value.parent)) return null;
  if (value.resolved !== undefined && typeof value.resolved !== "boolean") return null;
  return {
    ...sealed,
    ...(resolution ? { resolution } : {}),
    id: value.id,
    ws: value.ws,
    author: value.author,
    text: value.text,
    mentions: [...new Set(value.mentions)],
    quote: value.quote as string | null,
    ts: value.ts,
    tab: (value.tab as string | null | undefined) ?? null,
    anchor: (value.anchor as string | null | undefined) ?? null,
    parent: (value.parent as string | null | undefined) ?? null,
    resolved: value.resolved === true,
  };
}

export function parseInbox(value: unknown): Inbox | null {
  if (!record(value) || !noteId(value.id) || !id(value.ws) || !id(value.author) || !integer(value.ts, 0, Number.MAX_SAFE_INTEGER)) return null;
  const sealed = encryption(value);
  if (!sealed) return null;
  if (value.tab !== undefined && value.tab !== null && !id(value.tab)) return null;
  if (value.text !== undefined && !text(value.text, NOTE_TEXT_MAX)) return null;
  return {
    ...sealed,
    id: value.id,
    ws: value.ws,
    author: value.author,
    ts: value.ts,
    tab: (value.tab as string | null | undefined) ?? null,
    ...(typeof value.text === "string" ? { text: value.text } : {}),
  };
}

function list<T>(value: unknown, max: number, parse: (item: unknown) => T | null): T[] | null {
  if (!Array.isArray(value) || value.length > max) return null;
  const out: T[] = [];
  for (const raw of value) {
    const item = parse(raw);
    if (!item) return null;
    out.push(item);
  }
  return out;
}

function parseWatching(value: unknown): Watching | null {
  if (!record(value) || Object.keys(value).length > SHARES_MAX) return null;
  const out: Watching = Object.create(null) as Watching;
  for (const [ws, rawTabs] of Object.entries(value)) {
    if (!id(ws) || !record(rawTabs) || Object.keys(rawTabs).length > TABS_MAX) return null;
    const tabs: Record<string, string[]> = Object.create(null) as Record<string, string[]>;
    for (const [tab, rawMembers] of Object.entries(rawTabs)) {
      if (!id(tab) || !Array.isArray(rawMembers) || rawMembers.length > MEMBERS_MAX || !rawMembers.every(id)) return null;
      tabs[tab] = [...new Set(rawMembers)];
    }
    out[ws] = tabs;
  }
  return out;
}

/// Validate configurable relay responses before changing webview state. Size limits also bound
/// allocations from untrusted endpoints.
export function parseDown(value: unknown): Down | null {
  if (!record(value) || typeof value.t !== "string") return null;
  const sealed = encryption(value);
  if (!sealed) return null;
  switch (value.t) {
    case "lease":
      return integer(value.expires_in, 1, 60_000) ? { t: "lease", expires_in: value.expires_in } : null;
    case "welcome": {
      if (!id(value.you)) return null;
      const members = parseMembers(value.members);
      const shares = list(value.shares, SHARES_MAX, parseShared);
      const inbox = list(value.inbox, INBOX_MAX, parseInbox);
      const watching = parseWatching(value.watching);
      if (value.comments !== undefined && value.comments !== 1) return null;
      if (value.e2ee !== undefined && (value.e2ee !== 1 || !id(value.challenge))) return null;
      return members && shares && inbox && watching
        ? { t: "welcome", you: value.you, members, shares, inbox, watching, ...(value.comments === 1 ? { comments: 1 as const } : {}), ...(value.e2ee === 1 ? { e2ee: 1 as const, challenge: value.challenge as string } : {}) }
        : null;
    }
    case "presence": {
      const members = parseMembers(value.members);
      return members ? { t: "presence", members } : null;
    }
    case "share": {
      const share = parseShared(value.share);
      return share ? { t: "share", share } : null;
    }
    case "unshare":
      return id(value.ws) ? { t: "unshare", ws: value.ws } : null;
    case "watch": {
      if (!id(value.ws) || !id(value.tab)) return null;
      if (!Array.isArray(value.members) || value.members.length > MEMBERS_MAX || !value.members.every(id)) return null;
      if (!Array.isArray(value.added) || value.added.length > MEMBERS_MAX || !value.added.every(id)) return null;
      return { t: "watch", ws: value.ws, tab: value.tab, members: [...new Set(value.members)], added: [...new Set(value.added)] };
    }
    case "write":
      return id(value.ws) && id(value.tab) && id(value.from) && text(value.data, WRITE_MAX)
        ? { t: "write", ws: value.ws, tab: value.tab, data: value.data, from: value.from, ...sealed }
        : null;
    case "size":
      return id(value.ws) && id(value.tab) && integer(value.cols, 1, 1_000) && integer(value.rows, 1, 1_000)
        ? { t: "size", ws: value.ws, tab: value.tab, cols: value.cols, rows: value.rows }
        : null;
    case "note": {
      const note = parseNote(value.note);
      return note ? { t: "note", note } : null;
    }
    case "notes": {
      if (!id(value.ws)) return null;
      const items = list(value.items, NOTES_PER_WORKSPACE_MAX, parseNote);
      return items && items.every((note) => note.ws === value.ws) ? { t: "notes", ws: value.ws, items } : null;
    }
    case "inbox": {
      const items = list(value.items, INBOX_MAX, parseInbox);
      return items ? { t: "inbox", items } : null;
    }
    case "error":
      return id(value.code) ? { t: "error", code: value.code } : null;
    default:
      return null;
  }
}

/* binary frames */

/// Live tab output sent to its current viewers.
export const LIVE = 0;
/// A transcript snapshot for one newly connected viewer. Parts stay within the relay's 1 MB message
/// limit; the final part clears `more`.
export const SNAPSHOT = 1;

/// A numbered output segment. Owners batch segments to reduce relay messages; sequence numbers let
/// viewers skip content already included in snapshots.
export type Segment = { seq: number; bytes: Uint8Array };

export type Binary =
  | { kind: typeof LIVE; tab: string; segments: Segment[] }
  | { kind: typeof SNAPSHOT; tab: string; to: string; seq: number; more: boolean; bytes: Uint8Array };

const enc = new TextEncoder();
const dec = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false });

function decodeId(bytes: Uint8Array): string | null {
  try {
    const value = dec.decode(bytes);
    return id(value) ? value : null;
  } catch {
    return null;
  }
}

/// Write an integer up to 2^53 as eight big-endian bytes without BigInt.
function putU64(view: DataView, at: number, n: number) {
  view.setUint32(at, Math.floor(n / 4294967296));
  view.setUint32(at + 4, n >>> 0);
}
function getU64(view: DataView, at: number): number {
  return view.getUint32(at) * 4294967296 + view.getUint32(at + 4);
}

/// `[0][n][tab...][k][seq,len x k][bytes...]`. Length-prefixed IDs avoid depending on fixed UUID
/// representations.
export function encodeLive(tab: string, segments: Segment[]): Uint8Array {
  const id = enc.encode(tab);
  const total = segments.reduce((n, s) => n + s.bytes.length, 0);
  const out = new Uint8Array(2 + id.length + 2 + segments.length * 12 + total);
  const view = new DataView(out.buffer);
  out[0] = LIVE;
  out[1] = id.length;
  out.set(id, 2);
  let at = 2 + id.length;
  view.setUint16(at, segments.length);
  at += 2;
  for (const s of segments) {
    putU64(view, at, s.seq);
    view.setUint32(at + 8, s.bytes.length);
    at += 12;
  }
  for (const s of segments) {
    out.set(s.bytes, at);
    at += s.bytes.length;
  }
  return out;
}

/// `[1][n][tab...][m][to...][seq][more][bytes...]` carries a snapshot part through `seq`. `more` is 1
/// when another part follows.
export function encodeSnapshot(tab: string, to: string, seq: number, bytes: Uint8Array, more = false): Uint8Array {
  const id = enc.encode(tab);
  const who = enc.encode(to);
  const out = new Uint8Array(3 + id.length + who.length + 9 + bytes.length);
  const view = new DataView(out.buffer);
  out[0] = SNAPSHOT;
  out[1] = id.length;
  out.set(id, 2);
  let at = 2 + id.length;
  out[at] = who.length;
  out.set(who, at + 1);
  at += 1 + who.length;
  putU64(view, at, seq);
  out[at + 8] = more ? 1 : 0;
  out.set(bytes, at + 9);
  return out;
}

/// Return `null` for malformed frames; never forward them.
export function decodeBinary(data: ArrayBuffer | Uint8Array): Binary | null {
  const buf = data instanceof Uint8Array ? data : new Uint8Array(data);
  if (buf.length > BINARY_FRAME_MAX) return null;
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  if (buf.length < 2) return null;
  const n = buf[1];
  if (n === 0 || buf.length < 2 + n) return null;
  const tab = decodeId(buf.subarray(2, 2 + n));
  if (!tab) return null;
  let at = 2 + n;
  if (buf[0] === LIVE) {
    if (buf.length < at + 2) return null;
    const k = view.getUint16(at);
    if (k === 0 || k > BINARY_SEGMENTS_MAX) return null;
    at += 2;
    if (buf.length < at + k * 12) return null;
    const heads: [number, number][] = [];
    for (let i = 0; i < k; i++) {
      const seq = getU64(view, at);
      const len = view.getUint32(at + 8);
      if (!Number.isSafeInteger(seq)) return null;
      heads.push([seq, len]);
      at += 12;
    }
    const segments: Segment[] = [];
    for (const [seq, len] of heads) {
      if (buf.length < at + len) return null;
      segments.push({ seq, bytes: buf.subarray(at, at + len) });
      at += len;
    }
    if (at !== buf.length) return null;
    return { kind: LIVE, tab, segments };
  }
  if (buf[0] === SNAPSHOT) {
    if (buf.length < at + 1) return null;
    const m = buf[at];
    if (m === 0 || buf.length < at + 1 + m + 9) return null;
    const to = decodeId(buf.subarray(at + 1, at + 1 + m));
    if (!to) return null;
    at += 1 + m;
    const seq = getU64(view, at);
    if (!Number.isSafeInteger(seq) || (buf[at + 8] !== 0 && buf[at + 8] !== 1)) return null;
    return { kind: SNAPSHOT, tab, to, seq, more: buf[at + 8] === 1, bytes: buf.subarray(at + 9) };
  }
  return null;
}

/* invites */

/** V4 never accepts plaintext content, even from an authenticated client. */
export function isEncryptedUp(frame: Up): boolean {
  switch (frame.t) {
    case "share": {
      const s = frame.share;
      return !!s.encrypted && s.audience !== null && s.title === "" && s.repo_name === "" && s.branch === "" && s.stage === "" &&
        s.issue === null && s.tabs.every(tab => tab.title === "" && tab.note === null && tab.tokens === null && tab.status === "desligada");
    }
    case "note": return !!frame.encrypted && frame.text === "" && frame.quote === null && !frame.anchor;
    case "note_reply": return !!frame.encrypted && frame.text === "";
    case "write": return !!frame.encrypted && frame.data === "";
    case "note_resolve": return !!frame.encrypted;
    default: return true;
  }
}

/** Outer snapshot header only routes an opaque, unicast encrypted inner frame. */
export function encryptedBinary(data: ArrayBuffer | Uint8Array): { tab: string; to: string; encrypted: Encrypted } | null {
  const frame = decodeBinary(data);
  if (!frame || frame.kind !== SNAPSHOT || frame.seq !== 0 || frame.more) return null;
  try {
    const encrypted = parseEncrypted(JSON.parse(dec.decode(frame.bytes)));
    if (!encrypted || Object.keys(encrypted.boxes).length !== 1 || !encrypted.boxes[frame.to]) return null;
    return { tab: frame.tab, to: frame.to, encrypted };
  } catch { return null; }
}
export const isEncryptedBinary = (data: ArrayBuffer | Uint8Array): boolean => encryptedBinary(data) !== null;

/** Do not send ciphertext addressed to other devices in snapshots or broadcasts. */
export function downForMember(frame: Down, member: string): Down {
  const addressed = <T extends { encrypted?: Encrypted }>(value: T): T | null => {
    const box = value.encrypted?.boxes[member];
    return box ? { ...value, encrypted: { id: value.encrypted!.id, boxes: { [member]: box } } } : null;
  };
  const note = (n: Note): Note | null => {
    const result = addressed(n);
    if (!result) return null;
    if (n.resolution) {
      const resolution = addressed(n.resolution);
      if (resolution) result.resolution = resolution;
      else { delete result.resolution; result.resolved = false; }
    }
    return result;
  };
  const compact = <T>(list: (T | null)[]): T[] => list.filter((item): item is T => item !== null);
  switch (frame.t) {
    case "welcome": return { ...frame, shares: compact(frame.shares.map(addressed)), inbox: compact(frame.inbox.map(addressed)) };
    case "share": { const share = addressed(frame.share); return share ? { ...frame, share } : { t: "unshare", ws: frame.share.id }; }
    case "note": { const item = note(frame.note); return item ? { t: "note", note: item } : { t: "error", code: "noNote" }; }
    case "notes": return { ...frame, items: compact(frame.items.map(note)) };
    case "inbox": return { ...frame, items: compact(frame.items.map(addressed)) };
    case "write": return addressed(frame) ?? { t: "error", code: "noShare" };
    default: return frame;
  }
}

/// The pm2.<team>.<secret> invite format identifies individual enrollment so obsolete codes receive a clear compatibility error.
export function formatInvite(team: string, secret: string): string {
  return `pm2.${team}.${secret}`;
}

export function parseInvite(code: string): { team: string; secret: string } | null {
  const m = /^pm2\.([A-Za-z0-9_-]{8,64})\.([A-Za-z0-9_-]{16,128})$/.exec(code.trim());
  return m ? { team: m[1], secret: m[2] } : null;
}
