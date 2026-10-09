import { describe, expect, it } from "vitest";
import { empty, members, reduce, type Effect } from "../relay/src/logic";
import { downForMember, encodeLive, encodeSnapshot, type Down, type Share, type Up } from "../relay/src/protocol";
import { TeamChannel } from "./team-channel";
import { decodeBase64Url, encodeBase64Url, generateIdentity, seal, verifyIdentity } from "./team-crypto";
import { TeamSecurity } from "./team-security";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const privateText = "private-marker-conversation";
const shared: Share = {
  id: "workspace", title: "private-marker-title", repo_name: "private-marker-repo", branch: "private-marker-branch",
  stage: "private-marker-stage", issue: null, active: "tab", audience: ["bob"], sizes: {},
  tabs: [{ id: "tab", title: "private-marker-tab", status: "rodando", note: "private-marker-tab-note", tokens: null }],
};

function storage() {
  let state: unknown = null;
  return { read: async () => structuredClone(state), write: async (next: unknown) => { state = structuredClone(next); } };
}

async function team() {
  const relay = empty();
  const clients = new Map<string, TeamChannel>();
  const stores = new Map<string, ReturnType<typeof storage>>();
  const deliveries: Array<{ member: string; frame: Down }> = [];
  const wire: unknown[] = [];
  async function deliver(effects: Effect[]) {
    wire.push(...effects);
    for (const effect of effects) {
      if (effect.e !== "send") continue;
      let frame = downForMember(effect.frame, effect.sock);
      // Production Room adds E2EE negotiation and verifies challenge signatures.
      if (frame.t === "welcome") frame = { ...frame, e2ee: 1, challenge: "socket-challenge" };
      const result = await clients.get(effect.sock)!.incoming(frame);
      if (result) deliveries.push({ member: effect.sock, frame: result });
    }
  }
  async function connect(member: string, scope = "organization", owned: Share[] = []) {
    const disk = stores.get(member) ?? storage();
    stores.set(member, disk);
    const security = await TeamSecurity.load(scope, disk.read, disk.write);
    const channel = new TeamChannel(security, scope, member);
    for (const share of owned) channel.own(share);
    clients.set(member, channel);
    await deliver(reduce(relay, { k: "open", sock: member, member, name: member, now: Date.now() }));
    const identity = await channel.identity("socket-challenge");
    if (identity.t !== "identity") throw new Error("Missing identity proof");
    expect(await verifyIdentity(identity.key, [member, "socket-challenge"], identity.proof)).toBe(true);
    await deliver(reduce(relay, { k: "text", sock: member, frame: identity, now: Date.now(), rand: crypto.randomUUID() }));
    expect(channel.ready).toBe(true);
    return channel;
  }
  async function send(member: string, frame: Up) {
    const encrypted = await clients.get(member)!.outgoing(frame);
    wire.push(encrypted);
    const effects = reduce(relay, { k: "text", sock: member, frame: encrypted, now: Date.now(), rand: crypto.randomUUID() });
    await deliver(effects);
    return { encrypted, effects };
  }
  const alice = await connect("alice");
  const bob = await connect("bob");
  const carol = await connect("carol");
  return { relay, clients, stores, wire, deliveries, deliver, connect, send, alice, bob, carol };
}

function received<T extends Down["t"]>(deliveries: Array<{ member: string; frame: Down }>, member: string, type: T): Extract<Down, { t: T }>[] {
  return deliveries.filter(item => item.member === member).map(item => item.frame)
    .filter((frame): frame is Extract<Down, { t: T }> => frame.t === type);
}

