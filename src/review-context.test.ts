import { anchorSelection, reviewBatch, type ReviewNote } from "./review-comments";
import { expect, it } from "vitest";
import { encodeReviewContext, decodeReviewContext, reviewContext, type ReviewContext } from "./review-context";
import { splitMessageContexts } from "./message-context";
import { encodeBrowserContext } from "./browser-context";

const sample = (): ReviewContext => ({ batch: "batch", comments: [{ n: 1, repo: "api", file: "api/src/cart.ts", in: "index", old: null, new: [4, 5], hunk: "@@ -1,3 +1,4 @@", excerpt: ["+tax"], body: "Use <configured> tax 😀" }] });
it("round trips bounded review content and escapes delimiters", () => {
  const encoded = encodeReviewContext(sample());
  expect(encoded.split("\n")[1]).not.toContain("<");
  expect(decodeReviewContext(encoded)).toEqual(sample());
});
it("rejects unsafe paths, extra keys, invalid ranges, oversized bodies and unknown versions", () => {
  for (const file of ["/etc/passwd", "../a", "a/../b", "a\nb", "C:\\a", "a\\..\\b", "a\0b"]) {
    const value = sample(); value.comments[0].file = file;
    expect(() => encodeReviewContext(value)).toThrow();
  }
  for (const change of [{ extra: true }, { new: [5, 4] }, { new: [0, 3] }, { body: "é".repeat(4097) }]) {
    const value = sample(); Object.assign(value.comments[0], change);
    expect(() => encodeReviewContext(value)).toThrow();
  }
  expect(decodeReviewContext(encodeReviewContext(sample()).replace('v="1"', 'v="2"'))).toBeNull();
  expect(() => encodeReviewContext({ ...sample(), comments: Array.from({ length: 101 }, (_, n) => ({ ...sample().comments[0], n: n + 1 })) })).toThrow();
});
it("preserves order and literal malformed blocks next to browser blocks", () => {
  const review = encodeReviewContext(sample());
  const browser = { selection: { url: "http://localhost", selector: "button", tag: "button", text: "Send", html: "<button>Send</button>", styles: {}, rect: { x: 0, y: 0, width: 1, height: 1 }, viewport: { width: 100, height: 100 } } };
  const bad = review.replace('v="1"', 'v="9"');
  expect(splitMessageContexts(`${bad}\n${review}\n${encodeBrowserContext(browser)}\nDone`)).toEqual([
    bad + "\n", { kind: "review", value: sample() }, "\n", { kind: "browser", value: browser }, "\nDone",
  ]);
  const nested = '<prometeu-review v="9">\n' + review + '\n</prometeu-review>';
  expect(splitMessageContexts(nested)).toEqual([nested]);
});
it("bounds the encoded block after delimiter escaping and preserves rejected payloads literally", () => {
  const value = sample();
  value.comments = Array.from({ length: 40 }, (_, i) => ({ ...value.comments[0], n: i + 1, body: "<".repeat(8192) }));
  expect(() => encodeReviewContext(value)).toThrow("review.limit");
  const literal = '<prometeu-review v="1">\n' + JSON.stringify(value) + '\n</prometeu-review>';
  expect(splitMessageContexts(literal)).toEqual([literal]);
});

it("submits CRLF selections while preserving exact matching evidence", () => {
  const anchor = anchorSelection("api", "a.ts", "changes", "@@ -1 +1 @@\n-old\r\n+new\r\n", 2, 2);
  const note: ReviewNote = { id: "crlf", revision: 1, anchor, body: "Keep CRLF endings.", state: "draft", sent: null, created: 1, updated: 1 };
  const draft = reviewBatch("ws", "crlf", [note], a => a.path);
  const context = reviewContext(draft);
  expect(() => encodeReviewContext(context)).not.toThrow();
  expect(decodeReviewContext(encodeReviewContext(context))?.comments[0].excerpt).toEqual(["+new"]);
});
