import { createServer, request, type Server } from "node:http";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createTestHarness } from "wrangler";
import { encryptedBinary, encodeLive, encodeSnapshot, parseCreatedTeam, parseMembership, PROTO, type CreatedTeam, type Encrypted, type Share } from "./protocol";
import { generateIdentity, open, seal, signIdentity, type Identity } from "../../src/team-crypto";

async function startRelay(vars?: Record<string, string>) {
  const server = createTestHarness({ workers: [{ configPath: "relay/wrangler.toml", vars }] });
  const { url } = await server.listen();
  return {
    address: url.hostname,
    port: Number(url.port),
    // Dispatch HTTP directly to workerd: Wrangler's dev proxy loses its upstream connection after
    // a streamed body is canceled. WebSocket probes still use the listening server's URL.
    fetch: server.getWorker().fetch,
    stop: () => server.close(),
  };
}

let worker: Awaited<ReturnType<typeof startRelay>>;
let created: CreatedTeam;

// Closing under a loaded GitHub-hosted runner can exceed expect.poll's 1 s default.
const closed = (socket: WebSocket) =>
  expect.poll(() => socket.readyState, { timeout: 15_000 }).toBe(WebSocket.CLOSED);

// Assert the HTTP rejection itself. Network failures and 500 responses must never count as denied
// credentials; each probe uses a dedicated connection and consumes its response.
const upgradeStatus = (url: string): Promise<number> =>
  new Promise((resolve, reject) => {
    const target = new URL(url);
    target.protocol = "http:";
    const req = request(target.toString(), { agent: false, headers: {
      Connection: "Upgrade", Upgrade: "websocket",
      "Sec-WebSocket-Version": "13", "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==",
    } });
    req.on("response", response => {
      response.on("error", reject);
      let body = "";
      response.on("data", chunk => { body += chunk.toString(); });
      response.on("end", () => {
        if (response.statusCode! >= 500) reject(new Error(`HTTP ${response.statusCode}: ${body}`));
        else resolve(response.statusCode!);
      });
    });
    req.on("upgrade", (response, socket) => {
      socket.destroy();
      resolve(response.statusCode!);
    });
    req.on("error", reject);
    req.setTimeout(5_000, () => req.destroy(new Error("upgrade response timeout")));
    req.end();
  });

const socketResult = (url: string): Promise<{ open: boolean; first?: unknown }> =>
  new Promise((resolve) => {
    const socket = new WebSocket(url);
    const timer = setTimeout(() => {
      socket.close();
      resolve({ open: false });
    }, 5_000);
    socket.addEventListener("message", (event) => {
      clearTimeout(timer);
      let first: unknown = event.data;
      try {
        first = JSON.parse(String(event.data));
      } catch {
        // Keep the first protocol frame raw so failed JSON assertions expose incompatible runtime
        // responses.
      }
      socket.close();
      resolve({ open: true, first });
    });
    // WebSocket errors are followed by close; resolving only here prevents the next connection from
    // racing Durable Object cleanup on a loaded runner.
    socket.addEventListener("close", () => {
      clearTimeout(timer);
      resolve({ open: false });
    });
  });

