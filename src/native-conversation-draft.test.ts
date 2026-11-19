import { expect, it } from "vitest";
import type { BrowserContext } from "./browser-context";
import { afterNativePaste, nativeFilePaste, NativeReviewAcknowledgments } from "./native-conversation-draft";
import { anchorSelection, type ReviewDraft } from "./review-comments";
import { encodeReviewContext, reviewContext } from "./review-context";

const review = (batch = "batch"): ReviewDraft => ({
  workspace: "workspace", batch,
  items: [{ id: "note", revision: 1, anchor: anchorSelection("repo", "file.ts", "changes", "@@ -0,0 +1 @@\n+value", 1, 1), body: "Check this value", file: "file.ts" }],
});

it("pastes individual image paths for native attachment recognition while keeping document mentions", () => {
  expect(nativeFilePaste("/work/screenshot.png", "/work")).toBe("/work/screenshot.png");
  expect(nativeFilePaste("/work/My screenshot.PNG", "/work")).toBe("'/work/My screenshot.PNG'");
  expect(nativeFilePaste("/work/team's image.png", "/work")).toBe("'/work/team'\\''s image.png'");
  expect(nativeFilePaste("/work/$draft`one`.webp", "/work")).toBe("'/work/$draft`one`.webp'");
  expect(nativeFilePaste("/work/spec.md", "/work")).toBe("\n@spec.md\n");
  expect(nativeFilePaste("/outside/final note.pdf", "/work")).toBe('\n@"/outside/final note.pdf"\n');
});

it("clears only delivered native context while preserving additions made during paste", () => {
  const originalContext = { image: "/original.png" } as BrowserContext;
  const addedContext = { image: "/added.png" } as BrowserContext;
  const originalReview = review(), replacement = review("replacement");
  const captured = { text: "Original action", files: ["original.ts"], contexts: [originalContext], review: originalReview };
  expect(afterNativePaste(captured, captured)).toEqual({ text: "", files: [], contexts: [], review: undefined });
  const changed = { text: "Original action\nNew input", files: ["original.ts", "added.ts"], contexts: [originalContext, addedContext], review: replacement };
  expect(afterNativePaste(changed, captured)).toEqual({ text: changed.text, files: ["added.ts"], contexts: [addedContext], review: replacement });
});

it("requires the exact review content in observed user input before acknowledging pasted notes", () => {
  const pending = new NativeReviewAcknowledgments(), draft = review();
  const encoded = encodeReviewContext(reviewContext(draft));
  pending.remember("tab", draft);
  expect(pending.observed("tab", [])).toEqual([]);
  expect(pending.observed("another-tab", [encoded])).toEqual([]);
  expect(pending.observed("tab", [encoded.replace("Check this value", "Different content")])).toEqual([]);
  expect(pending.observed("tab", [`<prometeu-review v="1">\n${encoded}\n</prometeu-review>`])).toEqual([]);
  expect(pending.observed("tab", [`Review this:\n${encoded}\nPlease continue.`])).toEqual([draft]);
  expect(pending.observed("tab", [encoded])).toEqual([]);
});

it("recognizes a retained pasted review from a later snapshot once and prunes closed conversations", () => {
  const pending = new NativeReviewAcknowledgments(), draft = review();
  pending.remember("tab", draft);
  pending.remember("closed", review("closed-batch"));
  pending.forget(new Set(["tab"]));
  const snapshotUsers = ["Earlier prompt", encodeReviewContext(reviewContext(draft))];
  expect(pending.observed("tab", snapshotUsers)).toEqual([draft]);
  expect(pending.observed("tab", snapshotUsers)).toEqual([]);
  expect(pending.observed("closed", [encodeReviewContext(reviewContext(review("closed-batch")))])).toEqual([]);
});

it("retains separately pasted versions of the same review batch until each exact version is observed", () => {
  const pending = new NativeReviewAcknowledgments(), first = review();
  const second = { ...first, items: first.items.map(item => ({ ...item, revision: 2, body: "Use the updated value" })) };
  pending.remember("tab", first);
  pending.remember("tab", second);
  const firstEnvelope = encodeReviewContext(reviewContext(first));
  const secondEnvelope = encodeReviewContext(reviewContext(second));
  expect(pending.observed("tab", [firstEnvelope])).toEqual([first]);
  expect(pending.observed("tab", [firstEnvelope])).toEqual([]);
  expect(pending.observed("tab", [firstEnvelope, secondEnvelope])).toEqual([second]);
  expect(pending.observed("tab", [firstEnvelope, secondEnvelope])).toEqual([]);
});
