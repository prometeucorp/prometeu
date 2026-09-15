import { listen } from "@tauri-apps/api/event";
import { PROTO, formatInvite, normalizeName, parseInvite, parseMembership, type Member, type Note } from "../relay/src/protocol";
import type { CloudStatus } from "./cloud";
import { fromBack, t } from "./i18n";
import { invoke } from "./ipc";
import * as comments from "./team-comments";
import * as member from "./team-member";
import * as owner from "./team-owner";
import type { Membership, Phase } from "./team-ports";
import { wsUrl as transportWsUrl } from "./team-transport";
import * as viewer from "./team-viewer";
import type { Workspace } from "./types";

export type { SocketLike, Transport } from "./team-transport";
export type { GuestSink, Phase } from "./team-ports";

/// Desktop shell of the collaboration core: team.json configuration, Cloud organizations, Tauri ports and the
/// board-facing facade used by the interface. The portable core is team-member.ts plus the team-* features;
/// see ADR 0026.

export type TeamConfig = {
  /// Relay URL override.
  relay: string | null;
  team: string;
  secret: string;
  /// Individual member identity and proof issued during enrollment.
  member: string;
  credential: string;
  name: string;
  cloud?: { user: string; origin: string; slug: string; name: string };
};

export type Organization = { id: string; slug: string; name: string; member: string; role: "owner" | "admin" | "member" };

/* Composition: Tauri ports and the features this machine runs. Order decides hook order after welcome. */

member.useSecurityStore({
  read: () => invoke("team_security"),
  write: (state) => invoke("team_security_set", { state }),
});
member.register((ctx) => owner.install(ctx, {
  setShared: (id, shared, audience, remoteControl, team) => invoke("set_shared", { id, shared, audience, remoteControl, team }),
  snapshot: (tab) => invoke("chat_snapshot", { session: tab }),
  control: (tab, frame) => invoke("chat_control_remote", { session: tab, frame }),
  prompt: (tab, text) => invoke("chat_send", { session: tab, text }),
}));
member.register(comments.install);
member.register(viewer.install);

export const { useTransport, onChange, onError, nameOf, people, personOf } = member;
export const { setSink, isRemote, attachedTab, attach, detach, write } = viewer;
export const { boardChanged, share, remoteControl, sharedHere, sharedWithTeam, isShared, watchersOf } = owner;
export const { inboxCount, inboxItems, supportsThreads, inboxList } = comments;

/* Configuration. */

/// Use the deployed relay unless Settings or VITE_RELAY overrides it.
const RELAY = "wss://prometeu-relay.prometheus-capim.workers.dev";
const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env;

let organizations: Organization[] = [];
let account: CloudStatus = { user: null, origin: "", offline: false };
let organizationRequest = 0;
let cfg: TeamConfig | null = null;
let defaultName = "";
/// Retain a pre-enrollment relay override for eventual team.json persistence.
let relayDraft = "";

export type TeamStatus = {
  config: TeamConfig | null;
  phase: Phase;
  you: string | null;
  members: Member[];
  defaultName: string;
  /// Expose the configured override, application default, and effective relay URL.
  relay: string;
  relayDefault: string;
  relayEffective: string;
  organizations: Organization[];
  account: CloudStatus;
};

export const status = (): TeamStatus => {
  const { phase, you, members } = member.current();
  return {
    config: cfg,
    phase,
    you,
    members,
    defaultName,
    relay: cfg ? (cfg.relay ?? "") : relayDraft,
    relayDefault: env?.VITE_RELAY || RELAY,
    relayEffective: relayOf(cfg),
    organizations,
    account,
  };
};

export const invite = () => (cfg && !cfg.cloud ? formatInvite(cfg.team, cfg.secret) : null);

/* Lifecycle. */

export async function init() {
  try {
    const file = await invoke("team_config");
    defaultName = file.default_name;
    const stored = storedConfig(file.config);
    if (stored) relayDraft = stored.relay ?? "";
    if (stored?.credential || stored?.cloud) {
      cfg = { ...stored, credential: stored.credential ?? "" };
    } else if (stored) {
      // Exchange legacy v2 shared-secret identity for v3 enrollment without destroying old configuration if migration fails.
      const base = relayOf(stored);
      try {
        if (!base) throw new Error(t("err.team.noRelay"));
        const membership = await member.currentTransport().enroll(base, stored.team, stored.secret);
        cfg = { ...stored, ...membership };
        await invoke("team_config_set", { config: cfg });
      } catch {
        cfg = null;
        member.report(t("err.team.legacy"));
      }
    }
  } catch {
    // Keep the interface usable without team state if the backend is unavailable or older.
  }
  // Forward local conversation lines only for tabs with viewers.
  listen<[string, string, number]>("chat", ({ payload: [key, line, seq] }) => owner.output(key, line, seq));
  if (cfg && !cfg.cloud) await activate();
}

