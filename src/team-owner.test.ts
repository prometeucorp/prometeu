import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Down, Member, Up } from "../relay/src/protocol";
import { generateIdentity, seal, type Identity } from "./team-crypto";
import { t } from "./i18n";
import type { Membership, OwnerHost } from "./team-ports";
import type { Board, Workspace } from "./types";

/** The owner feature over the portable member core, with an injected host and storage: no Tauri, no IPC. */

const SCOPE = JSON.stringify(["organization", "https://cloud.test", "org1"]);
const PRIVATE = JSON.stringify([SCOPE, "user1", "alice"]);
const membership: Membership = {
  member: "alice", scope: SCOPE, privateScope: PRIVATE, shareScope: "organization:org1:alice", legacy: false,
  url: async () => "wss://relay.test/organization/org1?ticket=t",
};
/// Another organization of the same person, whose devices carry the same IDs in this harness.
const OTHER_SCOPE = JSON.stringify(["organization", "https://cloud.test", "org2"]);
const OTHER_PRIVATE = JSON.stringify([OTHER_SCOPE, "user1", "alice"]);
const otherMembership: Membership = {
  ...membership, scope: OTHER_SCOPE, privateScope: OTHER_PRIVATE, shareScope: "organization:org2:alice",
  url: async () => "wss://relay.test/organization/org2?ticket=t",
};

class Socket {
  sent: (Up | Uint8Array)[] = [];
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  key = "";
  constructor(private roster: () => Member[]) {}
  send(data: string | ArrayBuffer | Uint8Array) {
    if (data === "ping") return;
    if (typeof data !== "string") { this.sent.push(new Uint8Array(data as ArrayBuffer)); return; }
    const frame = JSON.parse(data) as Up;
    this.sent.push(frame);
    if (frame.t === "identity") { this.key = frame.key; this.presence(); }
  }
  close() { this.onclose?.(); }
  says(frame: Down) { this.onmessage?.({ data: JSON.stringify(frame) }); }
  presence() { this.says({ t: "presence", members: [{ id: "alice", name: "Alice", online: true, key: this.key }, ...this.roster()] }); }
  shares() { return this.sent.filter((frame): frame is Extract<Up, { t: "share" }> => !(frame instanceof Uint8Array) && frame.t === "share"); }
}

type Device = { id: string; name: string; person?: string; identity: Identity };
let member: typeof import("./team-member");
let owner: typeof import("./team-owner");
let host: { [K in keyof OwnerHost]: ReturnType<typeof vi.fn> };
let disk: unknown;
let devices: Device[];
let socket: Socket;

const device = async (id: string, name: string, person?: string): Promise<Device> => ({ id, name, person, identity: await generateIdentity() });
const roster = (): Member[] => devices.map(d => ({ id: d.id, name: d.name, online: true, key: d.identity.publicKey, ...(d.person ? { person: d.person } : {}) }));
const workspace = (audience: string[] | null, remoteControl = false): Workspace => ({
  id: "ws1", title: "Work", project: "p", repo: "/r", repo_name: "repo", branch: "main", worktree: "/w/ws1", repos: [],
  stage: "", archived: false, pinned: false, unread: false, agent: "claude", model: "", effort: "", mcp: null, plugins: null,
  skills: null, port: null, issue: null, cleaned: false, shared: true, share_team: membership.shareScope, audience,
  remote_control: remoteControl, preparing: false, failed: null, remote: null, active: "tab1",
  tabs: [{ id: "tab1", title: "Chat", status: "pronta", note: null, tokens: null }],
} as Workspace);
const board = (work: Workspace): Board => ({ workspaces: [work], projects: [], stages: [] });

async function connect(work: Workspace, announced = true, using = membership) {
  const sockets: Socket[] = [];
  member.useTransport({ needsRelay: true, create: vi.fn(), enroll: vi.fn(), socket: () => { const s = new Socket(roster); sockets.push(s); return s; } });
  await member.connect(using);
  await vi.waitFor(() => expect(sockets).toHaveLength(1));
  socket = sockets[0];
  socket.onopen?.();
  socket.says({ t: "welcome", comments: 1, e2ee: 1, challenge: crypto.randomUUID(), you: "alice", members: roster(), shares: [], inbox: [], watching: {} });
  await vi.waitFor(() => expect(member.current().phase).toBe("online"));
  owner.boardChanged(board(work));
  if (announced) await vi.waitFor(() => expect(socket.shares().length).toBeGreaterThan(0));
}

/// A device seals a message for the owner exactly as its own channel would.
async function write(from: Device, data: string): Promise<Down> {
  const id = crypto.randomUUID();
  const payload = { frame: { t: "write", ws: "ws1", tab: "tab1", data }, expires: Date.now() + 120_000 };
  const box = await seal(from.identity, socket.key, [SCOPE, from.id, "alice", id], new TextEncoder().encode(JSON.stringify(payload)));
  const frame: Down = { t: "write", ws: "ws1", tab: "tab1", from: from.id, data: "", encrypted: { id, boxes: { alice: box } } };
  socket.says(frame);
  return frame;
}

