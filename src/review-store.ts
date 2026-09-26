import { bodyValid, type ReviewAnchor, type ReviewDraft, type ReviewNote } from "./review-comments";
import { bounded, exactKeys, record, safeReviewPath, validSpan } from "./review-context";

type Disk = Pick<Storage, "length" | "key" | "getItem" | "setItem" | "removeItem">;
const PREFIX = "prometeu:review:";
function validAnchor(a: unknown): a is ReviewAnchor {
  return record(a) && exactKeys(a, ["repo", "path", "scope", "reference", "side", "stamp", "hunk", "old", "new", "excerpt", "match"])
    && bounded(a.repo, 4096) && !!a.repo && safeReviewPath(a.path) && ["changes", "staged", "compare"].includes(a.scope as string)
    && bounded(a.stamp, 64) && bounded(a.reference, 4096) && ["both", "before", "after"].includes(a.side as string) && validSpan(a.old) && validSpan(a.new)
    && (a.hunk === null || bounded(a.hunk, 4096)) && Array.isArray(a.excerpt) && a.excerpt.length <= 40
    && a.excerpt.every(v => typeof v === "string" && v.length <= 400)
    && (a.match === null ? a.hunk === null : record(a.match) && exactKeys(a.match, ["count", "hash"]) && Number.isInteger(a.match.count) && Number(a.match.count) > 0 && Number(a.match.count) <= 2500 && bounded(a.match.hash, 64));
}
function validNote(n: unknown): n is ReviewNote {
  return record(n) && exactKeys(n, ["id", "revision", "anchor", "body", "state", "sent", "created", "updated"])
    && bounded(n.id, 128) && !!n.id && Number.isSafeInteger(n.revision) && Number(n.revision) > 0 && validAnchor(n.anchor) && bodyValid(n.body)
    && ["draft", "sent", "resolved"].includes(n.state as string) && typeof n.created === "number" && Number.isFinite(n.created) && typeof n.updated === "number" && Number.isFinite(n.updated)
    && (n.sent === null || record(n.sent) && exactKeys(n.sent, ["batch", "tab", "at", "revision", "anchor"]) && bounded(n.sent.batch, 128) && bounded(n.sent.tab, 128) && typeof n.sent.at === "number" && Number.isFinite(n.sent.at) && Number.isSafeInteger(n.sent.revision) && Number(n.sent.revision) > 0 && validAnchor(n.sent.anchor));
}
type Exclusive = <T>(run: () => T) => Promise<T>;
const exclusive: Exclusive = async run => {
  if (!navigator.locks) throw new Error("review.storage.read");
  return navigator.locks.request("prometeu:review", run);
};
export function createReviewStore(disk: () => Disk, lock: Exclusive = exclusive) {
  const cache = new Map<string, { comments: ReviewNote[]; blocked: boolean; raw: string | null; dirty: boolean }>();
  const listeners = new Set<(workspace: string) => void>();
  function load(id: string) {
    const previous = cache.get(id);
    if (previous?.dirty) return previous;
    const entry = { comments: [] as ReviewNote[], blocked: false, raw: null as string | null, dirty: false };
    try {
      const raw = disk().getItem(PREFIX + id);
      if (previous && raw === previous.raw && !previous.blocked) return previous;
      entry.raw = raw;
      if (raw !== null) {
        if (raw.length > 12 * 1024 * 1024) throw new Error();
        const value: unknown = JSON.parse(raw);
        if (!record(value) || !exactKeys(value, ["v", "comments"]) || value.v !== 1 || !Array.isArray(value.comments) || value.comments.length > 200 || !value.comments.every(validNote) || new Set(value.comments.map(n => n.id)).size !== value.comments.length) throw new Error();
        entry.comments = value.comments;
      }
    } catch { entry.blocked = true; }
    cache.set(id, entry);
    return entry;
  }
  function change(id: string, mutate: (comments: ReviewNote[]) => ReviewNote[]) {
    return lock(() => {
      const entry = load(id);
      if (entry.blocked) throw new Error("review.storage.read");
      // Failed writes stay recoverable, but cannot overwrite another window's work.
      if (entry.dirty && disk().getItem(PREFIX + id) !== entry.raw) throw new Error("review.storage.conflict");
      const next = mutate(structuredClone(entry.comments));
      if (next.length > 200 || !next.every(validNote)) throw new Error("review.limit");
      entry.comments = next; entry.dirty = true;
      let failed = false;
      const raw = JSON.stringify({ v: 1, comments: next });
      try { disk().setItem(PREFIX + id, raw); entry.raw = raw; entry.dirty = false; } catch { failed = true; }
      listeners.forEach(listener => listener(id));
      if (failed) throw new Error("review.storage.write");
    });
  }
  function update(id: string, note: string, revision: number | undefined, apply: (notes: ReviewNote[]) => ReviewNote[]) {
    const expected = revision ?? load(id).comments.find(n => n.id === note)?.revision;
    return change(id, notes => {
      if (expected === undefined || notes.find(n => n.id === note)?.revision !== expected) throw new Error("review.storage.conflict");
      return apply(notes);
    });
  }
  return {
    list: (id: string) => structuredClone(load(id).comments),
    blocked: (id: string) => load(id).blocked,
    subscribe(listener: (workspace: string) => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    external(id: string) { listeners.forEach(listener => listener(id)); },
    add(id: string, anchor: ReviewAnchor, body: string) {
      return change(id, notes => [...notes, { id: crypto.randomUUID(), revision: 1, anchor, body, state: "draft", sent: null, created: Date.now(), updated: Date.now() }]);
    },
    edit(id: string, note: string, body: string, revision?: number) {
      return update(id, note, revision, notes => notes.map(n => n.id === note ? { ...n, body, revision: n.revision + 1, state: "draft", updated: Date.now() } : n));
    },
    state(id: string, note: string, state: ReviewNote["state"], revision?: number) {
      return update(id, note, revision, notes => notes.map(n => n.id === note ? { ...n, state, revision: n.revision + 1, updated: Date.now() } : n));
    },
    remove(id: string, note: string, revision?: number) { return update(id, note, revision, notes => notes.filter(n => n.id !== note)); },
    submit(draft: ReviewDraft, tab: string) {
      return change(draft.workspace, notes => notes.map(n => {
        const item = draft.items.find(item => item.id === n.id);
        if (!item) return n;
        return { ...n, state: n.revision === item.revision ? "sent" : n.state, sent: { batch: draft.batch, tab, at: Date.now(), revision: item.revision, anchor: item.anchor } };
      }));
    },
    prune(alive: Set<string>) {
      return lock(() => {
        const storage = disk(), gone: string[] = [];
        for (let i = 0; i < storage.length; i++) { const key = storage.key(i); if (key?.startsWith(PREFIX) && !alive.has(key.slice(PREFIX.length))) gone.push(key); }
        for (const key of gone) storage.removeItem(key);
        for (const id of cache.keys()) if (!alive.has(id)) cache.delete(id);
      });
    },
  };
}
export const reviews = createReviewStore(() => localStorage);
if (typeof window !== "undefined") window.addEventListener("storage", event => {
  if (event.storageArea === localStorage && event.key?.startsWith(PREFIX)) reviews.external(event.key.slice(PREFIX.length));
});
