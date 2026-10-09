import { DOWN_FRAME_MAX, parseDown, parseUp, type Down, type Member, type Shared, type Up } from "../relay/src/protocol";
import { fromBack, t } from "./i18n";
import { TeamChannel, personOf as personIn } from "./team-channel";
import { TeamSecurity } from "./team-security";
import { defaultTransport, type SocketLike, type Transport } from "./team-transport";
import type { Context, Feature, Gate, Membership, Phase, SecurityStore } from "./team-ports";

/// One reconnecting relay connection per membership: identity handshake, encrypted send queue and the
/// member directory. Features register hooks for everything else; the shell supplies membership and storage.

const enc = new TextEncoder();

/* Ports. */

let transport: Transport = defaultTransport();
export function useTransport(next: Transport) {
  transport = next;
}
export const currentTransport = () => transport;

/// Connecting without a store must fail visibly instead of minting a throwaway identity.
let store: SecurityStore = {
  read: async () => { throw new Error("Security store not configured"); },
  write: async () => { throw new Error("Security store not configured"); },
};
export function useSecurityStore(next: SecurityStore) {
  store = next;
}

const features: Feature[] = [];
export function register(install: (context: Context) => Feature) {
  features.push(install(ctx));
}

/* State. */

let membership: Membership | null = null;
let connection = 0;
let channel: TeamChannel | null = null;
let wireQueue: Promise<void> = Promise.resolve();
let queuedWireBytes = 0;
let identityLoading: Promise<unknown> = Promise.resolve();
let phase: Phase = "off";
let sock: SocketLike | null = null;
let you: string | null = null;
let members: Member[] = [];
let shares = new Map<string, Shared>();
let attempt = 0;
let retry = 0;
let pinger = 0;
let renewer = 0;

const listeners = new Set<() => void>();
export const onChange = (cb: () => void) => {
  listeners.add(cb);
  return () => void listeners.delete(cb);
};
const changed = () => listeners.forEach((cb) => cb());
/// Let the shell redraw after it changes state the member does not own.
export const notify = changed;

let fail: ((text: string) => void) | null = null;
export const onError = (cb: (text: string) => void) => void (fail = cb);
export const report = (text: string) => void fail?.(text);

export const current = () => ({ phase, you, members, membership });
export const nameOf = (member: string) => members.find((m) => m.id === member)?.name ?? member.slice(0, 8);
/// The person this device acts for; a second Mac of the same account is a companion of its first Mac (ADR 0036).
export const personOf = (id: string | null): string | null => id === null ? null : personIn(members, id);
/// One entry per person for audience and mention choices: companion devices fold into their primary member.
export const people = (): Member[] => members.filter((m) => !m.person)
  .map((m) => ({ ...m, online: m.online || members.some((d) => d.person === m.id && d.online) }));
/// No socket and no pending retry: the shell may start a connection.
export const idle = () => !sock && !retry;

function encryptedWork(bytes: number, work: () => Promise<void>) {
  if (queuedWireBytes + bytes > 16 * 1024 * 1024) {
    fail?.(t("err.team.encryption")); return false;
  }
  const generation = connection;
  queuedWireBytes += bytes;
  wireQueue = wireQueue.then(async () => {
    if (generation === connection) await work();
  }).catch(() => {
    if (generation === connection) fail?.(t("err.team.encryption"));
  }).finally(() => { queuedWireBytes -= bytes; });
  return true;
}

/* Lifecycle. */

/// Adopt a membership and connect; the member reconnects by itself until disconnect or reset.
export async function connect(next: Membership) {
  membership = next;
  attempt = 0;
  await reconnect();
}