describe("end-to-end encrypted channel through the relay", () => {
  it("negotiates automatically and keeps workspace, snapshot and stream opaque to the relay", async () => {
    const t = await team();
    await t.send("alice", { t: "share", share: shared });
    expect(t.bob.shares.get(shared.id)).toMatchObject(shared);
    expect(t.carol.shares.size).toBe(0);
    await t.send("bob", { t: "attach", ws: shared.id, tab: "tab" });
    const inputs = [encodeSnapshot("tab", "bob", 4, encoder.encode(privateText)),
      encodeSnapshot("tab", "bob", 4, encoder.encode(privateText.repeat(12_000))),
      encodeLive("tab", [{ seq: 5, bytes: encoder.encode(privateText) }])];
    for (const input of inputs) {
      const frames = await t.alice.outgoingBinary(input, ["bob"]);
      expect(frames).toHaveLength(1);
      expect(decoder.decode(frames[0])).not.toContain(privateText);
      const effects = reduce(t.relay, { k: "binary", sock: "alice", data: frames[0] });
      const sent = effects.filter((e): e is Extract<Effect, { e: "sendBinary" }> => e.e === "sendBinary");
      expect(sent.map(e => e.sock)).toEqual(["bob"]);
      expect(await t.bob.incomingBinary(Uint8Array.from(sent[0].data).buffer)).toEqual(input);
      await expect(t.carol.incomingBinary(Uint8Array.from(sent[0].data).buffer)).rejects.toThrow();
    }
    expect(JSON.stringify(t.wire)).not.toContain("private-marker");
    expect(JSON.stringify([...t.relay.shares])).not.toContain("private-marker");
  });

  it("allows remote control only from the owner's companion devices", async () => {
    const t = await team();
    await t.deliver(reduce(t.relay, { k: "roster", now: Date.now(), members: [
      { id: "alice", name: "alice" }, { id: "bob", name: "bob" }, { id: "carol", name: "carol" },
      { id: "phone", name: "alice (iPhone)", person: "alice" }, { id: "bob-phone", name: "bob (iPhone)", person: "bob" },
    ] }));
    const phone = await t.connect("phone");
    const bobPhone = await t.connect("bob-phone");
    await t.send("alice", { t: "share", share: shared });
    expect(phone.shares.size).toBe(0);
    expect(bobPhone.shares.get(shared.id)).toMatchObject({ id: shared.id, title: shared.title });
    t.alice.own(shared, true);
    await t.send("alice", { t: "share", share: shared });
    expect(phone.shares.get(shared.id)).toMatchObject({ id: shared.id, title: shared.title });
    expect(t.carol.shares.size).toBe(0);
    const note = await t.send("alice", { t: "note", ws: shared.id, tab: null, anchor: null, text: "look at this", mentions: ["alice"], quote: null });
    expect((note.encrypted as Extract<Up, { t: "note" }>).mentions.sort()).toEqual(["alice", "phone"]);
    expect(received(t.deliveries, "phone", "inbox").slice(-1)[0]?.items.map(item => item.text)).toEqual(["look at this"]);
    await t.send("phone", { t: "attach", ws: shared.id, tab: "tab" });
    await t.send("phone", { t: "write", ws: shared.id, tab: "tab", data: "from the phone" });
    expect(received(t.deliveries, "alice", "write").slice(-1)[0]).toMatchObject({ from: "phone", data: "from the phone" });
  });

  it("shares from a second Mac as a companion device", async () => {
    const t = await team();
    await t.deliver(reduce(t.relay, { k: "roster", now: Date.now(), members: [
      { id: "alice", name: "alice" }, { id: "bob", name: "bob" }, { id: "carol", name: "carol" },
      { id: "phone", name: "alice (iPhone)", person: "alice" }, { id: "mac2", name: "alice (MacBook Air)", person: "alice" },
    ] }));
    const phone = await t.connect("phone");
    const mac2 = await t.connect("mac2");
    // Sharing with the whole organization never reaches the owner's other devices by itself.
    const everyone = { ...shared, audience: null };
    mac2.own(everyone);
    const sent = await t.send("mac2", { t: "share", share: everyone });
    expect((sent.encrypted as Extract<Up, { t: "share" }>).share.audience?.sort()).toEqual(["bob", "carol"]);
    expect(t.bob.shares.get(shared.id)).toMatchObject({ id: shared.id, owner: "mac2", title: shared.title });
    expect(t.alice.shares.size).toBe(0);
    expect(phone.shares.size).toBe(0);
    mac2.own(shared, true);
    await t.send("mac2", { t: "share", share: shared });
    expect(t.alice.shares.get(shared.id)).toMatchObject({ id: shared.id, owner: "mac2", title: shared.title });
    expect(phone.shares.get(shared.id)).toMatchObject({ id: shared.id, owner: "mac2", title: shared.title });
    expect(t.carol.shares.size).toBe(0);
    await t.send("alice", { t: "attach", ws: shared.id, tab: "tab" });
    await t.send("alice", { t: "write", ws: shared.id, tab: "tab", data: "from the other Mac" });
    expect(received(t.deliveries, "mac2", "write").slice(-1)[0]).toMatchObject({ from: "alice", data: "from the other Mac" });
  });

  it("authenticates remote input and persists replay rejection across restarts", async () => {
    const t = await team();
    await t.send("alice", { t: "share", share: shared });
    await t.send("bob", { t: "attach", ws: shared.id, tab: "tab" });
    const sent = await t.send("bob", { t: "write", ws: shared.id, tab: "tab", data: privateText });
    expect(received(t.deliveries, "alice", "write").slice(-1)[0]?.data).toBe(privateText);
    expect(JSON.stringify(sent.encrypted)).not.toContain(privateText);
    const effect = sent.effects.find((e): e is Extract<Effect, { e: "send" }> => e.e === "send" && e.frame.t === "write")!;
    const replay = downForMember(effect.frame, "alice");
    await expect(t.alice.incoming(replay)).rejects.toThrow("Expired or repeated input");
    const reopened = await t.connect("alice", "organization", [shared]);
    await expect(reopened.incoming(replay)).rejects.toThrow("Expired or repeated input");
  });

  it("exchanges comments, mentions, replies, inbox and resolution without plaintext in relay storage", async () => {
    const t = await team();
    await t.send("alice", { t: "share", share: shared });
    await t.send("bob", { t: "note", ws: shared.id, tab: "tab", anchor: "private-marker-anchor",
      text: "private-marker-comment", mentions: ["alice"], quote: "private-marker-quote" });
    const root = received(t.deliveries, "alice", "note").slice(-1)[0]!.note;
    expect(root).toMatchObject({ text: "private-marker-comment", quote: "private-marker-quote", anchor: "private-marker-anchor" });
    expect(received(t.deliveries, "alice", "inbox").slice(-1)[0]?.items[0].text).toBe("private-marker-comment");
    await t.send("alice", { t: "note_reply", ws: shared.id, note: root.id, text: "private-marker-reply", mentions: ["bob"] });
    expect(received(t.deliveries, "bob", "note").slice(-1)[0]?.note).toMatchObject({ text: "private-marker-reply", parent: root.id });
    expect(received(t.deliveries, "bob", "inbox").slice(-1)[0]?.items[0].text).toBe("private-marker-reply");
    await t.send("alice", { t: "note_resolve", ws: shared.id, note: root.id });
    expect(received(t.deliveries, "bob", "note").slice(-1)[0]?.note.resolved).toBe(true);
    await t.send("bob", { t: "notes", ws: shared.id });
    expect(received(t.deliveries, "bob", "notes").slice(-1)[0]?.items).toHaveLength(2);
    expect(JSON.stringify(t.wire)).not.toContain("private-marker");
    expect(JSON.stringify([...t.relay.notes])).not.toContain("private-marker");
    expect(JSON.stringify([...t.relay.inbox])).not.toContain("private-marker");
  });

  it("rejects outsiders, tampered ciphertext and substituted context", async () => {
    const t = await team();
    const sent = await t.send("alice", { t: "share", share: shared });
    await expect(t.carol.outgoing({ t: "attach", ws: shared.id, tab: "tab" })).rejects.toThrow();
    await expect(t.alice.outgoingBinary(encodeSnapshot("tab", "carol", 1, encoder.encode(privateText)), ["carol"])).rejects.toThrow();
    const effect = sent.effects.find((e): e is Extract<Effect, { e: "send" }> => e.e === "send" && e.sock === "bob" && e.frame.t === "share")!;
    const valid = downForMember(effect.frame, "bob");
    if (valid.t !== "share") throw new Error("Missing share");
    const tampered = structuredClone(valid);
    const bytes = decodeBase64Url(tampered.share.encrypted!.boxes.bob.ct, 1024 * 1024);
    bytes[0] ^= 1;
    tampered.share.encrypted!.boxes.bob.ct = encodeBase64Url(bytes);
    await expect(t.bob.incoming(tampered)).rejects.toThrow();
    const moved = structuredClone(valid);
    moved.share.encrypted!.id = crypto.randomUUID();
    await expect(t.bob.incoming(moved)).rejects.toThrow();
    const wrongScope = new TeamChannel(t.bob.security, "different-organization", "bob");
    await wrongScope.incoming({ t: "welcome", you: "bob", members: members(t.relay), shares: [], inbox: [], watching: {}, e2ee: 1, challenge: "challenge" });
    await wrongScope.incoming({ t: "presence", members: members(t.relay) });
    await expect(wrongScope.incoming(valid)).rejects.toThrow();
    await expect(t.carol.incoming(valid)).rejects.toThrow();
  });

  it("adopts replacement identities and rejects relays without E2EE negotiation", async () => {
    const t = await team();
    await t.send("alice", { t: "share", share: shared });
    const impostor = await generateIdentity();
    const changed = members(t.relay).map(member => member.id === "bob" ? { ...member, key: impostor.publicKey } : member);
    await t.alice.incoming({ t: "presence", members: changed });
    // Bob reinstalled: the new key replaces the pin and sharing keeps working without manual acceptance.
    const wire = await t.alice.outgoing({ t: "share", share: shared });
    expect(wire.t === "share" && wire.share.encrypted?.boxes.bob).toBeDefined();
    await expect(t.alice.outgoingBinary(encodeSnapshot("tab", "bob", 1, encoder.encode(privateText)), ["bob"])).resolves.toHaveLength(1);
    const channel = new TeamChannel(t.bob.security, "organization", "bob");
    await expect(channel.incoming({ t: "welcome", you: "bob", members: [], shares: [], inbox: [], watching: {} })).rejects.toThrow("Relay lacks encryption");
    await expect(channel.outgoing({ t: "attach", ws: shared.id, tab: "tab" })).rejects.toThrow("Encryption not ready");
    await expect(t.bob.incomingBinary(Uint8Array.from(encodeSnapshot("tab", "bob", 1, encoder.encode(privateText))).buffer)).rejects.toThrow();
  });

  it("preserves local revocation against stale echoes and rejects old revisions after restart", async () => {
    const t = await team();
    const old = await t.send("alice", { t: "share", share: { ...shared, audience: ["bob", "carol"] } });
    if (old.encrypted.t !== "share") throw new Error("Missing share");
    const echo: Down = { t: "share", share: { ...old.encrypted.share, owner: "alice", online: true } };
    await t.send("alice", { t: "share", share: shared });
    await t.alice.incoming(downForMember(echo, "alice"));
    expect(t.alice.shares.get(shared.id)?.audience).toEqual(["bob"]);
    await expect(t.alice.outgoingBinary(encodeSnapshot("tab", "carol", 1, encoder.encode(privateText)), ["carol"])).rejects.toThrow();
    await expect(t.bob.incoming(downForMember(echo, "bob"))).rejects.toThrow("Repeated or replaced share");
    const reopened = await t.connect("bob");
    await expect(reopened.incoming(downForMember(echo, "bob"))).rejects.toThrow("Repeated or replaced share");
  });

  it("announces rights inside the encrypted share and reads an owner without them as older", async () => {
    const t = await team();
    t.alice.own(shared, false, { send: ["bob", "privatemarkerperson"], control: [] });
    await t.send("alice", { t: "share", share: shared });
    expect(t.bob.rightsOf(shared.id)).toEqual({ send: ["bob", "privatemarkerperson"], control: [] });
    expect(JSON.stringify(t.wire)).not.toContain("privatemarkerperson");
    // An owner from before rights seals the same announcement without the field.
    const id = crypto.randomUUID();
    const plain = { ...shared, audience: ["bob"] };
    const box = await seal(t.alice.security.identity, t.bob.security.identity.publicKey, ["organization", "alice", "bob", id],
      encoder.encode(JSON.stringify({ frame: { t: "share", share: plain }, revision: 99 })));
    const wire = { ...plain, title: "", repo_name: "", branch: "", stage: "", issue: null,
      tabs: plain.tabs.map(tab => ({ ...tab, title: "", note: null, tokens: null, status: "desligada" as const })),
      encrypted: { id, boxes: { bob: box } }, owner: "alice", online: true };
    await t.bob.incoming({ t: "share", share: wire });
    expect(t.bob.shares.get(shared.id)?.title).toBe(shared.title);
    expect(t.bob.rightsOf(shared.id)).toBeUndefined();
  });

  it("rejects comments and resolutions from outsiders even with valid encryption", async () => {
    const t = await team();
    await t.send("alice", { t: "share", share: shared });
    await t.send("alice", { t: "note", ws: shared.id, tab: "tab", text: privateText, mentions: ["bob"], quote: null });
    const root = t.relay.notes.get(shared.id)![0];
    async function outsider(frame: Up) {
      const id = crypto.randomUUID();
      const box = await seal(t.carol.security.identity, t.bob.security.identity.publicKey,
        ["organization", "carol", "bob", id], encoder.encode(JSON.stringify({ frame })));
      return { id, boxes: { bob: box } };
    }
    const encrypted = await outsider({ t: "note", ws: shared.id, tab: "tab", text: "forged", mentions: [], quote: null });
    const forged: Down = { t: "note", note: { ...root, id: encrypted.id, author: "carol", encrypted } };
    await expect(t.bob.incoming(forged)).rejects.toThrow("Outside encrypted audience");
    const resolution = await outsider({ t: "note_resolve", ws: shared.id, note: root.id });
    const resolved: Down = { t: "note", note: { ...root, resolved: true, resolution: { author: "carol", encrypted: resolution } } };
    await expect(t.bob.incoming(downForMember(resolved, "bob"))).rejects.toThrow("Outside encrypted audience");
    expect(await t.bob.incoming({ t: "inbox", items: [{ id: encrypted.id, ws: shared.id, author: "carol", ts: Date.now(), encrypted }] }))
      .toEqual({ t: "inbox", items: [] });
  });
});
