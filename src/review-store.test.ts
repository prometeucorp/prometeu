import { expect, it } from "vitest";
import { createReviewStore as makeReviewStore } from "./review-store";
import { anchorSelection, reviewBatch } from "./review-comments";
let queue = Promise.resolve();
const lock = <T>(run: () => T): Promise<T> => {
  const next = queue.then(run); queue = next.then(() => {}, async () => {}); return next;
};
const createReviewStore = (disk: Parameters<typeof makeReviewStore>[0]) => makeReviewStore(disk, lock);
const anchor = () => anchorSelection("api", "a", "changes", "@@ -0,0 +1 @@\n+x", 1, 1);
const storage = () => {
  const values = new Map<string, string>();
  return { get length() { return values.size; }, key: (i: number) => [...values.keys()][i] ?? null, getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => { values.set(k, v); }, removeItem: (k: string) => { values.delete(k); } };
};
it("persists notes and prunes only removed workspaces", async () => {
  const disk = storage(), store = createReviewStore(() => disk);
  await store.add("ws", anchor(), "Fix this"); await store.add("alive", anchor(), "Keep this");
  expect(createReviewStore(() => disk).list("ws")[0].body).toBe("Fix this");
  await store.prune(new Set(["alive"]));
  expect(disk.getItem("prometeu:review:ws")).toBeNull();
  expect(store.list("ws")).toEqual([]);
  expect(store.list("alive")).toHaveLength(1);
});
it("protects corrupt or unknown records from the next write", async () => {
  for (const raw of ['{"v":9,"comments":[]}', 'broken', '{"v":1,"comments":[{}]}']) {
    const disk = storage(); disk.setItem("prometeu:review:ws", raw);
    const store = createReviewStore(() => disk);
    expect(store.list("ws")).toEqual([]);
    await expect(store.add("ws", anchor(), "New")).rejects.toThrow("review.storage.read");
    expect(disk.getItem("prometeu:review:ws")).toBe(raw);
  }
});
it("keeps memory when writing fails and can persist it on the next edit", async () => {
  const disk = storage(), save = disk.setItem;
  disk.setItem = () => { throw new Error("quota"); };
  const store = createReviewStore(() => disk);
  await expect(store.add("ws", anchor(), "Keep my writing")).rejects.toThrow("review.storage.write");
  expect(store.list("ws")[0].body).toBe("Keep my writing");
  disk.setItem = save;
  await store.edit("ws", store.list("ws")[0].id, "Recovered");
  expect(createReviewStore(() => disk).list("ws")[0].body).toBe("Recovered");
});
it("submits only matching revisions and keeps the sent snapshot for subsequent edits", async () => {
  const disk = storage(), store = createReviewStore(() => disk);
  await store.add("ws", anchor(), "First");
  const first = store.list("ws")[0];
  const draft = reviewBatch("ws", "batch", [first], a => a.path);
  await store.edit("ws", first.id, "Second");
  await store.submit(draft, "tab");
  expect(store.list("ws")[0]).toMatchObject({ state: "draft", body: "Second", sent: { batch: "batch", tab: "tab", revision: 1 } });
  const current = reviewBatch("ws", "batch2", store.list("ws"), a => a.path);
  await store.submit(current, "tab");
  expect(store.list("ws")[0].state).toBe("sent");
  await store.state("ws", first.id, "resolved");
  await store.state("ws", first.id, "draft");
  expect(store.list("ws")[0].state).toBe("draft");
});
it("bounds note count and body bytes without changing the previous review", async () => {
  const disk = storage(), store = createReviewStore(() => disk);
  await expect(store.add("ws", anchor(), "é".repeat(4097))).rejects.toThrow("review.limit");
  expect(store.list("ws")).toHaveLength(0);
  for (let i = 0; i < 200; i++) await store.add("ws", anchor(), "Note " + i);
  await expect(store.add("ws", anchor(), "Extra")).rejects.toThrow("review.limit");
  expect(store.list("ws")).toHaveLength(200);
});
it("does not expose mutable store state to a draft or a view", async () => {
  const disk = storage(), store = createReviewStore(() => disk); await store.add("ws", anchor(), "Original");
  const notes = store.list("ws"); notes[0].body = "External change";
  expect(store.list("ws")[0].body).toBe("Original");
});

it("preserves independent notes from stores sharing the same disk", async () => {
  const disk = storage(), a = createReviewStore(() => disk), b = createReviewStore(() => disk);
  a.list("ws"); b.list("ws");
  await Promise.all([a.add("ws", anchor(), "From A"), b.add("ws", anchor(), "From B")]);
  expect(createReviewStore(() => disk).list("ws").map(n => n.body)).toEqual(["From A", "From B"]);
});

it("rejects stale edits and deletions while preserving newer revisions", async () => {
  const disk = storage(), a = createReviewStore(() => disk), b = createReviewStore(() => disk);
  await a.add("ws", anchor(), "Original");
  const note = b.list("ws")[0];
  await a.edit("ws", note.id, "Newer", note.revision);
  await expect(b.edit("ws", note.id, "Stale", note.revision)).rejects.toThrow("review.storage.conflict");
  await expect(b.remove("ws", note.id, note.revision)).rejects.toThrow("review.storage.conflict");
  expect(b.list("ws")[0].body).toBe("Newer");
  await a.remove("ws", note.id);
  await expect(b.state("ws", note.id, "resolved", note.revision)).rejects.toThrow("review.storage.conflict");
  expect(b.list("ws")).toEqual([]);
});
it("retains a failed write without overwriting subsequent writes from another window", async () => {
  const disk = storage(), write = disk.setItem, a = createReviewStore(() => disk), b = createReviewStore(() => disk);
  disk.setItem = () => { throw new Error("quota"); };
  await expect(a.add("ws", anchor(), "Unsaved")).rejects.toThrow("review.storage.write");
  disk.setItem = write;
  await b.add("ws", anchor(), "Saved elsewhere");
  await expect(a.edit("ws", a.list("ws")[0].id, "Retry")).rejects.toThrow("review.storage.conflict");
  expect(a.list("ws")[0].body).toBe("Unsaved");
  expect(b.list("ws")[0].body).toBe("Saved elsewhere");
});
