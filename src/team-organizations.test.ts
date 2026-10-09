import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Board, Workspace } from "./types";
import { PROTO, type Down, type Member, type Up } from "../relay/src/protocol";
import { generateIdentity, seal } from "./team-crypto";
import { t } from "./i18n";

const fake = vi.hoisted(() => ({
  config: null as unknown,
  security: null as unknown,
  invoke: vi.fn(),
  organizations: [{ id: "organization1", slug: "one", name: "One", member: "membership1", role: "owner" },
    { id: "organization2", slug: "two", name: "Two", member: "membership2", role: "member" }],
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: fake.invoke }));
const account = { user: { id: "user1", name: "Alice", email: "alice@example.com" }, origin: "https://cloud.test", offline: false };

class Socket {
  sent: (Up | Uint8Array)[] = [];
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  member = "";
  peers: Member[] = [];
  send(data: string | ArrayBuffer | Uint8Array) {
    if (data === "ping") return;
    const frame = typeof data === "string" ? JSON.parse(data) : new Uint8Array(data as ArrayBuffer);
    this.sent.push(frame);
    if (frame.t === "identity") this.says({ t: "presence", members: [
      { id: this.member, name: "Alice", online: true, key: frame.key }, ...this.peers,
    ] });
  }
  close() { this.onclose?.(); }
  says(frame: Down) { this.onmessage?.({ data: JSON.stringify(frame) }); }
  async welcome(member: string, watching = {}, peers: Member[] = []) {
    this.member = member; this.peers = peers;
    this.onopen?.();
    this.says({ t: "welcome", comments: 1, e2ee: 1, challenge: crypto.randomUUID(),
      you: member, members: peers, shares: [], inbox: [], watching });
    await vi.waitFor(() => expect(team.status().phase).toBe("online"));
  }
  content() { return this.sent.filter(frame => frame instanceof Uint8Array || frame.t !== "identity"); }
}
let team: typeof import("./team");
let sockets: Socket[];
let urls: string[];
const workspace = (scope?: string): Workspace => ({ id: "workspace1", title: "Work", repo_name: "repo", branch: "main", stage: "", issue: null,
  active: "tab1", tabs: [{ id: "tab1", title: "Chat", status: "pronta", note: null, tokens: null }], shared: true,
  share_team: scope, audience: null, remote_control: false, remote: null, archived: false, cleaned: false } as Workspace);
const board = (work: Workspace): Board => ({ workspaces: [work], projects: [], stages: [] });

beforeEach(async () => {
  vi.useFakeTimers(); vi.resetModules(); fake.config = null; fake.security = null; sockets = []; urls = [];
  fake.invoke.mockReset().mockImplementation(async (command: string, args: any) => {
    if (command === "team_config") return { config: fake.config, default_name: "Alice" };
    if (command === "team_config_set") { fake.config = args.config; return; }
    if (command === "team_security") return structuredClone(fake.security);
    if (command === "team_security_set") { fake.security = structuredClone(args.state); return; }
    if (command === "cloud_organizations") return { ...account, organizations: fake.organizations };
    if (command === "cloud_relay_ticket") return `wss://relay.test/organization/${args.organization}?ticket=${"t".repeat(43)}&p=${PROTO}`;
    if (command === "chat_snapshot") return { text: "private transcript", seq: 1 };
  });
  team = await import("./team");
  team.useTransport({ needsRelay: true, create: vi.fn(), enroll: vi.fn(), socket: url => { urls.push(url); const socket = new Socket(); sockets.push(socket); return socket; } });
  await team.init(); await team.refreshOrganizations(account);
});
afterEach(async () => { await team.leave(); vi.useRealTimers(); });

it("selects accepted organizations without enrolling and keeps the desktop bearer outside sockets and storage", async () => {
  expect(sockets).toHaveLength(0);
  await team.selectOrganization("organization1"); await vi.advanceTimersByTimeAsync(0);
  expect(urls[0]).toContain("/organization/organization1?ticket=");
  expect(fake.config).toMatchObject({ team: "organization1", member: "membership1", credential: "", secret: "" });
  expect(JSON.stringify(fake.config)).not.toContain("ticket");
  await sockets[0].welcome("membership1");
  expect(team.status().phase).toBe("online");
  await team.share("workspace1", null);
  // Sharing with the organization lets it view and comment; nobody gains a right to act (ADR 0090).
  expect(fake.invoke).toHaveBeenCalledWith("set_shared", { id: "workspace1", shared: true, audience: null, remoteControl: false,
    team: "organization:organization1:membership1", rights: { send: [], control: [] } }, undefined);
});

