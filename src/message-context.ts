import { splitBrowserContexts, type BrowserContext } from "./browser-context";
import { decodeReviewContext, type ReviewContext } from "./review-context";
export type MessageContext = string | { kind: "browser"; value: BrowserContext } | { kind: "review"; value: ReviewContext };
/** One scanner preserves order and treats unknown/nested envelopes as literal text. */
export function splitMessageContexts(text: string): MessageContext[] {
  const result: MessageContext[] = [];
  const markers = /^<(\/?)prometeu-(browser-element|review)(?=[\s>]).*$/gm;
  type Block = { start: number; end?: number; kind: string; invalid: boolean; children: Block[] };
  const roots: Block[] = [], stack: Block[] = [];
  for (const match of text.matchAll(markers)) {
    if (!match[1]) {
      const block: Block = { start: match.index, kind: match[2], invalid: false, children: [] };
      (stack[stack.length - 1]?.children ?? roots).push(block); stack.push(block);
    } else {
      const block = stack.pop();
      if (block) { block.end = match.index + match[0].length; block.invalid = block.kind !== match[2]; }
    }
  }
  let preserved = 0;
  // Closed nesting stays literal. Only unfinished ancestors allow resynchronizing
  // at complete children, so a truncated prefix cannot swallow subsequent tags.
  const pending = [...roots].reverse();
  while (pending.length) {
    const block = pending.pop()!;
    if (block.end === undefined) { pending.push(...[...block.children].reverse()); continue; }
    if (block.invalid || block.children.length) continue;
    const raw = text.slice(block.start, block.end);
    let part: MessageContext | null = null;
    if (block.kind === "review") {
      const value = decodeReviewContext(raw); if (value) part = { kind: "review", value };
    } else {
      const browser = splitBrowserContexts(raw);
      if (browser.length === 1 && typeof browser[0] !== "string") part = { kind: "browser", value: browser[0] };
    }
    if (!part) continue;
    if (block.start > preserved) result.push(text.slice(preserved, block.start));
    result.push(part); preserved = block.end;
  }
  if (preserved < text.length || !result.length) result.push(text.slice(preserved));
  return result;
}
