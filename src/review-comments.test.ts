import { describe, expect, it } from "vitest";
import { rows } from "./components/git/patch";
import { anchorEndRow, anchorSelection, place, mergeReviewDraft, reviewBatch, type ReviewNote } from "./review-comments";

const patch = "@@ -10,3 +10,4 @@\n a\n-b\n+c\n+d\n e";
const anchor = () => anchorSelection("api", "src/cart.ts", "changes", patch, 3, 4);
const note = (revision = 1): ReviewNote => ({ id: "note", revision, anchor: anchor(), body: "Use configured tax.", state: "draft", sent: null, created: 1, updated: 1 });

describe("review anchors", () => {
  it("records both sides for mixed ranges and only the selected side in split layout", () => {
    expect(anchorSelection("api", "a.ts", "changes", patch, 2, 4)).toMatchObject({ old: [11, 11], new: [11, 12], excerpt: ["-b", "+c", "+d"] });
    expect(anchorSelection("api", "a.ts", "staged", patch, 2, 4, "before")).toMatchObject({ old: [11, 11], new: null, excerpt: ["-b"] });
    expect(anchor()).toMatchObject({ old: null, new: [11, 12] });
  });
  it("rejects crossing hunks, headers, missing and truncated rows", () => {
    for (const [start, end] of [[0, 1], [3, 7], [3000, 3000]]) {
      expect(() => anchorSelection("api", "a", "changes", patch + "\n@@ -30 +30 @@\n+x", start, end)).toThrow();
    }
    expect(() => anchorSelection("api", "a", "changes", "", 1, 1)).toThrow();
    expect(() => anchorSelection("api", "a", "changes", "")).toThrow();
  });
  it("follows exact code across shifts without changing the original anchor", () => {
    const original = anchor();
    expect(place(original, patch.replace("10,3 +10,4", "20,3 +20,4"))).toMatchObject({ kind: "attached", anchor: { new: [21, 22] } });
    expect(original.new).toEqual([11, 12]);
    expect(place(original, patch.replace("+c", "+changed"))).toEqual({ kind: "changed" });
    expect(place(original, null)).toEqual({ kind: "detached" });
  });
  it("keeps long quote matching independent from display elision and long lines", () => {
    const large = "@@ -0,0 +1,60 @@\n" + Array.from({ length: 60 }, (_, n) => "+" + n + "x".repeat(500)).join("\n");
    const a = anchorSelection("api", "a", "changes", large, 1, 60);
    expect(a.excerpt.length).toBeLessThanOrEqual(40);
    expect(a.excerpt.every(line => line.length <= 400)).toBe(true);
    expect(place(a, large.replace("+1,60", "+5,60"))).toMatchObject({ kind: "attached", anchor: { new: [5, 64] } });
    expect(place(a, large.replace("+30x", "+30y"))).toEqual({ kind: "changed" });
  });
  it("uses the nearest repeated excerpt and refuses equally near matches", () => {
    const a = anchorSelection("api", "a", "changes", "@@ -0,0 +10 @@\n+x", 1, 1);
    expect(place(a, "@@ -0,0 +8 @@\n+x\n@@ -0,0 +30 @@\n+x")).toMatchObject({ kind: "attached", anchor: { new: [8, 8] } });
    expect(place(a, "@@ -0,0 +8 @@\n+x\n@@ -0,0 +12 @@\n+x")).toEqual({ kind: "ambiguous" });
  });
  it("keeps file notes attached without interpreting missing hunks as a match", () => {
    const a = anchorSelection("api", "a", "compare", patch);
    expect(a.hunk).toBeNull();
    expect(place(a, patch.replace("+c", "+changed"))).toMatchObject({ kind: "attached" });
    expect(place(a, null)).toEqual({ kind: "detached" });
  });
});

describe("review draft snapshots", () => {
  it("merges by note id, replaces revisions and produces working-directory paths", () => {
    const first = reviewBatch("ws", "batch", [note()], a => "api/" + a.path);
    const newer = reviewBatch("ws", "other", [{ ...note(2), body: "New instructions" }], a => "api/" + a.path);
    const draft = mergeReviewDraft(first, newer);
    expect(draft.batch).toBe("batch");
    expect(draft.items).toHaveLength(1);
    expect(draft.items[0]).toMatchObject({ revision: 2, file: "api/src/cart.ts", body: "New instructions" });
    expect(first.items[0].body).toBe("Use configured tax.");
  });
});

it("keeps source scope and matches selected-side text when a staged line becomes context", () => {
  const a = anchorSelection("api", "a", "staged", "@@ -0,0 +10 @@\n+x", 1, 1, "after");
  const placement = place(a, "@@ -9,2 +9,3 @@\n x\n+y\n z");
  expect(placement).toMatchObject({ kind: "attached", anchor: { scope: "staged", new: [9, 9] } });
  expect(a.new).toEqual([10, 10]);
});
it("updates file evidence on a new patch while retaining a file-level anchor", () => {
  const a = anchorSelection("api", "a", "changes", patch);
  const placement = place(a, patch.replace("+c", "+changed"));
  expect(placement.kind).toBe("attached");
  if (placement.kind === "attached") expect(placement.anchor.stamp).not.toBe(a.stamp);
});

it("places inline content within its anchor hunk when endpoints repeat", () => {
  const patch = "@@ -10,1 +10,1 @@ first\n keep\n@@ -10,1 +10,1 @@ second\n other";
  const anchor = anchorSelection("api", "a.ts", "changes", patch, 1, 1);
  expect(anchorEndRow(anchor, rows(patch))).toBe(1);
  expect(anchorEndRow({ ...anchor, hunk: "missing" }, rows(patch))).toBe(-1);
  expect(anchorEndRow(anchorSelection("api", "a.ts", "changes", patch), rows(patch))).toBe(-1);
});
