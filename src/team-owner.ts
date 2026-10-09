import { SNAPSHOT, decodeBinary, encodeLive, encodeSnapshot, type Down, type Segment, type Share, type Up, type Watching } from "../relay/src/protocol";
import { t } from "./i18n";
import { personOf, type TeamChannel } from "./team-channel";
import { remoteControl as parseRemoteControl, rightOf } from "./team-control";
import type { Context, Feature, Gate, OwnerHost } from "./team-ports";
import { NO_RIGHTS, withGrant, type Right } from "./team-rights";
import type { Board, ShareRights, Workspace } from "./types";

/// Owner feature: announce shared local workspaces, stream their conversations to authorized viewers and run
/// remote input on the local agent, holding teammates' messages for the owner. Only the machine that executes agents
/// installs it.

/// Split retained transcripts into whole-line chunks, leaving room for encryption and encoding within the relay's 1 MiB binary frame limit.
const SNAPSHOT_PART = 128 * 1024;
/// Batch output for 40 ms to reduce relay message count without perceptible display delay.
const COALESCE = 40;
const FRAME_MAX = 32 * 1024;
/// Supply legacy geometry fields required by the protocol; conversation rendering ignores them.
const NO_SIZE: [number, number] = [80, 24];
/// Frames that carry content for a workspace and must wait while its audience changes.
const CONTENT = new Set<Up["t"]>(["write", "note", "note_reply", "note_resolve"]);
const enc = new TextEncoder();

let ctx: Context;
let host: OwnerHost;

/// Retain the latest local board for announcements and reconnect recovery.
let lastBoard: Board | null = null;
/// Send serialized share descriptions only when they change.
const announced = new Map<string, string>();
/// Track local-tab viewers reported by the relay.
const watchers = new Map<string, string[]>();
/// Batch pending outgoing bytes per tab.
const queue = new Map<string, Segment[]>();
let queued = 0;
let flushTimer = 0;
/// Hold live output while its snapshot is being sent to preserve ordering.
const holding = new Map<string, number>();
const accessVersions = new Map<string, number>();
type Access = { audience: string[] | null; remoteControl: boolean; rights: ShareRights };
const pendingAccess = new Map<string, Access | false>();
/// Shares from before rights existed keep `null` and let people only view and comment (ADR 0090).
const rightsOf = (w: Workspace | undefined): ShareRights => w?.rights ?? NO_RIGHTS;

export function install(context: Context, actions: OwnerHost): Feature {
  ctx = context;
  host = actions;
  return { connecting, outgoing, frame, closed, reset };
}

/* Feature hooks. */

function connecting(channel: TeamChannel) {
  for (const workspace of lastBoard?.workspaces ?? []) {
    if (sharedHere(workspace) && !workspace.remote && !workspace.archived && !workspace.cleaned) {
      channel.own(toShare(workspace), workspace.remote_control, rightsOf(workspace));
    }
  }
}

/// Content for a workspace waits out an audience change; share and unshare invalidate everything queued before them.
function outgoing(frame: Up): Gate | null {
  const ws = frame.t === "share" ? frame.share.id : "ws" in frame ? frame.ws : undefined;
  if (!ws) return null;
  const content = CONTENT.has(frame.t);
  if (content && pendingAccess.has(ws)) return () => false;
  if (frame.t === "share" || frame.t === "unshare") accessVersions.set(ws, (accessVersions.get(ws) ?? 0) + 1);
  const version = accessVersions.get(ws);
  return () => version === accessVersions.get(ws) && !(content && pendingAccess.has(ws));
}