async function reconnect() {
  const m = membership;
  if (!m) return;
  retry = 0;
  const generation = ++connection;
  // The store is about to be read again: a write through the old channel would miss the replacement and be erased by
  // its next write, so answers given meanwhile do nothing instead.
  channel = null;
  try {
    await wireQueue;
    const loading = identityLoading.catch(() => {}).then(() => TeamSecurity.load(m.privateScope, store.read, store.write));
    identityLoading = loading;
    const security = await loading;
    if (generation !== connection) return;
    channel = new TeamChannel(security, m.scope, m.member);
    for (const feature of features) feature.connecting?.(channel);
  } catch (e) {
    // Invalid private storage blocks collaboration until repaired; retrying cannot fix it.
    if (generation !== connection) return;
    phase = "off";
    fail?.(fromBack(e));
    changed();
    return;
  }
  phase = "connecting";
  changed();
  let url: string | null;
  try {
    url = await m.url();
    if (generation !== connection) return;
  } catch (e) {
    // Tickets depend on the Cloud; keep trying with backoff.
    if (generation !== connection) return;
    phase = "off";
    fail?.(fromBack(e));
    changed();
    retry = setTimeout(() => void reconnect(), backoff());
    return;
  }
  if (url === null) {
    phase = "off";
    changed();
    return;
  }
  let s: SocketLike;
  try {
    s = transport.socket(url);
  } catch (error) {
    phase = "off";
    fail?.(t("err.team.relay", { cause: String(error) }));
    changed();
    return;
  }
  s.binaryType = "arraybuffer";
  sock = s;
  // The same ticket port serves desktop and browser renewals. Never move a ticket to a different relay or room.
  const renew = async () => {
    if (sock !== s || generation !== connection) return;
    try {
      const next = await m.url();
      if (sock !== s || generation !== connection) return;
      if (!next) { s.close(); return; }
      const endpoint = new URL(next), original = new URL(url);
      const frame = parseUp({ t: "renew", ticket: endpoint.searchParams.get("ticket") });
      if (endpoint.origin !== original.origin || endpoint.pathname !== original.pathname || !frame) {
        s.close(); return;
      }
      s.send(JSON.stringify(frame));
    } catch {
      // A temporary Cloud failure can recover while the current lease remains valid.
      if (sock === s && generation === connection) renewer = setTimeout(renew, 5_000);
    }
  };
  s.onopen = () => {
    if (sock !== s) return;
    attempt = 0;
    // Keep idle sockets alive without waking unnecessary relay work.
    pinger = setInterval(() => s.send("ping"), 30_000);
  };
  s.onmessage = (ev) => {
    if (sock !== s) return;
    if (typeof ev.data === "string") {
      if (ev.data === "pong") return;
      // Bound incoming JSON before parsing because a configurable relay may be hostile.
      if (ev.data.length > DOWN_FRAME_MAX || enc.encode(ev.data).byteLength > DOWN_FRAME_MAX) {
        s.close();
        return;
      }
      let raw: unknown;
      try {
        raw = JSON.parse(ev.data);
      } catch {
        return;
      }
      const frame = parseDown(raw);
      if (!frame) return;
      const activeChannel = channel;
      if (!activeChannel) return;
      encryptedWork(ev.data.length, async () => {
        const plain = await activeChannel.incoming(frame);
        if (sock !== s) return;
        if (frame.t === "welcome") {
          const identity = await activeChannel.identity(frame.challenge!);
          if (sock === s) s.send(JSON.stringify(identity));
        }
        if (plain?.t === "lease") {
          if (!m.legacy) {
            clearTimeout(renewer);
            renewer = setTimeout(renew, Math.max(1_000, Math.floor(plain.expires_in / 2)));
          }
          return;
        }
        if (plain) handle(plain);
        changed();
      });
    } else if (ev.data instanceof ArrayBuffer) {
      const data = ev.data;
      const activeChannel = channel;
      if (activeChannel) encryptedWork(data.byteLength, async () => {
        const plain = await activeChannel.incomingBinary(data);
        if (sock !== s) return;
        const bytes = plain.buffer.slice(plain.byteOffset, plain.byteOffset + plain.byteLength) as ArrayBuffer;
        for (const feature of features) feature.binary?.(bytes);
      });
    }
  };
  s.onclose = () => {
    if (sock !== s) return;
    sock = null;
    clearInterval(pinger);
    clearTimeout(renewer);
    members = members.map((m) => ({ ...m, online: false }));
    for (const sh of shares.values()) sh.online = false;
    for (const feature of features) feature.closed?.();
    phase = membership ? "connecting" : "off";
    changed();
    if (membership) retry = setTimeout(reconnect, backoff());
  };
  s.onerror = () => {
    // The following close event schedules reconnection.
  };
}

