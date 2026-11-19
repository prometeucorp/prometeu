import { describe, expect, it } from "vitest";
import {
  BINARY_FRAME_MAX,
  LIVE,
  SNAPSHOT,
  decodeBinary,
  encodeLive,
  encodeSnapshot,
  formatInvite,
  parseCreatedTeam,
  parseDown,
  parseInvite,
  parseMembership,
  parseUp,
} from "./protocol";

const bytes = (s: string) => new TextEncoder().encode(s);
const text = (b: Uint8Array) => new TextDecoder().decode(b);

describe("binary frames", () => {
  it("round-trips multiple live chunks with their sequence numbers", () => {
    const frame = encodeLive("tab-1", [
      { seq: 41, bytes: bytes("olá") },
      { seq: 42, bytes: new Uint8Array(0) },
      { seq: 2 ** 40 + 3, bytes: bytes("\x1b[2J") },
    ]);
    const got = decodeBinary(frame);
    expect(got?.kind).toBe(LIVE);
    if (got?.kind !== LIVE) return;
    expect(got.tab).toBe("tab-1");
    expect(got.segments.map((s) => [s.seq, text(s.bytes)])).toEqual([
      [41, "olá"],
      [42, ""],
      [2 ** 40 + 3, "\x1b[2J"],
    ]);
  });

  it("round-trips snapshot tab, recipient, sequence number and bytes", () => {
    const got = decodeBinary(encodeSnapshot("t", "bob", 9, bytes("full")));
    expect(got).toMatchObject({ kind: SNAPSHOT, tab: "t", to: "bob", seq: 9, more: false });
    if (got?.kind === SNAPSHOT) expect(text(got.bytes)).toBe("full");
    const part = decodeBinary(encodeSnapshot("t", "bob", 9, bytes("meta"), true));
    expect(part).toMatchObject({ kind: SNAPSHOT, more: true });
  });

  it("decodes an offset ArrayBuffer as delivered by WebSocket", () => {
    const frame = encodeLive("t", [{ seq: 1, bytes: bytes("x") }]);
    const padded = new Uint8Array(frame.length + 4);
    padded.set(frame, 4);
    const got = decodeBinary(padded.subarray(4));
    expect(got?.kind).toBe(LIVE);
  });

  it("rejects malformed frames", () => {
    expect(decodeBinary(new Uint8Array([]))).toBeNull();
    expect(decodeBinary(new Uint8Array([0, 5, 1]))).toBeNull();
    expect(decodeBinary(new Uint8Array([7, 1, 65]))).toBeNull();
    expect(decodeBinary(new Uint8Array([0, 1, 65, 0, 1]))).toBeNull();
  });

  it("rejects trailing garbage, invalid IDs, malformed UTF-8 and oversized frames", () => {
    const live = encodeLive("tab", [{ seq: 1, bytes: bytes("x") }]);
    const trailing = new Uint8Array(live.length + 1);
    trailing.set(live);
    expect(decodeBinary(trailing)).toBeNull();
    expect(decodeBinary(encodeLive("__proto__", [{ seq: 1, bytes: bytes("x") }]))).toBeNull();
    expect(decodeBinary(new Uint8Array([LIVE, 1, 0xff, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]))).toBeNull();
    expect(decodeBinary(new Uint8Array(BINARY_FRAME_MAX + 1))).toBeNull();
  });
});