function frame(down: Down) {
  switch (down.t) {
    case "welcome": {
      // After welcome, reannounce local shares and resnapshot their viewers before other requests so the relay knows their identities.
      announced.clear();
      const membership = ctx.membership();
      if (membership && !membership.legacy && lastBoard) {
        for (const share of down.shares) {
          const local = lastBoard.workspaces.find(w => w.id === share.id);
          if (share.owner === down.you && local && (!sharedHere(local) || local.archived || local.cleaned)) {
            ctx.send({ t: "unshare", ws: share.id });
          }
        }
      }
      if (lastBoard) boardChanged(lastBoard);
      rewatch(down.watching);
      break;
    }
    case "presence":
      announced.clear();
      if (lastBoard) boardChanged(lastBoard);
      break;
    case "watch":
      watched(down.tab, down.members, down.added);
      break;
    case "write":
      typed(down.ws, down.tab, down.data, down.from);
      break;
  }
}

function closed() {
  announced.clear();
  watchers.clear();
  holding.clear();
  queue.clear();
  queued = 0;
  clearTimeout(flushTimer);
  flushTimer = 0;
}

/// Held messages outlive a dropped connection, but not the membership their workspace was shared through.
function reset() {
  closed();
  accessVersions.clear();
  pendingAccess.clear();
  held = [];
}

/* Announcements. */

function toShare(w: Workspace): Share {
  return {
    id: w.id,
    title: w.title,
    repo_name: w.repo_name,
    branch: w.branch,
    stage: w.stage,
    issue: w.issue ? { identifier: w.issue.identifier, title: w.issue.title, url: w.issue.url } : null,
    active: w.active,
    tabs: w.tabs.map((tab) => ({ id: tab.id, title: tab.title, status: tab.status, note: tab.note, tokens: tab.tokens })),
    sizes: Object.fromEntries(w.tabs.map((tab) => [tab.id, NO_SIZE])),
    audience: w.audience,
  };
}

/// Announce changed eligible local shares and withdraw archived, cleaned, or removed workspaces.
export function boardChanged(board: Board) {
  lastBoard = board;
  dropStale();
  if (!ctx.membership()) return;
  const seen = new Set<string>();
  for (const w of board.workspaces) {
    if (!sharedHere(w) || w.remote) continue;
    if (w.archived || w.cleaned) {
      void host.setShared(w.id, false, null, false, null, null);
      continue;
    }
    seen.add(w.id);
    const share = toShare(w);
    const json = JSON.stringify([share, w.remote_control, rightsOf(w)]);
    if (announced.get(w.id) === json) continue;
    ctx.channel()?.own(share, w.remote_control, rightsOf(w));
    if (ctx.send({ t: "share", share })) announced.set(w.id, json);
  }
  for (const id of [...announced.keys()]) {
    if (seen.has(id)) continue;
    announced.delete(id);
    ctx.send({ t: "unshare", ws: id });
    for (const tab of [...watchers.keys()]) if (!tabOwnedBy(tab, seen)) watchers.delete(tab);
  }
}

const tabOwnedBy = (tab: string, ids: Set<string>) =>
  !!lastBoard?.workspaces.some((w) => ids.has(w.id) && w.tabs.some((t) => t.id === tab));

/// Require the tab to belong to a currently announced local workspace.
const mine = (tab: string) => tabOwnedBy(tab, new Set(announced.keys()));

/// Local audiences list people; a companion device is admitted through the person it belongs to.
function admitted(audience: string[] | null, remoteControl: boolean, member: string): boolean {
  const owner = ctx.you();
  if (member === owner) return true;
  const members = ctx.members();
  const person = personOf(members, member);
  if (owner && person === personOf(members, owner)) return remoteControl;
  return !audience || audience.includes(member) || audience.includes(person);
}

/// The access in force for a workspace, including a change still being saved; false while sharing stops.
function accessOf(w: Workspace): Access | false {
  return pendingAccess.get(w.id) ?? { audience: w.audience, remoteControl: w.remote_control, rights: rightsOf(w) };
}

const workspaceOf = (tab: string) => lastBoard?.workspaces.find(w => w.tabs.some(t => t.id === tab));

function canReceive(tab: string, member: string): boolean {
  const w = workspaceOf(tab);
  if (!w || !sharedHere(w) || w.archived || w.cleaned || !ctx.channel()?.security.key(member)) return false;
  const access = accessOf(w);
  return !!access && admitted(access.audience, access.remoteControl, member);
}