export async function refreshOrganizations(value: CloudStatus) {
  const request = ++organizationRequest;
  const identityChanged = account.user?.id !== value.user?.id || account.origin !== value.origin;
  account = value;
  if (identityChanged || !value.user) organizations = [];
  if (cfg?.cloud && (!value.user || cfg.cloud.user !== value.user.id || cfg.cloud.origin !== value.origin)) {
    member.reset(); cfg = null;
  }
  member.notify();
  if (!value.user || value.offline) return;
  try {
    const result = await invoke("cloud_organizations");
    if (request !== organizationRequest || result.user?.id !== account.user?.id || result.origin !== account.origin) return;
    organizations = result.organizations;
    if (cfg?.cloud) {
      const org = organizations.find(org => org.id === cfg!.team && org.member === cfg!.member);
      if (!org) { member.reset(); cfg = null; }
      else {
        cfg = { ...cfg, name: value.user.name, cloud: { ...cfg.cloud, slug: org.slug, name: org.name } };
        if (member.idle()) void activate();
      }
    }
    member.notify();
    // A single organization needs no choice; an explicit leave still wins over the shortcut.
    if (!cfg && organizations.length === 1 && organizations[0].id !== leftOrganization())
      await selectOrganization(organizations[0].id);
  } catch (error) {
    if (request === organizationRequest) member.report(fromBack(error));
  }
}

/// Local preference: the organization the person last left on this machine.
const LEFT_KEY = "prometeu:organizacao-saida";
const leftOrganization = () => globalThis.localStorage?.getItem(LEFT_KEY) ?? null;

export async function selectOrganization(id: string) {
  const org = organizations.find(item => item.id === id);
  if (!org || !account.user) throw t("err.cloud.response");
  globalThis.localStorage?.removeItem(LEFT_KEY);
  await adopt({ relay: null, team: org.id, secret: "", credential: "", member: org.member, name: account.user.name,
    cloud: { user: account.user.id, origin: account.origin, slug: org.slug, name: org.name } });
}

type StoredTeamConfig = Omit<TeamConfig, "credential"> & { credential?: string };

function storedConfig(value: unknown): StoredTeamConfig | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;
  if (raw.cloud && typeof raw.cloud === "object" && !Array.isArray(raw.cloud)) {
    const cloud = raw.cloud as Record<string, unknown>;
    if (typeof raw.team !== "string" || !/^[A-Za-z0-9_-]{8,64}$/.test(raw.team) ||
      typeof raw.member !== "string" || !/^[A-Za-z0-9_-]{8,64}$/.test(raw.member) ||
      typeof raw.name !== "string" || !raw.name.trim() ||
      !["user", "origin", "slug", "name"].every(key => typeof cloud[key] === "string" && (cloud[key] as string).length > 0)) return null;
    return { relay: null, team: raw.team, member: raw.member, secret: "", credential: "", name: raw.name,
      cloud: cloud as TeamConfig["cloud"] };
  }
  if (raw.relay !== null && typeof raw.relay !== "string") return null;
  if (typeof raw.team !== "string" || typeof raw.secret !== "string" || typeof raw.member !== "string") return null;
  const name = normalizeName(raw.name);
  if (!name) return null;
  if (!parseInvite(formatInvite(raw.team, raw.secret))) return null;
  if (!parseMembership({ member: raw.member, credential: raw.credential ?? "" }) && raw.credential !== undefined) return null;
  return {
    relay: raw.relay,
    team: raw.team,
    secret: raw.secret,
    member: raw.member,
    credential: raw.credential as string | undefined,
    name,
  };
}

/// Resolve relay from configuration, development override, then application default; the browser mock uses a fake endpoint.
const relayOf = (c: Pick<TeamConfig, "relay"> | StoredTeamConfig | null) =>
  (c ? c.relay || "" : relayDraft) || env?.VITE_RELAY || RELAY || (member.currentTransport().needsRelay ? "" : "ws://mock");

/// Convert HTTP(S) endpoints to WS(S), preserving existing WebSocket schemes.
const wsUrl = (relay: string) => transportWsUrl(relay, member.currentTransport().needsRelay);

const shareScopeOf = (c: TeamConfig) => c.cloud ? `organization:${c.team}:${c.member}` : `team:${c.team}`;

/// Resolve the stored configuration into the membership the core connects with.
function membershipOf(c: TeamConfig, base: string): Membership {
  const scope = JSON.stringify(c.cloud ? ["organization", c.cloud.origin, c.team] : ["team", base, c.team]);
  const common = { member: c.member, scope, privateScope: JSON.stringify([scope, c.cloud?.user ?? "", c.member]), shareScope: shareScopeOf(c) };
  const cloud = c.cloud;
  if (cloud) {
    return { ...common, legacy: false, url: async () => {
      // A logout during a pending connection must not spend a ticket for the old account.
      if (account.user?.id !== cloud.user || account.origin !== cloud.origin) return null;
      return invoke("cloud_relay_ticket", { organization: c.team, user: cloud.user, expectedOrigin: cloud.origin });
    } };
  }
  const endpoint = new URL(`${wsUrl(base)}/team/${encodeURIComponent(c.team)}`);
  endpoint.searchParams.set("c", c.credential);
  endpoint.searchParams.set("m", c.member);
  endpoint.searchParams.set("n", c.name);
  endpoint.searchParams.set("p", String(PROTO));
  const url = endpoint.toString();
  return { ...common, legacy: true, url: async () => url };
}