describe("control frames", () => {
  const validShare = {
    id: "ws1",
    title: "Work",
    repo_name: "repo",
    branch: "main",
    stage: "Fazendo",
    issue: null,
    active: "t1",
    tabs: [{ id: "t1", title: "Agent", status: "rodando", note: null, tokens: 10 }],
    sizes: { t1: [80, 24] },
    audience: ["alice", "alice"],
  };

  it("copies and normalizes valid shares and mentions", () => {
    const got = parseUp({ t: "share", share: validShare });
    expect(got?.t).toBe("share");
    if (got?.t === "share") expect(got.share.audience).toEqual(["alice"]);
    expect(parseUp({ t: "note", ws: "ws1", text: "hello", mentions: ["alice", "zé", "alice"], quote: null })).toEqual({
      t: "note",
      ws: "ws1",
      tab: null,
      anchor: null,
      text: "hello",
      mentions: ["alice"],
      quote: null,
    });
    expect(parseUp({ t: "note", ws: "ws1", tab: "t1", anchor: "s4.0", text: "hello", mentions: [], quote: "excerpt" })).toMatchObject({ tab: "t1", anchor: "s4.0" });
    expect(parseUp({ t: "note_reply", ws: "ws1", note: "n1", text: "done", mentions: ["alice"] })).toMatchObject({ t: "note_reply", note: "n1" });
    expect(parseUp({ t: "note_resolve", ws: "ws1", note: "n1" })).toEqual({ t: "note_resolve", ws: "ws1", note: "n1" });
  });

  it("preserves optional terminal decision ownership and accepts legacy tab advertisements", () => {
    const frame = (decision_surface?: unknown) => ({ t: "share", share: { ...validShare,
      tabs: [{ ...validShare.tabs[0], ...(decision_surface === undefined ? {} : { decision_surface }) }],
    } });
    const legacy = parseUp(frame());
    expect(legacy?.t === "share" && legacy.share.tabs[0]).not.toHaveProperty("decision_surface");
    const native = parseUp(frame("terminal"));
    expect(native?.t === "share" && native.share.tabs[0].decision_surface).toBe("terminal");
    for (const invalid of ["app", false, null, {}]) expect(parseUp(frame(invalid))).toBeNull();
  });

  it("preserves optional terminal conversation mode without changing legacy announcements", () => {
    const frame = (conversation_mode?: unknown) => ({ t: "share", share: { ...validShare,
      tabs: [{ ...validShare.tabs[0], ...(conversation_mode === undefined ? {} : { conversation_mode }) }],
    } });
    const legacy = parseUp(frame());
    expect(legacy?.t === "share" && legacy.share.tabs[0]).not.toHaveProperty("conversation_mode");
    const terminal = parseUp(frame("terminal"));
    expect(terminal?.t === "share" && terminal.share.tabs[0].conversation_mode).toBe("terminal");
    for (const invalid of ["chat", "unknown", false, null, {}]) expect(parseUp(frame(invalid))).toBeNull();
  });

  it("rejects invalid nested fields, dimensions, IDs and sizes", () => {
    expect(parseUp({ t: "share", share: { ...validShare, active: "missing" } })).toBeNull();
    expect(parseUp({ t: "share", share: { ...validShare, sizes: { t1: [0, 24] } } })).toBeNull();
    expect(parseUp({ t: "share", share: { ...validShare, tabs: [{ ...validShare.tabs[0], status: "hack" }] } })).toBeNull();
    expect(parseUp({ t: "attach", ws: "__proto__", tab: "t1" })).toBeNull();
    expect(parseUp({ t: "me", name: "x".repeat(81) })).toBeNull();
    expect(parseUp({ t: "write", ws: "ws1", tab: "t1", data: "x".repeat(64 * 1024 + 1) })).toBeNull();
    expect(parseUp({ t: "note", ws: "ws1", tab: "missing!", anchor: null, text: "x", mentions: [], quote: null })).toBeNull();
    expect(parseUp({ t: "note_reply", ws: "ws1", note: "?", text: "x", mentions: [] })).toBeNull();
  });

  it("validates and bounds all responses from a configurable relay", () => {
    const shared = { ...validShare, owner: "alice", online: true };
    const welcome = {
      t: "welcome",
      you: "bob",
      members: [{ id: "bob", name: "Bob", online: true }],
      shares: [shared],
      inbox: [],
      watching: { ws1: { t1: ["bob", "bob"] } },
      comments: 1,
    };
    expect(parseDown(welcome)).toMatchObject({ t: "welcome", comments: 1, watching: { ws1: { t1: ["bob"] } } });
    expect(parseDown({ ...welcome, members: [{ id: "__proto__", name: "x", online: true }] })).toBeNull();
    expect(parseDown({ t: "write", ws: "ws1", tab: "t1", from: "alice", data: "x".repeat(64 * 1024 + 1) })).toBeNull();
    expect(parseDown({ t: "notes", ws: "ws1", items: [{ id: "n1", ws: "other", author: "alice", text: "hello", mentions: [], quote: null, ts: 1 }] })).toBeNull();
    expect(parseDown({ t: "presence", members: Array.from({ length: 65 }, (_, i) => ({ id: `m${i}`, name: "x", online: true })) })).toBeNull();
    expect(parseDown({ ...welcome, comments: 2 })).toBeNull();
    expect(parseDown({ t: "share", share: { ...shared, left: ["carol", "carol"] } })).toMatchObject({ share: { left: ["carol"] } });
    expect(parseDown({ t: "share", share: { ...shared, left: [] } })).not.toHaveProperty("share.left");
    expect(parseDown({ t: "share", share: { ...shared, left: ["__proto__"] } })).toBeNull();
    expect(parseUp({ t: "leave", ws: "ws1" })).toEqual({ t: "leave", ws: "ws1" });
    expect(parseUp({ t: "readmit", ws: "ws1", person: "bob" })).toEqual({ t: "readmit", ws: "ws1", person: "bob" });
    expect(parseUp({ t: "readmit", ws: "ws1" })).toBeNull();
    expect(parseDown({ t: "note", note: { id: "n1", ws: "ws1", author: "alice", text: "hello", mentions: [], quote: null, ts: 1, tab: "t1", anchor: "s4.0", parent: null, resolved: false } })).toMatchObject({
      note: { tab: "t1", anchor: "s4.0", parent: null, resolved: false },
    });
  });
});