/// Viewing never grants acting. The owner's devices act through remote control; anyone else needs the right on their
/// person, and the whole organization never holds one (ADR 0090).
function entitled(access: Access, member: string, right: Right): boolean {
  const owner = ctx.you();
  if (member === owner) return true;
  const members = ctx.members();
  const person = personOf(members, member);
  if (owner && person === personOf(members, owner)) return access.remoteControl;
  return access.rights[right].includes(person);
}

/// Share with everyone using null, selected people using IDs, or stop team sharing using false. The audience views and
/// comments; people who stop viewing lose their rights.
export async function share(id: string, audience: string[] | null | false) {
  const workspace = lastBoard?.workspaces.find(w => w.id === id);
  const teamAudience = audience !== false && (audience === null || audience.length > 0) ? audience : [];
  await setAccess(id, teamAudience, workspace?.remote_control ?? false, rightsOf(workspace));
}

/// Grant or revoke Send messages or Control for one person. Acting needs viewing, so an explicit audience gains the
/// person; people who left the organization drop out of that right; granting approves the devices the person has now
/// (ADR 0090). Resolves how many became approved.
export async function grant(id: string, person: string, right: Right, on: boolean): Promise<number> {
  const workspace = lastBoard?.workspaces.find(w => w.id === id);
  if (!workspace) return 0;
  const current = rightsOf(workspace), members = ctx.members();
  const rights = { ...current, [right]: withGrant(current[right], person, on, new Set(members.map(m => personOf(members, m.id)))) };
  const audience = sharedWithTeam(workspace) ? workspace.audience : [];
  const generation = ctx.generation();
  await setAccess(id, !on || audience === null || audience.includes(person) ? audience : [...audience, person], workspace.remote_control, rights);
  return on ? approve([person], generation) : 0;
}

/// Toggle access for the owner's companion devices without changing the team audience. Turning it on approves the
/// devices the owner has now (ADR 0090); resolves how many became approved.
export async function remoteControl(id: string, enabled: boolean): Promise<number> {
  const workspace = lastBoard?.workspaces.find(w => w.id === id);
  if (!workspace) return 0;
  const audience = workspace.shared && (workspace.audience === null || workspace.audience.length > 0) ? workspace.audience : [];
  const generation = ctx.generation();
  await setAccess(id, audience, enabled, rightsOf(workspace));
  const you = ctx.you();
  return enabled && you ? approve([personOf(ctx.members(), you)], generation) : 0;
}

async function setAccess(id: string, audience: string[] | null, remoteControl: boolean, rights: ShareRights) {
  const on = remoteControl || audience === null || audience.length > 0;
  const membership = ctx.membership();
  if (on && !membership) throw t("err.team.noRelay");
  if (pendingAccess.has(id)) throw t("err.team.encryption");
  // Acting needs viewing: an explicit audience keeps only the rights of the people it names.
  const kept = audience === null ? rights
    : { send: rights.send.filter(p => audience.includes(p)), control: rights.control.filter(p => audience.includes(p)) };
  const generation = ctx.generation();
  pendingAccess.set(id, on ? { audience, remoteControl, rights: kept } : false);
  accessVersions.set(id, (accessVersions.get(id) ?? 0) + 1);
  try {
    await host.setShared(id, on, on ? audience : null, on && remoteControl, on ? membership!.shareScope : null, on ? kept : null);
    if (generation !== ctx.generation()) return;
    // IPC completion can precede the board event. Presence must not reannounce
    // the old audience during that gap.
    if (lastBoard) lastBoard = { ...lastBoard, workspaces: lastBoard.workspaces.map(w => w.id === id
      ? { ...w, shared: on, audience: on ? audience : null, remote_control: on && remoteControl, share_team: on ? membership!.shareScope : null,
          rights: on ? kept : null } : w) };
    dropStale();
    const w = lastBoard?.workspaces.find(w => w.id === id);
    if (w && on) {
      const updated = { ...toShare(w), audience };
      ctx.channel()?.own(updated, remoteControl, kept);
      if (ctx.phase() === "online" && !await ctx.sendConfirmed({ t: "share", share: updated })) throw t("err.team.encryption");
    } else if (!on && ctx.phase() === "online") {
      if (!await ctx.sendConfirmed({ t: "unshare", ws: id })) throw t("err.team.encryption");
    }
  } finally { if (generation === ctx.generation()) pendingAccess.delete(id); }
}