/// Reconnect with exponential backoff from one to thirty seconds and jitter to avoid synchronized retries.
function backoff(): number {
  const base = Math.min(30_000, 1000 * 2 ** attempt++);
  return Math.round(base * (0.8 + Math.random() * 0.4));
}

/// Close the socket and cancel queued work; the membership stays for a later connect.
export function disconnect() {
  connection++;
  clearTimeout(retry);
  retry = 0;
  clearInterval(pinger);
  clearTimeout(renewer);
  const s = sock;
  sock = null;
  s?.close();
  phase = "off";
  channel = null;
  for (const feature of features) feature.closed?.();
}

/// Forget the membership and every derived state.
export function reset() {
  disconnect();
  membership = null;
  you = null;
  members = [];
  shares = new Map();
  for (const feature of features) feature.reset?.();
}

/* Outgoing. */

export function send(frame: Up, completion?: (sent: boolean) => void): boolean {
  if (!sock || phase !== "online" || !channel) return false;
  const socket = sock, active = channel;
  const gates = features.map(f => f.outgoing?.(frame)).filter((gate): gate is Gate => !!gate);
  const open = () => gates.every(gate => gate());
  if (!open()) return false;
  return encryptedWork(JSON.stringify(frame).length, async () => {
    try {
      if (!open()) { completion?.(false); return; }
      const encrypted = await active.outgoing(frame);
      const valid = sock === socket && open();
      if (valid) socket.send(JSON.stringify(encrypted));
      completion?.(valid);
    } catch (error) { completion?.(false); throw error; }
  });
}

function sendConfirmed(frame: Up): Promise<boolean> {
  return new Promise(resolve => {
    const generation = connection;
    if (!send(frame, resolve)) resolve(false);
    // A canceled connection skips queued operations, including their callback.
    wireQueue.finally(() => { if (generation !== connection) resolve(false); });
  });
}

/// Reconnecting waits for this queue before loading the store again, so a write from an older connection can never
/// replace newer receipts, links or approvals.
function secure<T>(work: (security: TeamSecurity) => Promise<T>): Promise<T | null> {
  const active = channel, generation = connection;
  if (!active) return Promise.resolve(null);
  const result = wireQueue.then(() => generation === connection ? work(active.security) : null);
  wireQueue = result.then(() => {}, () => {});
  return result;
}

function sendBinary(data: Uint8Array, recipients: () => string[], valid: Gate): boolean {
  if (!sock || phase !== "online" || !channel) return false;
  const socket = sock, active = channel;
  return encryptedWork(data.length, async () => {
    if (!valid()) return;
    const encrypted = await active.outgoingBinary(data, recipients());
    if (sock === socket && valid()) for (const frame of encrypted) socket.send(frame);
  });
}

/* Incoming. */

/// Missing-share/tab errors often result from automatic refreshes; avoid displaying unsolicited errors for already-removed resources.
const QUIET = new Set(["noShare", "noTab"]);

function handle(frame: Down) {
  switch (frame.t) {
    case "welcome":
      you = frame.you;
      members = frame.members;
      shares = new Map(frame.shares.map((s) => [s.id, s]));
      phase = "online";
      break;
    case "presence":
      members = frame.members;
      break;
    case "share":
      shares.set(frame.share.id, frame.share);
      break;
    case "unshare":
      shares.delete(frame.ws);
      break;
    // Ignore legacy terminal-size frames; conversations no longer use PTY geometry.
    case "size":
      return;
    case "error": {
      // Uncorrelated missing-resource errors usually answer background requests. Let unshare reconcile the UI instead of showing unrelated header warnings.
      if (QUIET.has(frame.code)) return;
      // Fall back to a generic failure for unknown codes from newer relays.
      const key = `err.team.${frame.code}` as Parameters<typeof t>[0];
      const text = t(key);
      fail?.(text === key ? t("err.team.bad") : text);
      return;
    }
  }
  for (const feature of features) feature.frame?.(frame);
  // Remote input reaches the local agent without a directory change to redraw.
  if (frame.t !== "write") changed();
}

const ctx: Context = {
  send, sendConfirmed, sendBinary, secure, changed,
  fail: (text) => fail?.(text),
  generation: () => connection,
  phase: () => phase,
  you: () => you,
  members: () => members,
  shares: () => shares,
  membership: () => membership,
  channel: () => channel,
  nameOf,
};
