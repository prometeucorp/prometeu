import { splitBrowserContexts, type BrowserContext } from "./browser-context";
import { decodeReviewContext, type ReviewContext } from "./review-context";
export type MessageContext = string | { kind: "browser"; value: BrowserContext } | { kind: "review"; value: ReviewContext };
/** One scanner preserves order and treats unknown/nested envelopes as literal text. */
export function splitMessageContexts(text: string): MessageContext[] {
  const result: MessageContext[] = [];
  const markers = /^<(\/?)prometeu-(browser-element|review)(?=[\s>]).*$/gm;
  let preserved = 0, start = 0;
  const stack: string[] = [];
  let invalid = false;
  for (const match of text.matchAll(markers)) {
    if (!match[1]) {
      if (!stack.length) { start = match.index; invalid = false; } else invalid = true;
      stack.push(match[2]); continue;
    }
    if (!stack.length) continue;
    if (stack.pop() !== match[2]) invalid = true;
    if (stack.length) continue;
    const end = match.index + match[0].length, block = text.slice(start, end);
    let part: MessageContext | null = null;
    if (!invalid && match[2] === "review") {
      const value = decodeReviewContext(block); if (value) part = { kind: "review", value };
    } else if (!invalid) {
      const browser = splitBrowserContexts(block);
      if (browser.length === 1 && typeof browser[0] !== "string") part = { kind: "browser", value: browser[0] };
    }
    if (!part) continue;
    if (start > preserved) result.push(text.slice(preserved, start));
    result.push(part); preserved = end;
  }
  if (preserved < text.length || !result.length) result.push(text.slice(preserved));
  return result;
}