/// Expand an owned share's audience before mentioning a new member, so the relay accepts the mention that follows.
/// A mention lets that person view and comment, never act.
export async function includeMentioned(id: string, mentions: string[]) {
  const w = lastBoard?.workspaces.find((x) => x.id === id);
  if (w && sharedHere(w) && !w.remote && w.audience) {
    const missing = mentions.filter((m) => !w.audience!.includes(m));
    if (missing.length) await share(id, [...w.audience, ...missing]);
  }
}

/* Input approval: content follows a peer's adopted key, input waits for the owner (ADR 0090). */

/// A person whose discarded input in a local workspace awaits the owner's answer; `own` marks the owner's devices.
export type PausedInput = { person: string; name: string; own: boolean };

/// Approve the current keys of every device of these people; resolves how many became approved. Consent belongs to
/// the connection it was given on, so nothing is approved once the owner reconnected or switched organizations.
async function approve(people: string[], generation = ctx.generation()): Promise<number> {
  if (generation !== ctx.generation()) return 0;
  const members = ctx.members();
  const devices = members.filter(m => people.includes(personOf(members, m.id))).map(m => m.id);
  if (!devices.length) return 0;
  return await ctx.secure(security => security.approve(devices)) ?? 0;
}

/// People with discarded input in this workspace who can still act there, named as people, never devices (ADR 0041).
export function pausedIn(id: string): PausedInput[] {
  const security = ctx.channel()?.security;
  const w = lastBoard?.workspaces.find(w => w.id === id);
  const access = w && accessOf(w);
  if (!security || !w || !access || w.remote || !sharedHere(w)) return [];
  const you = ctx.you(), members = ctx.members();
  const people = new Set<string>();
  for (const [member, workspaces] of Object.entries(security.paused())) {
    if (workspaces.includes(id) && admitted(access.audience, access.remoteControl, member) &&
      (entitled(access, member, "send") || entitled(access, member, "control"))) people.add(personOf(members, member));
  }
  const self = you && personOf(members, you);
  return [...people].map(person => ({ person, name: ctx.nameOf(person), own: person === self }));
}

/// The owner's answer to the notice: input from the person's current devices runs again across the scope.
export async function allowInput(person: string): Promise<number> {
  const count = await approve([person]);
  ctx.changed();
  return count;
}

/// Hide the notice without approving; the next discarded input from that person shows it again.
export async function dismissInput(person: string) {
  const members = ctx.members();
  const devices = members.filter(m => personOf(members, m.id) === person).map(m => m.id);
  await ctx.secure(security => security.dismiss(devices));
  ctx.changed();
}

export const sharedHere = (workspace: Workspace) => {
  const membership = ctx.membership();
  return !!membership && workspace.shared &&
    (workspace.share_team ? workspace.share_team === membership.shareScope : membership.legacy);
};

export const sharedWithTeam = (workspace: Workspace) =>
  sharedHere(workspace) && (workspace.audience === null || workspace.audience.length > 0);

export const isShared = (id: string) => announced.has(id);
export const watchersOf = (tab: string): string[] => [...new Set((watchers.get(tab) ?? []).map(ctx.nameOf))];

/* Viewers and streaming. */

function watched(tab: string, who: string[], added: string[]) {
  if (!mine(tab)) return;
  who = who.filter(member => canReceive(tab, member));
  added = added.filter(member => canReceive(tab, member));
  if (who.length) watchers.set(tab, who);
  else watchers.delete(tab);
  for (const member of added) void snapshot(tab, member);
}

