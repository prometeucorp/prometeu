import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Down, Member, Up } from "../relay/src/protocol";
import { generateIdentity, seal, type Identity } from "./team-crypto";
import { t } from "./i18n";
import type { Membership, OwnerHost } from "./team-ports";
import type { Board, ShareRights, Workspace } from "./types";

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
/// Owner confirmation stays off unless a test is about it, so these tests see input run as soon as it is allowed.
const workspace = (audience: string[] | null, remoteControl = false, rights: ShareRights | null = { send: [], control: [] },
  confirm = false): Workspace => ({
  id: "ws1", title: "Work", project: "p", repo: "/r", repo_name: "repo", branch: "main", worktree: "/w/ws1", repos: [],
  stage: "", archived: false, pinned: false, unread: false, agent: "claude", model: "", effort: "", mcp: null, plugins: null,
  skills: null, port: null, issue: null, cleaned: false, shared: true, share_team: membership.shareScope, audience,
  remote_control: remoteControl, rights, confirm_messages: confirm, preparing: false, failed: null, remote: null, active: "tab1",
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
const controls = () => host.control.mock.calls.map(([, frame]) => (frame as { type: string }).type);
const respond = JSON.stringify({ v: 1, type: "request.respond", requestId: "r1", response: { outcome: "allow" } });
const interrupt = JSON.stringify({ v: 1, type: "turn.interrupt" });
const send = (...people: string[]): ShareRights => ({ send: people, control: [] });
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
  await upgradedFrom(workspace(null, true, send("bob")));
  await settled(await write(devices[1], "Merge it"));
  await settled(await write(devices[0], "Merge it"));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "Merge it" }), "Merge it"]);
});

it("pauses input after a key change while content keeps flowing", async () => {
  await upgradedFrom(workspace(["bob"], false, send("bob")));
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
  await settled(await write(devices[3], respond));
  expect(host.prompt).not.toHaveBeenCalled();
  expect(host.control).not.toHaveBeenCalled();
  // Without a right there is nothing to approve, so no notice either.
  expect(owner.pausedIn("ws1")).toEqual([]);
});

it("pauses a new device of a person who has the right", async () => {
  await upgradedFrom(workspace(null, false, { send: ["bob"], control: ["bob"] }));
  devices.push(await device("bob-phone", "Bob (iPhone)", "bob"));
  socket.presence();
  await settled(await write(devices[3], "Delete the branch"));
  await settled(await write(devices[3], respond));
  expect(host.prompt).not.toHaveBeenCalled();
  expect(host.control).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toEqual([{ person: "bob", name: "Bob", own: false }]));
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
  await upgradedFrom(workspace(null, false, send("dave")));
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
  await upgradedFrom(workspace(["bob"], false, send("bob")));
  devices[1] = { ...devices[1], identity: await generateIdentity() };
  socket.presence();
  await settled(await write(devices[1], "before"));
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toHaveLength(1));
  member.reset();
  await connect(workspace(["bob"], false, send("bob")));
  await vi.waitFor(() => expect(owner.pausedIn("ws1")).toEqual([{ person: "bob", name: "Bob", own: false }]));
  await settled(await write(devices[1], "after"));
  expect(host.prompt).not.toHaveBeenCalled();
  await owner.dismissInput("bob");
  expect(owner.pausedIn("ws1")).toEqual([]);
  await settled(await write(devices[1], "still paused"));
  expect(host.prompt).not.toHaveBeenCalled();
});

it("spends the replay receipt of discarded input so approval cannot run it later", async () => {
  await upgradedFrom(workspace(["bob"], false, send("bob")));
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

it("approves a person's devices when granting a right, never when choosing who views", async () => {
  await connect(workspace(["carol"]));
  await owner.share("ws1", ["carol", "bob"]);
  await settled(await write(devices[1], "seen, not granted"));
  expect(host.prompt).not.toHaveBeenCalled();
  expect(await owner.grant("ws1", "bob", "send", true)).toBe(1);
  await settled(await write(devices[1], "granted"));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "granted" })]);
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

