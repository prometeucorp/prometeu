import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Down, Member, Note, Up } from "../relay/src/protocol";
import { generateIdentity } from "./team-crypto";
import { t } from "./i18n";
import type { Board, Workspace } from "./types";

const config = {
  relay: "wss://relay.exemplo", team: "time1234", secret: "segredo-de-teste-1234567",
  member: "membro1", credential: "credencial-de-teste-com-tamanho-bastante", name: "Eu",
};
const fake = vi.hoisted(() => ({ invoke: vi.fn(), security: null as unknown,
  chat: null as ((event: { payload: [string, string, number] }) => void) | null,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (event, listener) => {
  if (event === "chat") fake.chat = listener;
  return () => {};
}) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: fake.invoke }));
let team: typeof import("./team");

/** The relay records ciphertext and publishes only the client's public identity. */
class Relay {
  sent: Up[] = [];
  binaries: Uint8Array[] = [];
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  members: Member[] = [];
  send(data: string | ArrayBuffer | Uint8Array) {
    if (typeof data !== "string") { this.binaries.push(new Uint8Array(data as ArrayBuffer)); return; }
    if (data === "ping") return;
    const frame = JSON.parse(data) as Up;
    this.sent.push(frame);
    if (frame.t === "identity") {
      this.members = [{ id: config.member, name: config.name, online: true, key: frame.key }, ...this.members.filter(member => member.id !== config.member)];
      this.says({ t: "presence", members: this.members });
    }
  }
  close() {}
  says(frame: Down) { this.onmessage?.({ data: JSON.stringify(frame) }); }
  only<T extends Up["t"]>(kind: T) {
    return this.sent.filter((f): f is Extract<Up, { t: T }> => f.t === kind);
  }
}
let relay: Relay;
let connections = 0;
const workspace = (id: string, shared: boolean): Workspace => ({
  id, title: id, project: "p", repo: "/r", repo_name: "r", branch: "b", worktree: `/w/${id}`,
  repos: [], stage: "", archived: false, pinned: false, unread: false, agent: "claude", model: "", effort: "",
  mcp: null, plugins: null, skills: null, port: null, issue: null, cleaned: false, shared, audience: null, remote_control: false,
  preparing: false, failed: null, remote: null, tabs: [], active: null,
});
const board = (...workspaces: Workspace[]): Board => ({ stages: [], projects: [], workspaces });
async function welcome() {
  await vi.waitFor(() => expect(relay.onmessage).not.toBeNull());
  relay.onopen?.();
  relay.says({ t: "welcome", comments: 1, e2ee: 1, challenge: crypto.randomUUID(),
    you: config.member, members: relay.members, shares: [], inbox: [], watching: {} });
  await vi.waitFor(() => expect(team.status().phase).toBe("online"));
}
async function reconnect() {
  const previous = connections;
  relay.onclose?.();
  await vi.advanceTimersByTimeAsync(31_000);
  await vi.waitFor(() => expect(connections).toBeGreaterThan(previous));
  await welcome();
}
async function announce(id: string) {
  team.boardChanged(board(workspace(id, true)));
  await vi.waitFor(() => expect(relay.only("share").some(f => f.share.id === id)).toBe(true));
}
async function outgoingNote(ws: string, text: string): Promise<Note> {
  expect(await team.addNote(ws, null, null, text, [], null)).toBe(true);
  await vi.waitFor(() => expect(relay.only("note").length).toBeGreaterThan(0));
  const sent = relay.only("note").slice(-1)[0]!;
  return { id: sent.encrypted!.id, ws, author: config.member, text: "", mentions: [], quote: null,
    ts: 1, tab: null, anchor: null, parent: null, resolved: false, encrypted: sent.encrypted };
}
beforeEach(async () => {
  vi.useFakeTimers(); vi.resetModules(); fake.security = null; fake.chat = null;
  fake.invoke.mockReset().mockImplementation(async (command: string, args: any) => {
    if (command === "team_config") return { config, default_name: "Eu" };
    if (command === "team_security") return structuredClone(fake.security);
    if (command === "team_security_set") { fake.security = structuredClone(args.state); return; }
    return null;
  });
  team = await import("./team");
  relay = new Relay(); connections = 0;
  team.useTransport({ socket: () => { connections++; return relay; }, create: vi.fn(), enroll: vi.fn(), needsRelay: true });
  await team.init();
});
afterEach(async () => { await team.leave(); vi.useRealTimers(); });

