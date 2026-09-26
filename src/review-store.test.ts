import { expect, it } from "vitest";
import { createReviewStore } from "./review-store";
import { anchorSelection, reviewBatch } from "./review-comments";
const anchor = () => anchorSelection("api", "a", "changes", "@@ -0,0 +1 @@\n+x", 1, 1);
const storage = () => {
  const values = new Map<string, string>();
  return { get length() { return values.size; }, key: (i: number) => [...values.keys()][i] ?? null, getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => { values.set(k, v); }, removeItem: (k: string) => { values.delete(k); } };
};
it("persists notes and prunes only removed workspaces", () => {
  const disk = storage(), store = createReviewStore(() => disk);
  store.add("ws", anchor(), "Fix this"); store.add("alive", anchor(), "Keep this");
  expect(createReviewStore(() => disk).list("ws")[0].body).toBe("Fix this");
  store.prune(new Set(["alive"]));
  expect(disk.getItem("prometeu:review:ws")).toBeNull();
  expect(store.list("ws")).toEqual([]);
  expect(store.list("alive")).toHaveLength(1);
});
it("protects corrupt or unknown records from the next write", () => {
  for (const raw of ['{"v":9,"comments":[]}', 'broken', '{"v":1,"comments":[{}]}']) {
    const disk = storage(); disk.setItem("prometeu:review:ws", raw);
    const store = createReviewStore(() => disk);
    expect(store.list("ws")).toEqual([]);
    expect(() => store.add("ws", anchor(), "New")).toThrow("review.storage.read");
    expect(disk.getItem("prometeu:review:ws")).toBe(raw);
  }
});
it("keeps memory when writing fails and can persist it on the next edit", () => {
  const disk = storage(), save = disk.setItem;
  disk.setItem = () => { throw new Error("quota"); };
  const store = createReviewStore(() => disk);
  expect(() => store.add("ws", anchor(), "Keep my writing")).toThrow("review.storage.write");
  expect(store.list("ws")[0].body).toBe("Keep my writing");
  disk.setItem = save;
  store.edit("ws", store.list("ws")[0].id, "Recovered");
  expect(createReviewStore(() => disk).list("ws")[0].body).toBe("Recovered");
});
it("submits only matching revisions and keeps the sent snapshot for subsequent edits", () => {
  const disk = storage(), store = createReviewStore(() => disk);
  store.add("ws", anchor(), "First");
  const first = store.list("ws")[0];
  const draft = reviewBatch("ws", "batch", [first], a => a.path);
  store.edit("ws", first.id, "Second");
  store.submit(draft, "tab");
  expect(store.list("ws")[0]).toMatchObject({ state: "draft", body: "Second", sent: { batch: "batch", tab: "tab", revision: 1 } });
  const current = reviewBatch("ws", "batch2", store.list("ws"), a => a.path);
  store.submit(current, "tab");
  expect(store.list("ws")[0].state).toBe("sent");
  store.state("ws", first.id, "resolved");
  store.state("ws", first.id, "draft");
  expect(store.list("ws")[0].state).toBe("draft");
});
it("bounds note count and body bytes without changing the previous review", () => {
  const store = createReviewStore(storage);
  expect(() => store.add("ws", anchor(), "é".repeat(4097))).toThrow("review.limit");
  expect(store.list("ws")).toHaveLength(0);
  for (let i = 0; i < 200; i++) store.add("ws", anchor(), "Note " + i);
  expect(() => store.add("ws", anchor(), "Extra")).toThrow("review.limit");
  expect(store.list("ws")).toHaveLength(200);
});
it("does not expose mutable store state to a draft or a view", () => {
  const store = createReviewStore(storage); store.add("ws", anchor(), "Original");
  const notes = store.list("ws"); notes[0].body = "External change";
  expect(store.list("ws")[0].body).toBe("Original");
});