async function activate() {
  if (!cfg) return;
  const base = relayOf(cfg);
  if (!base) {
    member.disconnect();
    member.notify();
    return;
  }
  let membership: Membership;
  try {
    membership = membershipOf(cfg, base);
  } catch (e) {
    // An invalid stored relay URL is a configuration error, not a transient failure; do not retry.
    member.disconnect();
    member.report(fromBack(e));
    member.notify();
    return;
  }
  await member.connect(membership);
}

/* Team actions. */

async function adopt(next: TeamConfig) {
  await invoke("team_config_set", { config: next });
  member.reset();
  cfg = next;
  await activate();
  member.notify();
}

const cleanName = (name: string) => {
  const n = normalizeName(name);
  if (!n) throw t("err.team.name");
  return n;
};

export async function create(name: string) {
  const n = cleanName(name);
  const base = relayOf(null);
  if (!base) throw t("err.team.noRelay");
  const { team, secret, member: id, credential } = await member.currentTransport().create(base);
  await adopt({ relay: relayDraft || null, team, secret, member: id, credential, name: n });
}

export async function join(code: string, name: string) {
  const n = cleanName(name);
  const parsed = parseInvite(code);
  if (!parsed) throw t("err.team.badCode");
  const base = relayOf(null);
  if (!base) throw t("err.team.noRelay");
  const membership = await member.currentTransport().enroll(base, parsed.team, parsed.secret);
  await adopt({ relay: relayDraft || null, team: parsed.team, secret: parsed.secret, ...membership, name: n });
}

export async function leave() {
  if (cfg?.cloud) globalThis.localStorage?.setItem(LEFT_KEY, cfg.team);
  member.reset();
  cfg = null;
  await invoke("team_config_set", { config: null });
  member.notify();
}

export async function setName(name: string) {
  if (!cfg || cfg.cloud) return;
  const n = cleanName(name);
  cfg = { ...cfg, name: n };
  await invoke("team_config_set", { config: cfg });
  member.send({ t: "me", name: n });
  member.notify();
}

export async function setRelay(url: string) {
  if (cfg?.cloud) return;
  const u = url.trim().replace(/\/+$/, "");
  if (u) wsUrl(u);
  relayDraft = u;
  if (cfg) {
    cfg = { ...cfg, relay: u || null };
    await invoke("team_config_set", { config: cfg });
    member.disconnect();
    void activate();
  }
  member.notify();
}

/* Board facade: remote shares appear as workspaces and board IDs map to relay identities. */

/// Remote workspace entries exist only in frontend board state and carry remote metadata.
export function remotes(): Workspace[] {
  return viewer.remotes().map(({ id, share: s }) => ({
    id,
    title: s.title,
    project: "@time",
    repo: "",
    repo_name: s.repo_name,
    branch: s.branch,
    worktree: "",
    repos: [],
    stage: s.stage,
    archived: false,
    pinned: false,
    unread: false,
    // Relay shares do not identify the provider; remote UI must not derive behavior from a placeholder provider value.
    agent: "claude",
    model: "",
    effort: "",
    mcp: null,
    plugins: null,
    skills: null,
    port: null,
    issue: s.issue ? { id: "", identifier: s.issue.identifier, title: s.issue.title, url: s.issue.url } : null,
    cleaned: false,
    shared: false,
    audience: null,
    remote_control: false,
    // Remote shares are announced only after preparation is complete.
    preparing: false,
    failed: null,
    remote: { owner: s.owner, online: s.online },
    tabs: s.tabs.map((tab) => ({ ...tab })),
    active: s.active,
  }));
}

/// Return cached workspace comments and request missing data; onChange delivers later results.
export function notesOf(id: string): Note[] {
  // Guard unannounced local workspaces at the request boundary, not only by hiding comment controls.
  if (!viewer.isRemote(id) && !owner.isShared(id)) return [];
  return comments.notesOf(viewer.relayId(id));
}

export async function addNote(
  id: string,
  tab: string | null,
  anchor: string | null,
  text: string,
  mentions: string[],
  quote: string | null,
): Promise<boolean> {
  try { await owner.includeMentioned(id, mentions); } catch { return false; }
  return comments.addNote(viewer.relayId(id), tab, anchor, text, mentions, quote);
}

export async function replyNote(id: string, note: string, text: string, mentions: string[]): Promise<boolean> {
  try { await owner.includeMentioned(id, mentions); } catch { return false; }
  return comments.replyNote(viewer.relayId(id), note, text, mentions);
}

export const resolveNote = (id: string, note: string) => comments.resolveNote(viewer.relayId(id), note);

export function readInbox(id: string): { workspace: string; note: string; tab: string | null } | null {
  const item = comments.readInbox(id);
  return item && { workspace: viewer.boardId(item.ws), note: item.note, tab: item.tab };
}