it("needs Send messages to prompt or interrupt and Control to answer, even when the whole organization views", async () => {
  await upgradedFrom(workspace(null));
  await settled(await write(devices[1], "Merge it"));
  await settled(await write(devices[1], interrupt));
  await settled(await write(devices[1], respond));
  expect(prompts()).toEqual([]);
  expect(controls()).toEqual([]);
  owner.boardChanged(board(workspace(null, false, send("bob"))));
  await settled(await write(devices[1], "Merge it"));
  await settled(await write(devices[1], interrupt));
  await settled(await write(devices[1], respond));
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "Merge it" })]);
  expect(controls()).toEqual(["turn.interrupt"]);
  owner.boardChanged(board(workspace(null, false, { send: [], control: ["bob"] })));
  await settled(await write(devices[1], "Merge it again"));
  await settled(await write(devices[1], respond));
  expect(prompts()).toHaveLength(1);
  expect(controls()).toEqual(["turn.interrupt", "request.respond"]);
});

it("lets shares from before rights only view and comment", async () => {
  await upgradedFrom(workspace(["bob"], false, null));
  await settled(await write(devices[1], "Merge it"));
  await settled(await write(devices[1], respond));
  expect(host.prompt).not.toHaveBeenCalled();
  expect(host.control).not.toHaveBeenCalled();
  socket.says({ t: "watch", ws: "ws1", tab: "tab1", members: ["bob"], added: ["bob"] });
  await vi.waitFor(() => expect(socket.sent.some(frame => frame instanceof Uint8Array)).toBe(true));
});

it("lets the owner's devices act through remote control without per-person rights", async () => {
  await upgradedFrom(workspace([], true));
  await settled(await write(devices[0], "Merge it"));
  await settled(await write(devices[0], respond));
  expect(prompts()).toEqual(["Merge it"]);
  expect(controls()).toEqual(["request.respond"]);
});

it("drops people who left the organization when a right changes, so a new grant reaches peers", async () => {
  const left = Array.from({ length: 64 }, (_, i) => `gone${i}`);
  await connect(workspace(null, false, { send: left, control: ["gone0"] }));
  expect(await owner.grant("ws1", "bob", "send", true)).toBe(1);
  expect(host.setShared).toHaveBeenLastCalledWith("ws1", true, null, false, membership.shareScope, { send: ["bob"], control: ["gone0"] });
});

it("adds a granted person to the audience and drops the rights of people who stop viewing", async () => {
  await connect(workspace(["carol"]));
  expect(await owner.grant("ws1", "bob", "control", true)).toBe(1);
  expect(host.setShared).toHaveBeenLastCalledWith("ws1", true, ["carol", "bob"], false, membership.shareScope, { send: [], control: ["bob"] });
  await owner.share("ws1", ["carol"]);
  expect(host.setShared).toHaveBeenLastCalledWith("ws1", true, ["carol"], false, membership.shareScope, { send: [], control: [] });
  expect(await owner.grant("ws1", "carol", "send", false)).toBe(0);
  await owner.share("ws1", false);
  expect(host.setShared).toHaveBeenLastCalledWith("ws1", false, null, false, null, null);
});