describe("invitations", () => {
  it("round-trips invitation codes", () => {
    const team = "abc_-123";
    const secret = "s3gr3d0-individual-123456";
    expect(parseInvite(formatInvite(team, secret))).toEqual({ team, secret });
    expect(parseInvite(`  pm2.${team}.${secret}\n`)).toEqual({ team, secret });
  });
  it("rejects invalid invitation codes", () => {
    expect(parseInvite("pm1.abcdefgh.abcdefghijklmnop")).toBeNull();
    expect(parseInvite("pm2.a.b")).toBeNull();
    expect(parseInvite("a.b")).toBeNull();
    expect(parseInvite("pm1.a.b.c")).toBeNull();
    expect(parseInvite("")).toBeNull();
  });

  it("validates credentials returned by endpoints", () => {
    const membership = { member: "member_123", credential: "c".repeat(43) };
    expect(parseMembership(membership)).toEqual(membership);
    expect(parseMembership({ ...membership, credential: "short" })).toBeNull();
    const created = { team: "team_123", secret: "s".repeat(32), ...membership };
    expect(parseCreatedTeam(created)).toEqual(created);
    expect(parseCreatedTeam({ ...created, team: "__proto__" })).toBeNull();
  });

  it("bounds renewable leases and accepts only a single-use ticket in renewal controls", () => {
    const ticket = "t".repeat(43);
    expect(parseUp({ t: "renew", ticket, member: "spoofed" })).toEqual({ t: "renew", ticket });
    for (const value of [null, "short", "t".repeat(44), "!".repeat(43)]) expect(parseUp({ t: "renew", ticket: value })).toBeNull();
    expect(parseDown({ t: "lease", expires_in: 60_000 })).toEqual({ t: "lease", expires_in: 60_000 });
    for (const value of [null, 0, -1, 1.5, 60_001, "60000"]) expect(parseDown({ t: "lease", expires_in: value })).toBeNull();
  });
});