const receipts = () => (disk as { scopes: Record<string, { receipts?: Record<string, number> }> }).scopes[PRIVATE].receipts ?? {};
const approvedIn = (scope: string) => (disk as { scopes: Record<string, { approved?: Record<string, string> }> }).scopes[scope]?.approved;
const prompts = () => host.prompt.mock.calls.map(([, text]) => text);
/// Remote input is handled asynchronously after decryption; wait until its receipt is persisted and the queue settles.
async function settled(frame: Down) {
  if (frame.t !== "write") throw new Error("Not an input frame");
  await vi.waitFor(() => expect(receipts()[frame.encrypted!.id]).toBeDefined());
  await new Promise(resolve => setTimeout(resolve, 0));
}

beforeEach(async () => {
  vi.resetModules();
  disk = null;
  member = await import("./team-member");
  owner = await import("./team-owner");
  host = { setShared: vi.fn(async () => {}), snapshot: vi.fn(async () => ({ text: "private transcript", seq: 1 })),
    control: vi.fn(async () => {}), prompt: vi.fn(async () => {}) };
  member.useSecurityStore({ read: async () => structuredClone(disk), write: async (state) => { disk = structuredClone(state); } });
  member.register((ctx) => owner.install(ctx, host as unknown as OwnerHost));
  devices = [await device("phone", "Alice (iPhone)", "alice"), await device("bob", "Bob"), await device("carol", "Carol")];
});
afterEach(() => member.reset());

/// Pin the current roster as the previous version did: links without approvals, approved on upgrade.
async function upgradedFrom(work: Workspace) {
  await connect(work);
  const scope = (disk as { scopes: Record<string, Record<string, unknown>> }).scopes[PRIVATE];
  delete scope.approved;
  member.reset();
  await connect(work);
}

it("keeps teammates and devices pinned before the upgrade able to send input", async () => {
  await upgradedFrom(workspace(null, true));
  await settled(await write(devices[1], "Merge it"));
  await settled(await write(devices[0], "Merge it"));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "Merge it" }), "Merge it"]);
});

it("pauses input after a key change while content keeps flowing", async () => {
  await upgradedFrom(workspace(["bob"]));
  devices[1] = { ...devices[1], identity: await generateIdentity() };
  socket.presence();
  await vi.waitFor(() => expect(Object.keys(socket.shares().slice(-1)[0].share.encrypted!.boxes)).toContain("bob"));
  const changed = await write(devices[1], "Merge it");
  await settled(changed);
  expect(host.prompt).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toEqual([{ person: "bob", name: "Bob", own: false }]));
  socket.says({ t: "watch", ws: "ws1", tab: "tab1", members: ["bob"], added: ["bob"] });
  await vi.waitFor(() => expect(socket.sent.some(frame => frame instanceof Uint8Array)).toBe(true));
  expect(host.snapshot).toHaveBeenCalledWith("tab1");
});

it("does not run input from a member who first appears in an organization-wide share", async () => {
  await upgradedFrom(workspace(null));
  devices.push(await device("dave", "Dave"));
  socket.presence();
  await settled(await write(devices[3], "Delete the branch"));
  expect(host.prompt).not.toHaveBeenCalled();
  expect(host.control).not.toHaveBeenCalled();
  await settled(await write(devices[3], JSON.stringify({ v: 1, type: "request.respond", requestId: "r1", response: { outcome: "allow" } })));
  expect(host.control).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toEqual([{ person: "dave", name: "Dave", own: false }]));
});

it("does not run input from a new device of the owner's person even with remote control", async () => {
  await upgradedFrom(workspace([], true));
  devices.push(await device("tablet", "Alice (iPad)", "alice"));
  socket.presence();
  await settled(await write(devices[3], "Merge it"));
  await settled(await write(devices[3], JSON.stringify({ v: 1, type: "turn.interrupt" })));
  expect(host.prompt).not.toHaveBeenCalled();
  expect(host.control).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toEqual([{ person: "alice", name: "Alice", own: true }]));
  // The phone approved before the upgrade keeps working.
  await settled(await write(devices[0], "Merge it"));
  expect(prompts()).toEqual(["Merge it"]);
});

it("runs input from every current device of a person after the owner allows them", async () => {
  await upgradedFrom(workspace(null));
  devices.push(await device("dave", "Dave"), await device("dave-phone", "Dave (iPhone)", "dave"));
  socket.presence();
  await settled(await write(devices[3], "first"));
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toHaveLength(1));
  expect(await owner.allowInput("dave")).toBe(2);
  expect(owner.pausedIn("ws1")).toEqual([]);
  await settled(await write(devices[3], "second"));
  await settled(await write(devices[4], "third"));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Dave", text: "second" }), t("team.remotePrompt", { name: "Dave (iPhone)", text: "third" })]);
});