it.each([
  ["turning remote control on", () => owner.remoteControl("ws1", true)],
  ["granting a right", () => owner.grant("ws1", "bob", "send", true)],
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
  await upgradedFrom(workspace(["bob"], false, send("bob")));
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
  const reconnected = connect(workspace(["bob"], false, send("bob")));
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

/* Owner confirmation (ADR 0090): a teammate's message waits in the owner's composer, only in memory. */

/// Bob may send messages, the owner's phone acts through remote control and messages wait for the owner.
const holding = (rights: ShareRights = send("bob")) => workspace(["bob"], true, rights, true);

it("holds a teammate's message until the owner sends it, keeping the team prefix", async () => {
  await upgradedFrom(holding());
  await settled(await write(devices[1], "Merge it"));
  expect(host.prompt).not.toHaveBeenCalled();
  const [held] = owner.pendingIn("tab1");
  expect(held).toEqual({ id: expect.any(String), name: "Bob", text: "Merge it" });
  expect(await owner.sendPending(held.id)).toBe(true);
  expect(prompts()).toEqual([t("team.remotePrompt", { name: "Bob", text: "Merge it" })]);
  expect(owner.pendingIn("tab1")).toEqual([]);
  expect(await owner.sendPending(held.id)).toBe(false);
  expect(prompts()).toHaveLength(1);
});

it("hands an edited message to the owner without the prefix and forgets a discarded one", async () => {
  await upgradedFrom(holding());
  await settled(await write(devices[1], "Merge it"));
  await settled(await write(devices[1], "Then deploy"));
  const [first, second] = owner.pendingIn("tab1");
  expect(owner.editPending(first.id)).toBe("Merge it");
  owner.discardPending(second.id);
  expect(owner.pendingIn("tab1")).toEqual([]);
  expect(owner.editPending(first.id)).toBeNull();
  expect(host.prompt).not.toHaveBeenCalled();
});

it("reports a failed send like the composer, without holding the message again", async () => {
  await upgradedFrom(holding());
  await settled(await write(devices[1], "Merge it"));
  const [held] = owner.pendingIn("tab1");
  // A stopped agent queues the text before its restart fails, so offering it again could run it twice.
  host.prompt.mockRejectedValueOnce("err.session.noTab");
  await expect(owner.sendPending(held.id)).rejects.toBe("err.session.noTab");
  expect(owner.pendingIn("tab1")).toEqual([]);
  expect(await owner.sendPending(held.id)).toBe(false);
  expect(host.prompt).toHaveBeenCalledTimes(1);
});

it.each([
  ["its conversation closes", (w: Workspace): Board => board({ ...w, tabs: [{ ...w.tabs[0], id: "tab2" }], active: "tab2" })],
  ["its workspace is archived", (w: Workspace): Board => board({ ...w, archived: true })],
  ["its workspace is removed", (): Board => ({ workspaces: [], projects: [], stages: [] })],
  ["sharing stops", (w: Workspace): Board => board({ ...w, shared: false, audience: null, remote_control: false, rights: null })],
  ["the sender loses Send messages", (w: Workspace): Board => board({ ...w, rights: { send: [], control: ["bob"] } })],
])("drops a held message when %s", async (_, change) => {
  await upgradedFrom(holding());
  await settled(await write(devices[1], "Merge it"));
  const [held] = owner.pendingIn("tab1");
  owner.boardChanged(change(holding()));
  expect(owner.pendingIn("tab1")).toEqual([]);
  expect(await owner.sendPending(held.id)).toBe(false);
  expect(owner.editPending(held.id)).toBeNull();
  owner.boardChanged(board(holding()));
  expect(owner.pendingIn("tab1")).toEqual([]);
  expect(host.prompt).not.toHaveBeenCalled();
});

it("drops a held message as soon as the owner revokes the sender's right", async () => {
  await upgradedFrom(holding());
  await settled(await write(devices[1], "Merge it"));
  expect(owner.pendingIn("tab1")).toHaveLength(1);
  await owner.grant("ws1", "bob", "send", false);
  expect(owner.pendingIn("tab1")).toEqual([]);
});

it("runs the owner's devices, interrupts and answers at once, and teammates' messages once confirmation is off", async () => {
  await upgradedFrom(holding({ send: ["bob"], control: ["bob"] }));
  await settled(await write(devices[0], "From my phone"));
  await settled(await write(devices[1], interrupt));
  await settled(await write(devices[1], respond));
  expect(prompts()).toEqual(["From my phone"]);
  expect(controls()).toEqual(["turn.interrupt", "request.respond"]);
  expect(owner.pendingIn("tab1")).toEqual([]);
  owner.boardChanged(board({ ...holding(), confirm_messages: false }));
  await settled(await write(devices[1], "Merge it"));
  expect(prompts()).toEqual(["From my phone", t("team.remotePrompt", { name: "Bob", text: "Merge it" })]);
});

it("holds messages on boards saved before the option and never writes them to disk", async () => {
  const { confirm_messages: _, ...older } = workspace(["bob"], false, send("bob"));
  await upgradedFrom(older as Workspace);
  await settled(await write(devices[1], "the release plan"));
  expect(owner.pendingIn("tab1")).toEqual([expect.objectContaining({ text: "the release plan" })]);
  expect(JSON.stringify(disk)).not.toContain("the release plan");
  expect(host.setShared).not.toHaveBeenCalled();
  expect(host.prompt).not.toHaveBeenCalled();
});

it("keeps held messages while the connection drops and forgets them with the organization", async () => {
  await upgradedFrom(holding());
  await settled(await write(devices[1], "Merge it"));
  await settled(await write(devices[1], "Then deploy"));
  socket.close();
  const [first] = owner.pendingIn("tab1");
  expect(await owner.sendPending(first.id)).toBe(true);
  expect(owner.pendingIn("tab1")).toHaveLength(1);
  member.reset();
  expect(owner.pendingIn("tab1")).toEqual([]);
});

it("keeps only the latest messages a teammate floods one conversation with", async () => {
  await upgradedFrom(holding());
  for (let i = 0; i <= 20; i++) await settled(await write(devices[1], `message ${i}`));
  const held = owner.pendingIn("tab1");
  expect(held).toHaveLength(20);
  expect(held[0].text).toBe("message 1");
});