describe("relay in the Worker runtime", () => {
  beforeAll(async () => {
    worker = await startRelay();
    const response = await worker.fetch("/teams", { method: "POST" });
    expect(response.status).toBe(200);
    const parsed = parseCreatedTeam(await response.json());
    expect(parsed).not.toBeNull();
    created = parsed!;
  }, 30_000);

  afterAll(async () => {
    await worker?.stop();
  });

  it("exchanges invitations for individual credentials", async () => {
    const denied = await worker.fetch(`/team/${created.team}/enroll`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ secret: "segredo-errado-123456" }),
    });
    expect(denied.status).toBe(401);

    const response = await worker.fetch(`/team/${created.team}/enroll`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ secret: created.secret }),
    });
    expect(response.status).toBe(200);
    const membership = parseMembership(await response.json());
    expect(membership).not.toBeNull();
    expect(membership?.member).not.toBe(created.member);
    expect(membership?.credential).not.toBe(created.credential);
  });

  it("rejects the legacy shared secret on WebSocket connections", async () => {
    const base = `ws://${worker.address}:${worker.port}`;
    const old = await upgradeStatus(`${base}/team/${created.team}?s=${created.secret}&m=${created.member}&n=Alice&p=${PROTO}`);
    expect(old).toBe(401);
  });

  it("rejects oversized enrollment streams without breaking the next request", async () => {
    const bytes = new TextEncoder().encode(JSON.stringify({ secret: created.secret, padding: "x".repeat(4096) }));
    let offset = 0;
    const body = new ReadableStream<Uint8Array>({
      pull(controller) {
        if (offset >= bytes.length) return controller.close();
        controller.enqueue(bytes.slice(offset, offset + 256));
        offset += 256;
      },
    });
    const request = { method: "POST", body, duplex: "half" as const };
    const response = await worker.fetch(`/team/${created.team}/enroll`, request);
    expect(response.status).toBe(413);
    expect(await response.text()).toBe("too big");
    const next = await worker.fetch(`/team/${created.team}/enroll`, {
      // Preserve the bounded source: reserializing these ignored numbers would exceed 1 KiB.
      method: "POST", body: `{"secret":"${created.secret}","padding":[${Array(60).fill("1e20").join(",")}]}`,
    });
    const nextBody = await next.text();
    expect(next.status, nextBody).toBe(200);
    expect(parseMembership(JSON.parse(nextBody))).not.toBeNull();
  });

  it("binds credentials to members and delivers welcome", async () => {
    const base = `ws://${worker.address}:${worker.port}`;
    const swapped = await upgradeStatus(`${base}/team/${created.team}?c=${created.credential}&m=outro_membro&n=Eve&p=${PROTO}`);
    expect(swapped).toBe(401);

    const ok = await socketResult(`${base}/team/${created.team}?c=${created.credential}&m=${created.member}&n=Alice&p=${PROTO}`);
    expect(ok).toMatchObject({ open: true, first: { t: "welcome", you: created.member, e2ee: 1, challenge: expect.any(String) } });
  });

  it("rejects protocol 3 before accepting a socket", async () => {
    const base = `ws://${worker.address}:${worker.port}`;
    expect(await upgradeStatus(`${base}/team/${created.team}?c=${created.credential}&m=${created.member}&p=3`)).toBe(426);
  });
});


