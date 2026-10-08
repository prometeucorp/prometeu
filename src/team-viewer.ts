import { SNAPSHOT, decodeBinary, type Down, type Shared } from "../relay/src/protocol";
import { t } from "./i18n";
import { AttachLifecycle } from "./attach-lifecycle";
import { Mirror } from "./mirror";
import { personOf } from "./team-channel";
import type { Context, Feature, GuestSink } from "./team-ports";
import { viewerRights, type Right } from "./team-rights";
import type { ShareRights } from "./types";

/// Viewer feature: list shares owned by other members, attach to one tab at a time, mirror its transcript
/// and forward input to the online owner. Any member client installs it.

/// After snapshot timeout, open with the available mirror if the owner disappeared.
const SNAPSHOT_WAIT = 10_000;
/// Prefix remote workspace IDs to avoid collisions with local board identities and accidental backend routing. Retain relay IDs separately.
const PREFIX = "@time:";

let ctx: Context;
let guest: GuestSink = { live: () => {}, reset: () => {} };
export const setSink = (sink: GuestSink) => void (guest = sink);

const remoteIds = new Map<string, { ws: string; owner: string }>();
const remoteId = (owner: string, ws: string) => `${PREFIX}${owner}/${ws}`;
/// The currently displayed remote tab, if any.
let attached: { ws: string; tab: string } | null = null;
/// Cache each viewed remote transcript through mirror.ts, which merges snapshot and live data independently of UI/network.
const mirror = new Map<string, Mirror>();
const attachLife = new AttachLifecycle();

export function install(context: Context): Feature {
  ctx = context;
  return { frame, binary, closed, reset };
}

/* Feature hooks. */

function frame(down: Down) {
  switch (down.t) {
    case "welcome":
      if (attached) ctx.send({ t: "attach", ws: attached.ws, tab: attached.tab });
      break;
    case "unshare":
      for (const [id, r] of remoteIds) if (r.ws === down.ws) remoteIds.delete(id);
      if (attached?.ws === down.ws) {
        attachLife.cancel();
        attached = null;
      }
      break;
  }
}

function binary(data: ArrayBuffer) {
  const bin = decodeBinary(data);
  if (!bin) return;
  if (bin.kind === SNAPSHOT) {
    if (bin.to !== ctx.you()) return;
    // Buffer intermediate snapshot chunks until completion.
    if (!mirrorFor(bin.tab).seed(bin.bytes, bin.seq, bin.more)) return;
    const completed = attachLife.completeTab(bin.tab);
    if (!completed && attached?.tab === bin.tab) {
      // An unsolicited recovery snapshot replaces the current view.
      guest.reset(bin.tab, mirrorOf(bin.tab));
    }
    return;
  }
  const m = mirrorFor(bin.tab);
  for (const seg of bin.segments) {
    const fresh = m.absorb(seg.seq, seg.bytes);
    if (fresh && attached?.tab === bin.tab) guest.live(bin.tab, fresh);
  }
}

function closed() {
  attachLife.completeCurrent();
}

function reset() {
  attachLife.cancel();
  attached = null;
  mirror.clear();
  remoteIds.clear();
}

/* Remote shares. */

/// Shares owned by other members, keyed by a prefixed ID that cannot collide with local identities.
export function remotes(): { id: string; share: Shared }[] {
  const out: { id: string; share: Shared }[] = [];
  const you = ctx.you();
  for (const s of ctx.shares().values()) {
    if (s.owner === you) continue;
    const id = remoteId(s.owner, s.id);
    remoteIds.set(id, { ws: s.id, owner: s.owner });
    out.push({ id, share: s });
  }
  return out;
}

/// Determine remoteness from the stable prefix, independent of transient share presence.
export const isRemote = (id: string) => id.startsWith(PREFIX);

/// Translate prefixed IDs back to relay identities; local IDs pass through.
export const relayId = (id: string) => remoteIds.get(id)?.ws ?? id;

/// Translate a relay identity to its prefixed ID when it belongs to a known remote share.
export const boardId = (ws: string) => [...remoteIds].find(([, r]) => r.ws === ws)?.[0] ?? ws;

/// The rights a remote owner announced for one of its shares; undefined when that owner predates rights (ADR 0090).
export function announcedRights(id: string): ShareRights | undefined {
  const found = remoteIds.get(id);
  return found && ctx.channel()?.rightsOf(found.ws);
}

/// What this member may do in a remote conversation, so the interface hides what the owner would discard anyway.
export function rightsIn(id: string): Record<Right, boolean> {
  const found = remoteIds.get(id);
  const s = found && ctx.shares().get(found.ws);
  const you = ctx.you();
  if (!s || !you) return { send: false, control: false };
  const members = ctx.members(), person = personOf(members, you);
  return viewerRights(ctx.channel()?.rightsOf(s.id), person, personOf(members, s.owner) === person);
}

/// Use the active remote attachment to route input; tab ID alone does not identify its owner.
export const attachedTab = () => attached?.tab ?? null;

/// Attach one remote tab at a time, releasing the previous watcher and waiting for its snapshot.
export async function attach(id: string, tab: string): Promise<{ bytes: Uint8Array } | null> {
  const found = remoteIds.get(id);
  const s = found && ctx.shares().get(found.ws);
  if (!s) throw t("err.team.noShare");
  attached = { ws: s.id, tab };
  // Invalidate old waits even when the new destination is already offline.
  if (!s.online) attachLife.cancel();
  if (s.online) {
    const ticket = attachLife.start(s.id, tab, SNAPSHOT_WAIT);
    if (!ctx.send({ t: "attach", ws: s.id, tab })) {
      attachLife.cancel();
      return attached?.ws === s.id && attached.tab === tab ? { bytes: mirrorOf(tab) } : null;
    }
    await ticket.wait;
    // Stop stale attachment continuations after detach or newer navigation, without touching ChatView.
    if (!attachLife.current(ticket) || attached?.ws !== s.id || attached.tab !== tab) return null;
  }
  return { bytes: mirrorOf(tab) };
}

export function detach() {
  attachLife.cancel();
  if (attached) ctx.send({ t: "detach" });
  attached = null;
}

/// Forward viewer input or JSON control to the online owner of the currently attached tab.
export async function write(data: string): Promise<boolean> {
  if (!attached) return false;
  const s = ctx.shares().get(attached.ws);
  if (!s) return false;
  if (!s.online) {
    ctx.fail(t("err.team.offline"));
    return false;
  }
  return ctx.sendConfirmed({ t: "write", ws: s.id, tab: attached.tab, data });
}

function mirrorOf(tab: string): Uint8Array {
  return mirror.get(tab)?.bytes() ?? new Uint8Array(0);
}

function mirrorFor(tab: string): Mirror {
  let m = mirror.get(tab);
  if (!m) {
    m = new Mirror();
    mirror.set(tab, m);
  }
  return m;
}
