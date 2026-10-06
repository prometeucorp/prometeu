import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { compareId, isRead, markAll, markRead, merge, readKey, readState, saveRead, threads, workspaceFor } from "./notification-feed";
import type { GitHubNotification, Workspace } from "./types";

const note = (id: string, extra: Partial<GitHubNotification> = {}): GitHubNotification => ({
  id, kind: "review_approved", repository: "org/app", number: 5, subject: "pr", actor: "reviewer",
  target: "review", target_id: "9", url: "https://github.com/org/app/pull/5#pullrequestreview-9", created_at: "2026-10-05T12:00:00Z", ...extra,
});

beforeEach(() => {
  const saved = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => saved.set(key, value),
  });
});
afterEach(() => vi.unstubAllGlobals());

it("orders numeric ids and keeps the newest rows without duplicates", () => {
  expect(["10", "9", "100"].sort(compareId)).toEqual(["9", "10", "100"]);
  const merged = merge([note("9"), note("10")], [note("10", { actor: "edited" }), note("11")]);
  expect(merged.map(item => item.id)).toEqual(["9", "10", "11"]);
  expect(merged[1].actor).toBe("edited");
});

it("tracks read state per account, by id and by watermark, surviving damaged storage", () => {
  let state = readState("a");
  expect(isRead(state, "1")).toBe(false);
  state = markRead(state, "3");
  expect(isRead(state, "3")).toBe(true);
  expect(isRead(state, "2")).toBe(false);
  state = markAll(state, [note("9"), note("10")]);
  expect(state).toEqual({ before: "10", ids: [] });
  expect(isRead(state, "9")).toBe(true);
  expect(isRead(state, "11")).toBe(false);
  saveRead("a", state);
  expect(readState("a")).toEqual(state);
  expect(readState("b")).toEqual({ before: "", ids: [] });
  localStorage.setItem(readKey("a"), "{broken");
  expect(readState("a")).toEqual({ before: "", ids: [] });
});

it("links a notification to the workspace that owns its PR or issue", () => {
  const workspace = { id: "w", archived: false, cleaned: false, issue: null, repos: [{ path: "/clone", pr: { number: 5 } }] } as unknown as Workspace;
  const repositories = new Map([["/clone", ["org/app"]]]);
  expect(workspaceFor(note("1"), [workspace], repositories)).toBe(workspace);
  expect(workspaceFor(note("1", { repository: "other/app" }), [workspace], repositories)).toBeUndefined();
  expect(workspaceFor(note("1"), [{ ...workspace, archived: true }], repositories)).toBeUndefined();
  expect(workspaceFor(note("1", { subject: "issue" }), [workspace], repositories)).toBeUndefined();
  const fromIssue = { ...workspace, repos: [], issue: { id: "github:org/app/issues/7", url: "https://github.com/Org/App/issues/7" } } as unknown as Workspace;
  expect(workspaceFor(note("2", { subject: "issue", number: 7 }), [fromIssue], new Map())).toBe(fromIssue);
});

it("groups notifications by issue or PR, led by the most urgent unread event", () => {
  const items = [
    note("1", { kind: "ci_failed" }),
    note("2", { kind: "commented", actor: "bot[bot]" }),
    note("3", { kind: "assigned", number: 6 }),
    note("4", { kind: "commented", repository: "Org/App" }),
  ];
  const [pr5, pr6] = threads(items, { before: "", ids: [] });
  expect(pr5.items.map(item => item.id)).toEqual(["1", "2", "4"]);
  expect(pr5.lead.id).toBe("1");
  expect(pr5.latest.id).toBe("4");
  expect(pr6.items.map(item => item.id)).toEqual(["3"]);
  // Once the failure is read, the newest unread event leads; with nothing unread, the newest.
  expect(threads(items, { before: "", ids: ["1"] })[0].lead.id).toBe("4");
  expect(threads(items, { before: "4", ids: [] })[0].lead.id).toBe("4");
});

it("keeps a whole thread read when the list of read ids is full", () => {
  const full = Array.from({ length: 500 }, (_, index) => String(index + 1));
  const state = markRead({ before: "", ids: full }, "1", "1000", "1001");
  expect(state.ids).toHaveLength(500);
  expect(["1", "1000", "1001"].every(id => isRead(state, id))).toBe(true);
  expect(isRead(state, "2")).toBe(false);
  expect(markRead({ before: "10", ids: [] }, "9", "11").ids).toEqual(["11"]);
});