describe("organization sharing through Cloud authorization", () => {
  let cloud: Server;
  let relay: Awaited<ReturnType<typeof startRelay>>;
  const tickets = new Map<string, { organization: string; member: string; name: string; lifetime: number }>();
  const members = [{ id: "owner001", name: "Alice" }, { id: "guest001", name: "Bob" }];
  const sockets: WebSocket[] = [];
  const identities = new Map<string, Identity>();
  async function encrypted(sender: Identity, recipients: string[], text: string): Promise<Encrypted> {
    const id = crypto.randomUUID();
    const boxes = Object.fromEntries(await Promise.all(recipients.map(async member => [
      member, await seal(sender, identities.get(member)!.publicKey, ["worker-test", id], new TextEncoder().encode(text)),
    ])));
    return { id, boxes };
  }
  async function decrypted(recipient: Identity, sender: Identity, value: Encrypted, member: string): Promise<string> {
    return new TextDecoder().decode(await open(recipient, sender.publicKey, ["worker-test", value.id], value.boxes[member]));
  }
  async function share(identity: Identity): Promise<Share> {
    return { id: "workspace1", title: "", repo_name: "", branch: "", stage: "", issue: null,
      active: "tab1", tabs: [{ id: "tab1", title: "", status: "desligada", note: null, tokens: null }],
      sizes: { tab1: [80, 24] }, audience: ["guest001"],
      encrypted: await encrypted(identity, ["owner001", "guest001"], "Private workspace metadata") };
  }
  const issue = (char: string, member: number, lifetime = 60_000) => {
    const ticket = char.repeat(43);
    tickets.set(ticket, { organization: "organization1", member: members[member].id, name: members[member].name, lifetime });
    return ticket;
  };
  beforeAll(async () => {
    cloud = createServer(async (request, response) => {
      expect(request.url).toBe("/api/relay/authorize");
      expect(request.headers.authorization).toBeUndefined();
      const chunks: Uint8Array[] = [];
      for await (const chunk of request) chunks.push(chunk);
      const input = JSON.parse(Buffer.concat(chunks).toString());
      const access = tickets.get(input.ticket);
      response.setHeader("Content-Type", "application/json");
      if (!access || access.organization !== input.organization) {
        response.writeHead(401); response.end('{}'); return;
      }
      tickets.delete(input.ticket);
      response.end(JSON.stringify({ ...access, expires_at: Date.now() + access.lifetime, members }));
    });
    await new Promise<void>(resolve => cloud.listen(0, "127.0.0.1", resolve));
    const address = cloud.address() as { port: number };
    relay = await startRelay({ CLOUD_URL: `http://127.0.0.1:${address.port}` });
  }, 30_000);
  afterAll(async () => {
    sockets.forEach(socket => socket.close());
    await relay?.stop();
    await new Promise<void>(resolve => cloud?.close(() => resolve()));
  });
  const base = () => `ws://${relay.address}:${relay.port}`;
  async function connect(ticket: string, spoofed = "spoofed", authenticate = true) {
    const socket = new WebSocket(`${base()}/organization/organization1?ticket=${ticket}&m=${spoofed}&n=Impersonated&p=${PROTO}`);
    socket.binaryType = "arraybuffer";
    sockets.push(socket);
    const frames: any[] = [];
    const binaries: ArrayBuffer[] = [];
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("missing welcome")), 5_000);
      socket.addEventListener("message", event => {
        if (typeof event.data === "string") {
          const frame = JSON.parse(event.data); frames.push(frame);
          if (frame.t === "welcome") { clearTimeout(timer); resolve(); }
        } else binaries.push(event.data);
      });
      socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("socket rejected")); });
    });
    const welcome = frames[0];
    expect(welcome).toMatchObject({ t: "welcome", e2ee: 1, challenge: expect.any(String) });
    const member = welcome.you as string;
    const identity = identities.get(member) ?? await generateIdentity();
    identities.set(member, identity);
    if (authenticate) {
      socket.send(JSON.stringify({ t: "identity", key: identity.publicKey, proof: await signIdentity(identity, [member, welcome.challenge]) }));
      await expect.poll(() => frames.some(frame => frame.t === "presence" && frame.members.some((item: { id: string; key?: string }) => item.id === member && item.key === identity.publicKey))).toBe(true);
    }
    return { socket, frames, binaries, identity, member };
  }
  it("isolates organizations and refuses anonymous or legacy enrollment", async () => {
    const ticket = issue("a", 0);
    expect(await upgradeStatus(`${base()}/organization/organization2?ticket=${ticket}&p=${PROTO}`)).toBe(401);
    expect(await upgradeStatus(`${base()}/organization/organization1?c=${ticket}&m=owner001&p=${PROTO}`)).toBe(401);
    expect((await relay.fetch("/organization/organization1/enroll", { method: "POST" })).status).toBe(404);
  });
  it("shares only with accepted members, ignores spoofed identity and expires active access", async () => {
    const owner = await connect(issue("b", 0));
    const guestTicket = issue("c", 1, 2_000);
    const guest = await connect(guestTicket, "owner001");
    expect(guest.frames[0]).toMatchObject({ t: "welcome", you: "guest001", members: expect.arrayContaining([{ id: "guest001", name: "Bob", online: true }]) });
    expect(await upgradeStatus(`${base()}/organization/organization1?ticket=${guestTicket}&p=${PROTO}`)).toBe(401);
    guest.socket.send(JSON.stringify({ t: "me", name: "Alice" }));
    owner.socket.send(JSON.stringify({ t: "share", share: await share(owner.identity) }));
    await expect.poll(() => guest.frames.some(frame => frame.t === "share")).toBe(true);
    guest.socket.send(JSON.stringify({ t: "attach", ws: "workspace1", tab: "tab1" }));
    await expect.poll(() => owner.frames.some(frame => frame.t === "watch" && frame.members.includes("guest001"))).toBe(true);
    guest.socket.send(JSON.stringify({ t: "write", ws: "workspace1", tab: "tab1", data: "", encrypted: await encrypted(guest.identity, ["owner001"], "Please check this") }));
    await expect.poll(() => owner.frames.some(frame => frame.t === "write" && frame.from === "guest001")).toBe(true);
    const write = owner.frames.find(frame => frame.t === "write");
    expect(write.data).toBe("");
    expect(await decrypted(owner.identity, guest.identity, write.encrypted, owner.member)).toBe("Please check this");
    await closed(guest.socket);
    const presence = owner.frames.filter(frame => frame.t === "presence").at(-1);
    expect(presence.members.find((member: { id: string }) => member.id === "guest001").name).toBe("Bob");
    expect(await upgradeStatus(`${base()}/organization/organization1?ticket=${"d".repeat(43)}&p=${PROTO}`)).toBe(401);
    owner.socket.close();
    await closed(owner.socket);
  });

  it("persists and routes only recipient ciphertext for comments, snapshots and inbox", async () => {
    const owner = await connect(issue("e", 0));
    const guest = await connect(issue("f", 1));
    owner.socket.send(JSON.stringify({ t: "share", share: await share(owner.identity) }));
    await expect.poll(() => guest.frames.some(frame => frame.t === "share")).toBe(true);
    const metadata = guest.frames.find(frame => frame.t === "share").share;
    expect(metadata.title).toBe("");
    expect(Object.keys(metadata.encrypted.boxes)).toEqual([guest.member]);
    expect(await decrypted(guest.identity, owner.identity, metadata.encrypted, guest.member)).toBe("Private workspace metadata");
    guest.socket.send(JSON.stringify({ t: "attach", ws: "workspace1", tab: "tab1" }));
    await expect.poll(() => owner.frames.some(frame => frame.t === "watch" && frame.members.includes(guest.member))).toBe(true);
    const stream = await encrypted(owner.identity, [guest.member], "Private transcript");
    owner.socket.send(encodeSnapshot("tab1", guest.member, 0, new TextEncoder().encode(JSON.stringify(stream))));
    await expect.poll(() => guest.binaries.length).toBe(1);
    const binary = encryptedBinary(guest.binaries[0]);
    expect(binary).not.toBeNull();
    expect(await decrypted(guest.identity, owner.identity, binary!.encrypted, guest.member)).toBe("Private transcript");
    const body = await encrypted(owner.identity, [owner.member, guest.member], "Private comment");
    owner.socket.send(JSON.stringify({ t: "note", ws: "workspace1", tab: "tab1", text: "", quote: null, mentions: [guest.member], encrypted: body }));
    await expect.poll(() => guest.frames.some(frame => frame.t === "note")).toBe(true);
    const note = guest.frames.find(frame => frame.t === "note").note;
    expect(note.text).toBe("");
    expect(Object.keys(note.encrypted.boxes)).toEqual([guest.member]);
    expect(await decrypted(guest.identity, owner.identity, note.encrypted, guest.member)).toBe("Private comment");
    guest.socket.close();
    await closed(guest.socket);
    const returned = await connect(issue("g", 1));
    expect(returned.frames[0].inbox).toHaveLength(1);
    expect(Object.keys(returned.frames[0].inbox[0].encrypted.boxes)).toEqual([guest.member]);
    expect(await decrypted(guest.identity, owner.identity, returned.frames[0].inbox[0].encrypted, guest.member)).toBe("Private comment");
    returned.socket.send(JSON.stringify({ t: "notes", ws: "workspace1" }));
    await expect.poll(() => returned.frames.some(frame => frame.t === "notes")).toBe(true);
    const stored = returned.frames.find(frame => frame.t === "notes").items[0];
    expect(stored.id).toBe(note.id);
    expect(stored.encrypted).toEqual(note.encrypted);
    expect(JSON.stringify(returned.frames)).not.toContain("Private comment");
    // Finish each close handshake before starting the next through the dev proxy.
    owner.socket.close();
    await closed(owner.socket);
    returned.socket.close();
    await closed(returned.socket);
  });

  it("rejects forged identity proof and plaintext text or binary content", async () => {
    const forged = await connect(issue("h", 0), "spoofed", false);
    forged.socket.send(JSON.stringify({ t: "identity", key: forged.identity.publicKey,
      proof: await signIdentity(forged.identity, [forged.member, "wrong-challenge"]) }));
    await closed(forged.socket);
    const plaintext = await connect(issue("i", 0));
    plaintext.socket.send(JSON.stringify({ t: "write", ws: "workspace1", tab: "tab1", data: "Unencrypted input" }));
    await closed(plaintext.socket);
    const binary = await connect(issue("j", 0));
    binary.socket.send(encodeLive("tab1", [{ seq: 1, bytes: new TextEncoder().encode("Unencrypted output") }]));
    await closed(binary.socket);
  });

  it("renews an identified socket without losing its watcher or broadcasting presence, then expires the renewed lease", async () => {
    const owner = await connect(issue("k", 0));
    const guest = await connect(issue("l", 1, 2_000));
    const originalDeadline = Date.now() + 2_000;
    owner.socket.send(JSON.stringify({ t: "share", share: await share(owner.identity) }));
    await expect.poll(() => guest.frames.some(frame => frame.t === "share")).toBe(true);
    guest.socket.send(JSON.stringify({ t: "attach", ws: "workspace1", tab: "tab1" }));
    await expect.poll(() => owner.frames.some(frame => frame.t === "watch" && frame.members.includes(guest.member))).toBe(true);
    const presenceCount = owner.frames.filter(frame => frame.t === "presence").length;
    const watchCount = owner.frames.filter(frame => frame.t === "watch").length;
    guest.socket.send(JSON.stringify({ t: "renew", ticket: issue("m", 1, 4_000) }));
    await expect.poll(() => guest.frames.filter(frame => frame.t === "lease").length).toBe(2);
    await expect.poll(() => Date.now() >= originalDeadline, { timeout: 4_000 }).toBe(true);
    expect(guest.socket.readyState).toBe(WebSocket.OPEN);
    expect(guest.frames.filter(frame => frame.t === "welcome")).toHaveLength(1);
    expect(owner.frames.filter(frame => frame.t === "presence")).toHaveLength(presenceCount);
    expect(owner.frames.filter(frame => frame.t === "watch")).toHaveLength(watchCount);
    const stream = await encrypted(owner.identity, [guest.member], "After renewal");
    owner.socket.send(encodeSnapshot("tab1", guest.member, 0, new TextEncoder().encode(JSON.stringify(stream))));
    await expect.poll(() => guest.binaries.length).toBe(1);
    await closed(guest.socket);
    owner.socket.close();
    await closed(owner.socket);
  });

  it("rejects renewal before identity, with another member or organization, and with a spent or revoked ticket", async () => {
    for (const kind of ["unidentified", "other-member", "other-organization", "spent", "revoked"] as const) {
      const guest = await connect(issue("n", 1), "spoofed", kind !== "unidentified");
      const ticket = issue("o", kind === "other-member" ? 0 : 1);
      if (kind === "other-organization") tickets.get(ticket)!.organization = "organization2";
      if (kind === "revoked") tickets.delete(ticket);
      guest.socket.send(JSON.stringify({ t: "renew", ticket }));
      if (kind === "spent") {
        await expect.poll(() => guest.frames.filter(frame => frame.t === "lease").length).toBe(2);
        guest.socket.send(JSON.stringify({ t: "renew", ticket }));
      }
      await closed(guest.socket);
    }
  });
});