/// Resend full snapshots to existing viewers after owner reconnection.
function rewatch(watching: Watching) {
  watchers.clear();
  for (const tabs of Object.values(watching)) {
    for (const [tab, raw] of Object.entries(tabs)) {
      const who = raw.filter(member => canReceive(tab, member));
      if (!who.length || !mine(tab)) continue;
      watchers.set(tab, who);
      for (const member of who) void snapshot(tab, member);
    }
  }
}

/// Send local snapshots in chunks while holding live output; otherwise a viewer awaiting its first snapshot could discard later lines.
async function snapshot(tab: string, member: string) {
  if (!mine(tab) || !canReceive(tab, member)) return;
  const generation = ctx.generation();
  holding.set(tab, (holding.get(tab) ?? 0) + 1);
  try {
    const s = await host.snapshot(tab);
    if (generation !== ctx.generation() || !mine(tab)) return;
    const parts = split(s.text);
    parts.forEach((part, i) => push(encodeSnapshot(tab, member, s.seq, enc.encode(part), i < parts.length - 1)));
  } catch {
    // If the session disappeared, the viewer opens its available mirror.
  } finally {
    if (generation === ctx.generation()) {
      const left = (holding.get(tab) ?? 1) - 1;
      if (left > 0) holding.set(tab, left);
      else holding.delete(tab);
      flush();
    }
  }
}

/// Split at complete lines within relay frame size; send one empty chunk for an empty conversation.
function split(text: string): string[] {
  const out: string[] = [];
  let rest = text;
  while (rest.length > SNAPSHOT_PART) {
    const cut = rest.lastIndexOf("\n", SNAPSHOT_PART);
    const at = cut === -1 ? SNAPSHOT_PART : cut + 1;
    out.push(rest.slice(0, at));
    rest = rest.slice(at);
  }
  out.push(rest);
  return out;
}

/// Ignore output for unwatched tabs with a cheap map lookup.
export function output(key: string, line: string, seq: number) {
  if (!watchers.has(key) || !mine(key)) return;
  const list = queue.get(key) ?? [];
  const bytes = enc.encode(line + "\n");
  list.push({ seq, bytes });
  queue.set(key, list);
  queued += bytes.length;
  if (queued >= FRAME_MAX) flush();
  else if (!flushTimer) flushTimer = setTimeout(flush, COALESCE);
}

function flush() {
  clearTimeout(flushTimer);
  flushTimer = 0;
  for (const [tab, segments] of queue) {
    if (holding.has(tab)) continue;
    queue.delete(tab);
    queued -= segments.reduce((n, s) => n + s.bytes.length, 0);
    if (!watchers.has(tab) || !mine(tab)) continue;
    push(encodeLive(tab, segments));
  }
  if (queue.size && !flushTimer) flushTimer = setTimeout(flush, COALESCE);
}

/// Encrypt a local binary frame for the viewers still authorized when it leaves the queue.
function push(data: Uint8Array): boolean {
  const inner = decodeBinary(data);
  if (!inner) return false;
  const ws = lastBoard?.workspaces.find(w => w.tabs.some(t => t.id === inner.tab))?.id;
  const version = ws ? accessVersions.get(ws) : undefined;
  const valid = () => mine(inner.tab) && (!ws || version === accessVersions.get(ws))
    && (inner.kind !== SNAPSHOT || canReceive(inner.tab, inner.to));
  return ctx.sendBinary(data, () => (watchers.get(inner.tab) ?? []).filter(member => canReceive(inner.tab, member)), valid);
}

