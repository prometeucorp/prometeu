import { describe, expect, it, vi } from "vitest";
import { generateIdentity } from "./team-crypto";
import { TeamSecurity } from "./team-security";

function storage(initial: unknown = null) {
  let value = initial;
  let fail = false;
  let writes = 0;
  return {
    read: async () => structuredClone(value),
    write: async (next: unknown) => {
      if (fail) throw new Error("Storage unavailable");
      value = structuredClone(next);
      writes++;
    },
    fail: (next: boolean) => { fail = next; },
    writes: () => writes,
  };
}

describe("team identities and initial trust", () => {
  it("persists replay state before use and rejects clock rollback and write failures", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
    try {
      disk.fail(true);
      await expect(security.consume("message", 1_060_000)).rejects.toThrow("Storage unavailable");
      disk.fail(false);
      await security.consume("message", 1_060_000);
      const reopened = await TeamSecurity.load("team", disk.read, disk.write);
      await expect(reopened.consume("message", 1_060_000)).rejects.toThrow("Expired or repeated input");
      await expect(reopened.consume("too-late", 999_999)).rejects.toThrow();
      await expect(reopened.consume("too-early", 1_120_001)).rejects.toThrow();
      now.mockReturnValue(999_999);
      await expect(reopened.consume("rollback", 1_060_000)).rejects.toThrow();
      now.mockReturnValue(1_070_000);
      await reopened.consume("next", 1_080_000);
      expect(JSON.stringify(await disk.read())).not.toContain('"message"');
    } finally { now.mockRestore(); }
  });

  it("preserves share sequence and ownership without advancing revisions after failed writes", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const bob = await generateIdentity();
    expect(await security.nextRevision()).toBe(1);
    await security.observeShare("ws", "bob", bob.publicKey, 2, "revision-two");
    const reopened = await TeamSecurity.load("team", disk.read, disk.write);
    expect(await reopened.nextRevision()).toBe(2);
    await reopened.observeShare("ws", "bob", bob.publicKey, 2, "revision-two");
    await expect(reopened.observeShare("ws", "bob", bob.publicKey, 1, "old")).rejects.toThrow();
    await expect(reopened.observeShare("ws", "bob", bob.publicKey, 2, "forged")).rejects.toThrow();
    await expect(reopened.observeShare("ws", "eve", bob.publicKey, 3, "replacement")).rejects.toThrow();
    disk.fail(true);
    await expect(reopened.observeShare("ws", "bob", bob.publicKey, 3, "next")).rejects.toThrow("Storage unavailable");
    disk.fail(false);
    await reopened.observeShare("ws", "bob", bob.publicKey, 2, "revision-two");
    await reopened.observeShare("ws", "bob", bob.publicKey, 3, "next");
    await expect(reopened.observeShare("ws", "bob", bob.publicKey, 2, "revision-two")).rejects.toThrow();
  });

  it("persists identity and bindings before exposing them and preserves other organizations", async () => {
    const disk = storage();
    const bob = await generateIdentity();
    const alpha = await TeamSecurity.load("alpha", disk.read, disk.write);
    expect(disk.writes()).toBe(1);
    await alpha.observe([{ id: "bob", key: bob.publicKey }], "alice");
    expect(alpha.key("bob")).toBe(bob.publicKey);
    const beta = await TeamSecurity.load("beta", disk.read, disk.write);
    expect(beta.identity).not.toEqual(alpha.identity);
    const reopened = await TeamSecurity.load("alpha", disk.read, disk.write);
    expect(reopened.identity).toEqual(alpha.identity);
    expect(reopened.key("bob")).toBeUndefined();
    await reopened.observe([{ id: "bob", key: bob.publicKey }], "alice");
    expect(reopened.key("bob")).toBe(bob.publicKey);
    expect((await TeamSecurity.load("beta", disk.read, disk.write)).identity).toEqual(beta.identity);
  });

  it("adopts a peer's replacement key only after persisting it", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const old = await generateIdentity();
    const next = await generateIdentity();
    await security.observe([{ id: "bob", key: old.publicKey }], "alice");
    disk.fail(true);
    await expect(security.observe([{ id: "bob", key: next.publicKey }], "alice")).rejects.toThrow("Storage unavailable");
    expect(security.key("bob")).toBeUndefined();
    disk.fail(false);
    await security.observe([{ id: "bob", key: next.publicKey }], "alice");
    expect(security.key("bob")).toBe(next.publicKey);
    const reopened = await TeamSecurity.load("team", disk.read, disk.write);
    await reopened.observe([{ id: "bob", key: next.publicKey }], "alice");
    expect(reopened.key("bob")).toBe(next.publicKey);
    // Adoption follows the directory in both directions: a key seen again replaces the pin again.
    await reopened.observe([{ id: "bob", key: old.publicKey }], "alice");
    expect(reopened.key("bob")).toBe(old.publicKey);
  });

  it("blocks missing keys without removing bindings and rejects replacement of its own identity", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const bob = await generateIdentity();
    await security.observe([{ id: "bob", key: bob.publicKey }, { id: "alice", key: security.identity.publicKey }], "alice");
    expect(security.key("alice")).toBe(security.identity.publicKey);
    await security.observe([{ id: "bob" }], "alice");
    expect(security.key("bob")).toBeUndefined();
    expect(security.key("alice")).toBeUndefined();
    await security.observe([], "alice");
    expect(security.key("bob")).toBeUndefined();
    await security.observe([{ id: "bob", key: bob.publicKey }], "alice");
    expect(security.key("bob")).toBe(bob.publicKey);
    await expect(security.observe([{ id: "alice", key: bob.publicKey }], "alice")).rejects.toThrow("Own identity key changed");
    expect(security.key("alice")).toBeUndefined();
  });

  it("does not expose the first key when persistence fails", async () => {
    const disk = storage();
    disk.fail(true);
    await expect(TeamSecurity.load("team", disk.read, disk.write)).rejects.toThrow("Storage unavailable");
    disk.fail(false);
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const bob = await generateIdentity();
    disk.fail(true);
    await expect(security.observe([{ id: "bob", key: bob.publicKey }], "alice")).rejects.toThrow("Storage unavailable");
    expect(security.key("bob")).toBeUndefined();
    disk.fail(false);
    await security.observe([{ id: "bob", key: bob.publicKey }], "alice");
    expect(security.key("bob")).toBe(bob.publicKey);
  });

  it("rejects corrupt storage without regenerating identity", async () => {
    const identity = await generateIdentity();
    const invalid = [false, [], {}, { version: 2, scopes: {} }, { version: 1, scopes: [] },
      { version: 1, scopes: { team: null } },
      { version: 1, scopes: { team: { identity: {}, peers: {} } } },
      { version: 1, scopes: { team: { identity, peers: { bob: "bad" } } } },
      { version: 1, scopes: { team: { identity, peers: Object.fromEntries([["__proto__", identity.publicKey]]) } } },
      { version: 1, scopes: { team: { identity, peers: Object.fromEntries(Array.from({ length: 65 }, (_, i) => [`peer${i}`, identity.publicKey])) } } },
    ];
    for (const value of invalid) {
      const disk = storage(value);
      await expect(TeamSecurity.load("team", disk.read, disk.write)).rejects.toThrow();
      expect(disk.writes()).toBe(0);
    }
  });

  it("serializes concurrent observations without losing bindings", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const bob = await generateIdentity();
    const carol = await generateIdentity();
    await Promise.all([
      security.observe([{ id: "bob", key: bob.publicKey }], "alice"),
      security.observe([{ id: "carol", key: carol.publicKey }], "alice"),
    ]);
    const reopened = await TeamSecurity.load("team", disk.read, disk.write);
    const impostor = await generateIdentity();
    await reopened.observe([{ id: "bob", key: impostor.publicKey }, { id: "carol", key: carol.publicKey }], "alice");
    expect(reopened.key("bob")).toBe(impostor.publicKey);
    expect(reopened.key("carol")).toBe(carol.publicKey);
  });

  it("rejects invalid directories without exposing observed bindings", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const bob = await generateIdentity();
    const next = await generateIdentity();
    await security.observe([{ id: "bob", key: bob.publicKey }], "alice");
    await expect(security.observe([{ id: "bob", key: "bad" }], "alice")).rejects.toThrow();
    expect(security.key("bob")).toBeUndefined();
    await expect(security.observe([{ id: "bob" }, { id: "bob" }], "alice")).rejects.toThrow();
    await security.observe([{ id: "bob", key: next.publicKey }], "alice");
    expect(security.key("bob")).toBe(next.publicKey);
    await security.observe([], "alice");
    expect(security.key("bob")).toBeUndefined();
  });
});

