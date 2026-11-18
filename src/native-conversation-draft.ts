import type { BrowserContext } from "./browser-context";
import type { ReviewDraft } from "./review-comments";
import { encodeReviewContext, reviewContext } from "./review-context";
import { splitMessageContexts } from "./message-context";
import { isImage, mentions } from "./mentions";

export type NativeConversationDraft = {
  text: string;
  files: string[];
  contexts: BrowserContext[];
  review?: ReviewDraft;
};

/** Native image recognition requires one literal path per paste, without a mention prefix. */
export function nativeFilePaste(path: string, root: string | null): string {
  if (!isImage(path)) return `\n${mentions([path], root)}\n`;
  // Single-quote shell-sensitive paths for native CLIs that parse pasted paths with shlex.
  return /^[\w/.-]+$/.test(path) ? path : `'${path.replace(/'/g, "'\\''")}'`;
}

/** A successful paste consumes only the captured context, preserving additions made while it was pending. */
export function afterNativePaste(current: NativeConversationDraft, pasted: NativeConversationDraft): NativeConversationDraft {
  return {
    text: current.text === pasted.text ? "" : current.text,
    files: current.files.filter(file => !pasted.files.includes(file)),
    contexts: current.contexts.filter(context => !pasted.contexts.includes(context)),
    review: current.review === pasted.review ? undefined : current.review,
  };
}

/** Pasting does not submit review notes. Only an exact review envelope in observed user input does. */
export class NativeReviewAcknowledgments {
  private pending = new Map<string, Map<string, ReviewDraft>>();

  remember(tab: string, draft: ReviewDraft) {
    const versions = this.pending.get(tab) ?? new Map<string, ReviewDraft>();
    versions.set(encodeReviewContext(reviewContext(draft)), draft);
    this.pending.set(tab, versions);
  }

  observed(tab: string, userTexts: string[]): ReviewDraft[] {
    const versions = this.pending.get(tab);
    if (!versions) return [];
    const envelopes = new Set(userTexts.flatMap(text => splitMessageContexts(text).flatMap(part =>
      typeof part !== "string" && part.kind === "review" ? [encodeReviewContext(part.value)] : [])));
    const accepted: ReviewDraft[] = [];
    for (const [encoded, draft] of versions) {
      if (!envelopes.has(encoded)) continue;
      versions.delete(encoded);
      accepted.push(draft);
    }
    if (!versions.size) this.pending.delete(tab);
    return accepted;
  }

  forget(alive: Set<string>) {
    for (const tab of this.pending.keys()) if (!alive.has(tab)) this.pending.delete(tab);
  }
}