/// Revalidate remote input and control against announced local tabs before writing to a real process, even though the relay already filters it.
function typed(ws: string, tab: string, data: string, from: string) {
  const w = workspaceOf(tab);
  const access = w && accessOf(w);
  if (!announced.has(ws) || !mine(tab) || !w || !access || !canReceive(tab, from)) return;
  const parsed = parseRemoteControl(data);
  if (parsed.recognized && !parsed.frame) return;
  // Input is execution on this Mac. It needs the right for its kind, and then a member and key the owner approved;
  // only the latter leaves a notice. The channel already spent this message's replay receipt, so discarded input
  // can never run later (ADR 0090).
  if (!entitled(access, from, rightOf(parsed))) return;
  if (!ctx.channel()?.security.approved(from)) {
    void ctx.secure(security => security.pause(from, ws)).then(added => { if (added) ctx.changed(); }).catch(() => {});
    return;
  }
  if (parsed.recognized) {
    void host.control(tab, parsed.frame).catch(() => {});
    return;
  }
  const you = ctx.you(), members = ctx.members();
  if (you && personOf(members, from) === personOf(members, you)) {
    void host.prompt(tab, data).catch(() => {});
    return;
  }
  // A teammate's message waits for the owner unless this workspace turned confirmation off; interrupts, answers and
  // the owner's devices never wait (ADR 0090).
  if (w.confirm_messages !== false) return hold(w.id, tab, from, data);
  void host.prompt(tab, t("team.remotePrompt", { name: ctx.nameOf(from), text: data })).catch(() => {});
}

/* Owner confirmation: a teammate's message waits in the owner's composer (ADR 0090). */

/// A teammate's message waiting on this Mac, named by the sender's person rather than the device (ADR 0041).
export type PendingMessage = { id: string; name: string; text: string };
/// `author` names the sending device in the team prefix, exactly as a message that runs at once.
type Held = PendingMessage & { ws: string; tab: string; person: string; author: string };
/// Bounds what one flooding sender can hold in memory; a conversation drops its oldest message first.
const HELD_MAX = 20;
/// Only in memory and in arrival order: there is no offline inbox, and a third party's content never reaches disk.
let held: Held[] = [];
let heldIds = 0;

function hold(ws: string, tab: string, from: string, text: string) {
  const person = personOf(ctx.members(), from);
  held.push({ id: String(++heldIds), name: ctx.nameOf(person), text, ws, tab, person, author: ctx.nameOf(from) });
  const here = held.filter(m => m.tab === tab);
  if (here.length > HELD_MAX) held = held.filter(m => m !== here[0]);
  ctx.changed();
}

/// A held message lasts while its workspace stays shared here with that conversation and its sender keeps Send
/// messages, including a change of access still being saved.
function holds(m: Held): boolean {
  const w = lastBoard?.workspaces.find(w => w.id === m.ws);
  if (!w || w.remote || !sharedHere(w) || w.archived || w.cleaned || !w.tabs.some(t => t.id === m.tab)) return false;
  const access = accessOf(w);
  return !!access && access.rights.send.includes(m.person);
}

function dropStale() {
  const kept = held.filter(holds);
  if (kept.length === held.length) return;
  held = kept;
  ctx.changed();
}

/// Messages waiting in a local conversation, oldest first.
export const pendingIn = (tab: string): PendingMessage[] =>
  held.filter(m => m.tab === tab && holds(m)).map(({ id, name, text }) => ({ id, name, text }));

/// Take a message out of the wait; null when it no longer exists or no longer holds.
function take(id: string): Held | null {
  const at = held.findIndex(m => m.id === id);
  if (at === -1) return null;
  const [m] = held.splice(at, 1);
  ctx.changed();
  return holds(m) ? m : null;
}

/// Send a held message as its sender's, with the team prefix. Resolves false when it is gone; a failed send keeps it
/// waiting so the owner can try again.
export async function sendPending(id: string): Promise<boolean> {
  const at = held.findIndex(m => m.id === id);
  const m = take(id);
  if (!m) return false;
  try {
    await host.prompt(m.tab, t("team.remotePrompt", { name: m.author, text: m.text }));
  } catch (error) {
    if (holds(m)) { held.splice(Math.min(at, held.length), 0, m); ctx.changed(); }
    throw error;
  }
  return true;
}

/// Hand a held message to the owner's composer: whatever the owner sends from it is the owner's own message.
export const editPending = (id: string): string | null => take(id)?.text ?? null;

export function discardPending(id: string) {
  take(id);
}