it("keeps the pause and its notice across a restart until the owner answers", async () => {
  await upgradedFrom(workspace(["bob"]));
  devices[1] = { ...devices[1], identity: await generateIdentity() };
  socket.presence();
  await settled(await write(devices[1], "before"));
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toHaveLength(1));
  member.reset();
  await connect(workspace(["bob"]));
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toEqual([{ person: "bob", name: "Bob", own: false }]));
  await settled(await write(devices[1], "after"));
  expect(host.prompt).not.toHaveBeenCalled();
  await owner.dismissInput("bob");
  expect(owner.pausedIn("ws1")).toEqual([]);
  await settled(await write(devices[1], "still paused"));
  expect(host.prompt).not.toHaveBeenCalled();
});

it("spends the replay receipt of discarded input so approval cannot run it later", async () => {
  await upgradedFrom(workspace(["bob"]));
  devices[1] = { ...devices[1], identity: await generateIdentity() };
  socket.presence();
  const blocked = await write(devices[1], "Merge it");
  await settled(blocked);
  expect(await owner.allowInput("bob")).toBe(1);
  socket.says(blocked);
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(host.prompt).not.toHaveBeenCalled();
  await settled(await write(devices[1], "Merge it now"));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "Merge it now" })]);
});

it("approves the owner's current devices when remote control is turned on", async () => {
  await connect({ ...workspace([]), shared: false }, false);
  expect(await owner.remoteControl("ws1", true)).toBe(1);
  // The board event that follows the IPC completion announces the workspace.
  owner.boardChanged(board(workspace([], true)));
  await settled(await write(devices[0], "Merge it"));
  expect(prompts()).toEqual(["Merge it"]);
  expect(await owner.remoteControl("ws1", false)).toBe(0);
});

it("approves people chosen for the audience, never the whole organization", async () => {
  await connect(workspace(["carol"]));
  expect(await owner.share("ws1", null)).toBe(0);
  await settled(await write(devices[1], "from the organization"));
  expect(host.prompt).not.toHaveBeenCalled();
  expect(await owner.share("ws1", ["bob"])).toBe(1);
  await settled(await write(devices[1], "chosen"));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "chosen" })]);
});

it("writes approvals on the connection's queue and drops them once that connection is gone", async () => {
  await connect(workspace(null));
  const approval = owner.allowInput("bob");
  // Switching organizations reloads the store; a late write from the old connection would overwrite it.
  member.reset();
  expect(await approval).toBe(0);
  const scope = (disk as { scopes: Record<string, { approved?: Record<string, string> }> }).scopes[PRIVATE];
  expect(scope.approved).toEqual({});
});

it.each([
  ["turning remote control on", () => owner.remoteControl("ws1", true)],
  ["choosing who views", () => owner.share("ws1", ["bob"])],
])("approves nobody in another organization after %s while switching to it", async (_, act) => {
  await connect({ ...workspace([]), shared: false }, false);
  let saved!: () => void;
  host.setShared.mockImplementationOnce(() => new Promise<void>(resolve => { saved = resolve; }));
  const acting = act();
  // The owner switches organizations while the board write is still in flight; consent given in the first one
  // must not approve the devices the second directory shows.
  member.reset();
  await connect({ ...workspace([]), shared: false }, false, otherMembership);
  saved();
  expect(await acting).toBe(0);
  expect(approvedIn(OTHER_PRIVATE)).toEqual({});
});

it("never writes an answer through the store a reconnect is replacing", async () => {
  await upgradedFrom(workspace(["bob"]));
  devices[1] = { ...devices[1], identity: await generateIdentity() };
  socket.presence();
  await settled(await write(devices[1], "Merge it"));
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toHaveLength(1));
  // The next connection has read the store but not finished loading it when the owner answers.
  let reads = 0, release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  member.useSecurityStore({
    read: async () => { const state = structuredClone(disk); reads++; await held; return state; },
    write: async (state) => { disk = structuredClone(state); },
  });
  const reconnected = connect(workspace(["bob"]));
  await vi.waitFor(() => expect(reads).toBe(1));
  const answer = owner.allowInput("bob");
  release();
  await reconnected;
  // The answer did nothing rather than vanish after the load: the notice is still there and can be answered.
  expect(await answer).toBe(0);
  expect(owner.pausedIn("ws1")).toEqual([{ person: "bob", name: "Bob", own: false }]);
  expect(await owner.allowInput("bob")).toBe(1);
  await settled(await write(devices[1], "Merge it now"));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "Merge it now" })]);
});