it("persists remote control independently and encrypts only for owner devices", async () => {
  await team.selectOrganization("organization1"); await vi.advanceTimersByTimeAsync(0);
  const phone = await generateIdentity();
  await sockets[0].welcome("membership1", {}, [
    { id: "phone1", name: "Alice (iPhone)", person: "membership1", online: true, key: phone.publicKey },
  ]);
  const work = { ...workspace("organization:organization1:membership1"), shared: false, audience: null };
  team.boardChanged(board(work));

  await team.remoteControl(work.id, true);
  expect(fake.invoke).toHaveBeenCalledWith("set_shared", {
    id: work.id, shared: true, audience: [], remoteControl: true,
    team: "organization:organization1:membership1", rights: { send: [], control: [] },
  }, undefined);
  const share = sockets[0].sent.find((frame): frame is Extract<Up, { t: "share" }> => !(frame instanceof Uint8Array) && frame.t === "share");
  expect(Object.keys(share!.share.encrypted!.boxes).sort()).toEqual(["membership1", "phone1"]);

  await team.remoteControl(work.id, false);
  expect(fake.invoke).toHaveBeenLastCalledWith("set_shared", {
    id: work.id, shared: false, audience: null, remoteControl: false, team: null, rights: null,
  }, undefined);

  const teamWork = workspace("organization:organization1:membership1");
  team.boardChanged(board(teamWork));
  await team.remoteControl(teamWork.id, true);
  await team.remoteControl(teamWork.id, false);
  expect(fake.invoke).toHaveBeenCalledWith("set_shared", {
    id: teamWork.id, shared: true, audience: null, remoteControl: false,
    team: "organization:organization1:membership1", rights: { send: [], control: [] },
  }, undefined);
});

it("sends the owner's companion input as their own message and needs a granted right for colleagues", async () => {
  await team.selectOrganization("organization1"); await vi.advanceTimersByTimeAsync(0);
  const phone = await generateIdentity(), colleague = await generateIdentity();
  await sockets[0].welcome("membership1", {}, [
    { id: "phone1", name: "Alice (iPhone)", person: "membership1", online: true, key: phone.publicKey },
    { id: "colleague", name: "Bob", online: true, key: colleague.publicKey },
  ]);
  team.boardChanged(board({ ...workspace("organization:organization1:membership1"), remote_control: true }));
  await vi.waitFor(() => expect(sockets[0].sent.some(frame => !(frame instanceof Uint8Array) && frame.t === "share")).toBe(true));
  // Turning remote control on is the owner's consent to their current devices (ADR 0090).
  expect(await team.remoteControl("workspace1", true)).toBe(1);
  const identity = sockets[0].sent.find((frame): frame is Extract<Up, { t: "identity" }> => !(frame instanceof Uint8Array) && frame.t === "identity")!;
  const sends = () => fake.invoke.mock.calls.filter(([command]) => command === "chat_send");
  const write = async (from: string, sender: typeof phone, text: string) => {
    const id = crypto.randomUUID();
    const data = { frame: { t: "write", ws: "workspace1", tab: "tab1", data: text }, expires: Date.now() + 120_000 };
    const box = await seal(sender, identity.key, [JSON.stringify(["organization", account.origin, "organization1"]), from, "membership1", id], new TextEncoder().encode(JSON.stringify(data)));
    sockets[0].says({ t: "write", ws: "workspace1", tab: "tab1", from, data: "", encrypted: { id, boxes: { membership1: box } } });
    await vi.waitFor(() => expect((fake.security as { scopes: Record<string, { receipts?: Record<string, number> }> })
      .scopes[JSON.stringify([JSON.stringify(["organization", account.origin, "organization1"]), "user1", "membership1"])].receipts?.[id]).toBeDefined());
    await vi.advanceTimersByTimeAsync(0);
  };
  await write("phone1", phone, "Faz o merge");
  expect(sends()).toEqual([["chat_send", { session: "tab1", text: "Faz o merge" }, undefined]]);
  // The whole organization views the workspace, but the colleague acts only once granted Send messages, and then
  // the message waits for the owner, who sends it with the team prefix.
  await write("colleague", colleague, "Faz o merge");
  expect(sends()).toHaveLength(1);
  expect(await team.grant("workspace1", "colleague", "send", true)).toBe(1);
  await write("colleague", colleague, "Faz o merge");
  expect(sends()).toHaveLength(1);
  const [held] = team.pendingIn("tab1");
  expect(held).toMatchObject({ name: "Bob", text: "Faz o merge" });
  expect(await team.sendPending(held.id)).toBe(true);
  expect(sends()).toContainEqual(["chat_send", { session: "tab1", text: t("team.remotePrompt", { name: "Bob", text: "Faz o merge" }) }, undefined]);
});

it("renews organization access on the same socket without changing presence or repeating identity", async () => {
  await team.selectOrganization("organization1"); await vi.advanceTimersByTimeAsync(0);
  await sockets[0].welcome("membership1");
  const initialRequests = fake.invoke.mock.calls.filter(([command]) => command === "cloud_relay_ticket").length;
  const changed = vi.fn();
  const unlisten = team.onChange(changed);
  // An older relay does not announce renewable leases; the client keeps the existing reconnect path.
  await vi.advanceTimersByTimeAsync(60_000);
  expect(fake.invoke.mock.calls.filter(([command]) => command === "cloud_relay_ticket")).toHaveLength(initialRequests);
  for (let cycle = 1; cycle <= 3; cycle++) {
    sockets[0].says({ t: "lease", expires_in: 60_000 });
    await vi.advanceTimersByTimeAsync(30_000);
    await vi.waitFor(() => expect(sockets[0].sent.filter(frame => !(frame instanceof Uint8Array) && frame.t === "renew")).toHaveLength(cycle));
    expect(team.status().phase).toBe("online");
    expect(sockets).toHaveLength(1);
  }
  expect(sockets[0].sent.filter(frame => !(frame instanceof Uint8Array) && frame.t === "identity")).toHaveLength(1);
  expect(changed).not.toHaveBeenCalled();
  unlisten();
});