describe("notas do time", () => {
  it("não consulta um workspace local que nunca foi anunciado", () => {
    expect(team.notesOf("workspace-local-nunca-compartilhado")).toEqual([]);
  });
  it("pergunta pelas notas depois de anunciar o que é meu e reutiliza a identidade ao reconectar", async () => {
    await welcome(); await announce("ws1");
    const key = relay.only("identity")[0].key;
    team.notesOf("ws1");
    await vi.waitFor(() => expect(relay.only("notes")).toHaveLength(1));
    relay.sent = [];
    await reconnect();
    await vi.waitFor(() => expect(relay.only("notes")).toHaveLength(1));
    expect(relay.sent.filter(f => f.t === "share" || f.t === "notes").map(f => f.t)).toEqual(["share", "notes"]);
    expect(relay.only("identity")[0].key).toBe(key);
  });
  it("não repete o pedido de um workspace que parou de ser compartilhado", async () => {
    await welcome(); await announce("ws1");
    team.notesOf("ws1");
    team.boardChanged(board(workspace("ws1", false)));
    await vi.waitFor(() => expect(relay.only("unshare")).toHaveLength(1));
    relay.sent = [];
    await reconnect();
    expect(relay.only("notes")).toEqual([]);
  });
  it("cifra comentário, resposta e resolução; atualização autenticada substitui a raiz", async () => {
    await welcome(); await announce("comments-ws");
    expect(team.supportsThreads()).toBe(true);
    const root = await outgoingNote("comments-ws", "texto privado do comentário");
    expect(await team.replyNote(root.ws, root.id, "resposta privada", [])).toBe(true);
    expect(await team.resolveNote(root.ws, root.id)).toBe(true);
    await vi.waitFor(() => expect(relay.only("note_resolve")).toHaveLength(1));
    const reply = relay.only("note_reply")[0];
    const resolution = relay.only("note_resolve")[0];
    expect(root.encrypted?.boxes[config.member]).toBeTruthy();
    expect(reply).toMatchObject({ note: root.id, text: "", encrypted: { boxes: expect.any(Object) } });
    expect(JSON.stringify(relay.sent)).not.toContain("texto privado");
    expect(JSON.stringify(relay.sent)).not.toContain("resposta privada");
    relay.says({ t: "notes", ws: root.ws, items: [root] });
    await vi.waitFor(() => expect(team.notesOf(root.ws)[0]?.text).toBe("texto privado do comentário"));
    relay.says({ t: "note", note: { ...root, resolved: true,
      resolution: { author: config.member, encrypted: resolution.encrypted! } } });
    await vi.waitFor(() => expect(team.notesOf(root.ws)[0]?.resolved).toBe(true));
    expect(team.notesOf(root.ws)).toHaveLength(1);
  });
  it("bloqueia comentário durante mudança de audiência e usa somente destinatários confirmados ao terminar", async () => {
    await welcome();
    const bob = await generateIdentity(), eve = await generateIdentity();
    relay.members.push({ id: "bob", name: "Bob", online: true, key: bob.publicKey },
      { id: "eve", name: "Eve", online: true, key: eve.publicKey });
    relay.says({ t: "presence", members: relay.members });
    await vi.waitFor(() => expect(team.status().members).toHaveLength(3));
    await announce("ws1");
    const before = await outgoingNote("ws1", "antes da remoção");
    expect(Object.keys(before.encrypted!.boxes).sort()).toEqual(["bob", "eve", config.member]);
    let finish!: () => void;
    const previous = fake.invoke.getMockImplementation()!;
    fake.invoke.mockImplementation((command, args) => command === "set_shared"
      ? new Promise<void>(resolve => { finish = resolve; }) : previous(command, args));
    const change = team.share("ws1", ["bob"]);
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    expect(await team.addNote("ws1", null, null, "rascunho durante remoção", [], null)).toBe(false);
    expect(relay.only("note")).toHaveLength(1);
    await expect(team.share("ws1", ["eve"])).rejects.toBeTruthy();
    // The IPC resolves before the next board event; the old board must not supply recipients.
    finish(); await change;
    const sharesBeforePresence = relay.only("share").length;
    relay.says({ t: "presence", members: relay.members });
    await vi.waitFor(() => expect(relay.only("share").length).toBeGreaterThan(sharesBeforePresence));
    const after = await outgoingNote("ws1", "depois da remoção");
    expect(Object.keys(after.encrypted!.boxes).sort()).toEqual(["bob", config.member]);
    expect(JSON.stringify(relay.sent)).not.toContain("rascunho durante remoção");
  });
  it("informa falha de cifra sem confirmar envio do comentário", async () => {
    const fail = vi.fn(); team.onError(fail);
    await welcome(); await announce("ws1");
    expect(await team.addNote("workspace-sem-chave", null, null, "rascunho privado", [], null)).toBe(false);
    expect(relay.only("note")).toEqual([]);
    await vi.waitFor(() => expect(fail).toHaveBeenCalledWith(t("err.team.encryption")));
  });
  it("abrir item cifrado de Para mim não o remove", async () => {
    await welcome(); await announce("ws1");
    const root = await outgoingNote("ws1", "revisa em privado");
    relay.says({ t: "inbox", items: [{ id: root.id, ws: root.ws, author: root.author, ts: root.ts, encrypted: root.encrypted }] });
    await vi.waitFor(() => expect(team.inboxCount()).toBe(1));
    expect(team.inboxList()[0].text).toBe("revisa em privado");
    expect(team.readInbox(root.id)).toEqual({ workspace: "ws1", note: root.id, tab: null });
    expect(team.inboxCount()).toBe(1);
    expect(relay.only("inbox_read")).toEqual([]);
  });
  it("recusa relay antigo sem retornar a comentários e inbox em texto", async () => {
    const fail = vi.fn(); team.onError(fail);
    await vi.waitFor(() => expect(relay.onmessage).not.toBeNull());
    relay.onopen?.();
    relay.says({ t: "welcome", you: config.member, members: [], shares: [],
      inbox: [{ id: "legacy", ws: "ws1", author: "alice", ts: 1 }], watching: {} });
    await vi.waitFor(() => expect(fail).toHaveBeenCalledWith(t("err.team.encryption")));
    expect(team.status().phase).not.toBe("online");
    expect(team.readInbox("legacy")).toBeNull();
    expect(await team.addNote("ws1", null, null, "segredo", [], null)).toBe(false);
    expect(team.inboxCount()).toBe(0);
    expect(relay.sent).toEqual([]);
  });
});
it("retoma stream após reconectar ao mesmo time com snapshot anterior pendente", async () => {
  await welcome();
  const guest = await generateIdentity();
  relay.members.push({ id: "guest", name: "Guest", online: true, key: guest.publicKey });
  relay.says({ t: "presence", members: relay.members });
  await vi.waitFor(() => expect(team.status().members).toHaveLength(2));
  const work = { ...workspace("ws-stream", true), active: "tab-stream", tabs: [
    { id: "tab-stream", title: "Chat", status: "pronta", note: null, tokens: null },
  ] } as Workspace;
  team.boardChanged(board(work));
  await vi.waitFor(() => expect(relay.only("share")).toHaveLength(1));
  let finish!: (snapshot: { text: string; seq: number }) => void;
  let snapshots = 0;
  const previous = fake.invoke.getMockImplementation()!;
  fake.invoke.mockImplementation((command, args) => {
    if (command !== "chat_snapshot") return previous(command, args);
    snapshots++;
    return snapshots === 1 ? new Promise(resolve => { finish = resolve; }) : Promise.resolve({ text: "snapshot atual", seq: 2 });
  });
  const watch: Down = { t: "watch", ws: work.id, tab: "tab-stream", members: ["guest"], added: ["guest"] };
  relay.says(watch);
  await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
  await reconnect();
  await vi.waitFor(() => expect(relay.only("share")).toHaveLength(2));
  finish({ text: "snapshot antigo", seq: 1 });
  await vi.advanceTimersByTimeAsync(0);
  expect(relay.binaries).toHaveLength(0);
  relay.says(watch);
  await vi.waitFor(() => expect(relay.binaries).toHaveLength(1));
  fake.chat!({ payload: ["tab-stream", "linha depois da reconexão", 3] });
  await vi.advanceTimersByTimeAsync(40);
  await vi.waitFor(() => expect(relay.binaries).toHaveLength(2));
});

describe("erro do relay", () => {
  it("cala pedidos automáticos e mostra o erro da ação da pessoa", async () => {
    const fail = vi.fn(); team.onError(fail); await welcome();
    relay.says({ t: "error", code: "noShare" });
    relay.says({ t: "error", code: "noTab" });
    relay.says({ t: "error", code: "tooBig" });
    await vi.waitFor(() => expect(fail).toHaveBeenCalledWith(t("err.team.tooBig")));
    expect(fail).toHaveBeenCalledTimes(1);
  });
});
describe("ouvintes", () => {
  it("deixa uma tela removida parar de ouvir mudanças", async () => {
    const changed = vi.fn(); const stop = team.onChange(changed);
    await team.setName("Primeiro nome");
    expect(changed).toHaveBeenCalledTimes(1);
    stop(); await team.setName("Segundo nome");
    expect(changed).toHaveBeenCalledTimes(1);
  });
});