describe("remote input from approved identities", () => {
  it("approves the keys pinned before the upgrade and pauses a later replacement", async () => {
    const identity = await generateIdentity();
    const bob = await generateIdentity();
    const next = await generateIdentity();
    // A file written by the previous version: links, but no approvals.
    const disk = storage({ version: 1, scopes: { team: { identity, peers: { bob: bob.publicKey } } } });
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    await security.observe([{ id: "bob", key: bob.publicKey }], "alice");
    expect(security.approved("bob")).toBe(true);
    await security.observe([{ id: "bob", key: next.publicKey }], "alice");
    // Content keeps the new key (ADR 0042); only input waits for the owner.
    expect(security.key("bob")).toBe(next.publicKey);
    expect(security.approved("bob")).toBe(false);
    const reopened = await TeamSecurity.load("team", disk.read, disk.write);
    await reopened.observe([{ id: "bob", key: next.publicKey }], "alice");
    expect(reopened.approved("bob")).toBe(false);
  });

  it("approves nobody on first contact and persists an approval before using it", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const bob = await generateIdentity();
    await security.observe([{ id: "bob", key: bob.publicKey }, { id: "alice", key: security.identity.publicKey }], "alice");
    expect(security.approved("alice")).toBe(true);
    expect(security.approved("bob")).toBe(false);
    disk.fail(true);
    await expect(security.approve(["bob"])).rejects.toThrow("Storage unavailable");
    expect(security.approved("bob")).toBe(false);
    disk.fail(false);
    expect(await security.approve(["bob", "nobody"])).toBe(1);
    expect(security.approved("bob")).toBe(true);
    expect(await security.approve(["bob"])).toBe(0);
    const reopened = await TeamSecurity.load("team", disk.read, disk.write);
    expect(reopened.approved("bob")).toBe(false);
    await reopened.observe([{ id: "bob", key: bob.publicKey }], "alice");
    expect(reopened.approved("bob")).toBe(true);
  });

  it("approves one key per member, so a key seen again needs approval again", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const old = await generateIdentity();
    const next = await generateIdentity();
    await security.observe([{ id: "bob", key: old.publicKey }], "alice");
    await security.approve(["bob"]);
    await security.observe([{ id: "bob", key: next.publicKey }], "alice");
    await security.approve(["bob"]);
    expect(security.approved("bob")).toBe(true);
    await security.observe([{ id: "bob", key: old.publicKey }], "alice");
    expect(security.approved("bob")).toBe(false);
    await security.observe([{ id: "bob" }], "alice");
    expect(security.approved("bob")).toBe(false);
  });

  it("remembers paused input per workspace across restarts until approval or dismissal", async () => {
    const disk = storage();
    const security = await TeamSecurity.load("team", disk.read, disk.write);
    const bob = await generateIdentity();
    const carol = await generateIdentity();
    await security.observe([{ id: "bob", key: bob.publicKey }, { id: "carol", key: carol.publicKey }], "alice");
    expect(await security.pause("bob", "ws1")).toBe(true);
    expect(await security.pause("bob", "ws1")).toBe(false);
    expect(await security.pause("bob", "ws2")).toBe(true);
    expect(await security.pause("carol", "ws1")).toBe(true);
    const reopened = await TeamSecurity.load("team", disk.read, disk.write);
    expect(reopened.paused()).toEqual({});
    await reopened.observe([{ id: "bob", key: bob.publicKey }, { id: "carol", key: carol.publicKey }], "alice");
    expect(reopened.paused()).toEqual({ bob: ["ws1", "ws2"], carol: ["ws1"] });
    await reopened.approve(["bob"]);
    await reopened.dismiss(["carol"]);
    expect(reopened.paused()).toEqual({});
    expect(reopened.approved("carol")).toBe(false);
    // An approved identity never records a pause.
    expect(await reopened.pause("bob", "ws1")).toBe(false);
    expect((await TeamSecurity.load("team", disk.read, disk.write)).paused()).toEqual({});
  });

  it("rejects corrupt approvals and pauses without regenerating identity", async () => {
    const identity = await generateIdentity();
    const scope = (extra: Record<string, unknown>) => ({ version: 1, scopes: { team: { identity, peers: {}, ...extra } } });
    const invalid = [
      scope({ approved: [] }),
      scope({ approved: { bob: "bad" } }),
      scope({ approved: Object.fromEntries([["__proto__", identity.publicKey]]) }),
      scope({ approved: Object.fromEntries(Array.from({ length: 65 }, (_, i) => [`peer${i}`, identity.publicKey])) }),
      scope({ paused: { bob: "ws1" } }),
      scope({ paused: { bob: ["not an id"] } }),
      scope({ paused: { bob: Array.from({ length: 65 }, (_, i) => `ws${i}`) } }),
    ];
    for (const value of invalid) {
      const disk = storage(value);
      await expect(TeamSecurity.load("team", disk.read, disk.write)).rejects.toThrow();
      expect(disk.writes()).toBe(0);
    }
  });
});