it("discards an unfinished renewal after switching organizations", async () => {
  await team.selectOrganization("organization1"); await vi.advanceTimersByTimeAsync(0);
  await sockets[0].welcome("membership1");
  let finish!: (value: string) => void;
  const previous = fake.invoke.getMockImplementation()!;
  fake.invoke.mockImplementation((command, args) => command === "cloud_relay_ticket" && args.organization === "organization1"
    ? new Promise(resolve => { finish = resolve; }) : previous(command, args));
  sockets[0].says({ t: "lease", expires_in: 60_000 });
  await vi.advanceTimersByTimeAsync(30_000);
  await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
  await team.selectOrganization("organization2"); await vi.advanceTimersByTimeAsync(0);
  await sockets[1].welcome("membership2");
  finish(`wss://relay.test/organization/organization1?ticket=${"o".repeat(43)}&p=4`);
  await vi.advanceTimersByTimeAsync(0);
  expect(sockets.flatMap(s => s.sent).some(frame => !(frame instanceof Uint8Array) && frame.t === "renew")).toBe(false);
});

it("does not publish legacy shares, another organization's shares or stale transcript responses after switching", async () => {
  await team.selectOrganization("organization1"); await vi.advanceTimersByTimeAsync(0);
  const guest = await generateIdentity();
  await sockets[0].welcome("membership1", {}, [{ id: "guest", name: "Guest", online: true, key: guest.publicKey }]);
  team.boardChanged(board(workspace()));
  expect(sockets[0].content()).toEqual([]);
  const work = workspace("organization:organization1:membership1");
  team.boardChanged(board(work));
  await vi.waitFor(() => expect(sockets[0].sent).toContainEqual(expect.objectContaining({ t: "share" })));
  let finish!: (value: { text: string; seq: number }) => void;
  const previous = fake.invoke.getMockImplementation()!;
  fake.invoke.mockImplementation((command, args) => command === "chat_snapshot"
    ? new Promise(resolve => { finish = resolve; }) : previous(command, args));
  sockets[0].says({ t: "watch", ws: work.id, tab: "tab1", members: ["guest"], added: ["guest"] });
  await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
  await team.selectOrganization("organization2"); await vi.advanceTimersByTimeAsync(0);
  await sockets[1].welcome("membership2", { workspace1: { tab1: ["guest"] } });
  finish({ text: "private transcript", seq: 1 }); await vi.advanceTimersByTimeAsync(0);
  expect(sockets[1].content()).toEqual([]);
  expect(sockets[0].sent.some(frame => frame instanceof Uint8Array)).toBe(false);
  expect(team.sharedHere(work)).toBe(false);
  sockets[0].says({ t: "presence", members: [{ id: "leak", name: "Old", online: true }] });
  expect(team.status().members.map(member => member.id)).toEqual(["membership2"]);
});

it("discards a pending relay ticket after logout and drops revoked membership", async () => {
  let finish!: (value: string) => void;
  const previous = fake.invoke.getMockImplementation()!;
  fake.invoke.mockImplementation((command, args) => command === "cloud_relay_ticket" ? new Promise(resolve => { finish = resolve; }) : previous(command, args));
  const selection = team.selectOrganization("organization1");
  await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
  await team.refreshOrganizations({ ...account, user: null });
  finish("wss://relay.test/organization/organization1?ticket=old"); await selection; await vi.advanceTimersByTimeAsync(0);
  expect(sockets).toHaveLength(0);
  expect(team.status().config).toBeNull();
  fake.invoke.mockImplementation(previous);
  await team.refreshOrganizations(account); await team.selectOrganization("organization1"); await vi.advanceTimersByTimeAsync(0);
  await sockets[0].welcome("membership1");
  fake.invoke.mockImplementation((command, args) => command === "cloud_organizations" ? Promise.resolve({ ...account, organizations: [] }) : previous(command, args));
  await team.refreshOrganizations(account);
  expect(team.status().config).toBeNull();
  expect(team.status().phase).toBe("off");
});

it("adopts the only accepted organization and stops after an explicit leave", async () => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  });
  const previous = fake.invoke.getMockImplementation()!;
  fake.invoke.mockImplementation((command, args) => command === "cloud_organizations"
    ? Promise.resolve({ ...account, organizations: [fake.organizations[0]] }) : previous(command, args));
  await team.refreshOrganizations(account); await vi.advanceTimersByTimeAsync(0);
  expect(team.status().config).toMatchObject({ team: "organization1", member: "membership1" });
  await team.leave();
  await team.refreshOrganizations(account); await vi.advanceTimersByTimeAsync(0);
  expect(team.status().config).toBeNull();
  vi.unstubAllGlobals();
});
