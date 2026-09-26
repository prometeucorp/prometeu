import type { Change, RepoDiff } from "./types";
import { key, stamp } from "./components/git/patch";
import { diffView } from "./components/git/diff-view";
export { key, keys, sum, rows, splitRows, type Row, type SplitRow } from "./components/git/patch";

// Persisted review state belongs to the Desktop adapter, not the visual component.
const seenOf = new Map<string, Record<string, string>>();
const seenKey = (id: string) => `prometeu:visto:${id}`;

function seenMap(id: string): Record<string, string> {
  let m = seenOf.get(id);
  if (!m) {
    try {
      m = JSON.parse(localStorage.getItem(seenKey(id)) ?? "{}") as Record<string, string>;
    } catch {
      m = {};
    }
    seenOf.set(id, m);
  }
  return m;
}

function saveSeen(id: string) {
  const m = seenMap(id);
  if (Object.keys(m).length) localStorage.setItem(seenKey(id), JSON.stringify(m));
  else localStorage.removeItem(seenKey(id));
}

export const isSeen = (id: string, repo: string, c: Change) => seenMap(id)[key(repo, c.path)] === stamp(c);

export function setSeen(id: string, repo: string, c: Change, on: boolean) {
  const m = seenMap(id);
  if (on) m[key(repo, c.path)] = stamp(c);
  else delete m[key(repo, c.path)];
  saveSeen(id);
}

export function seeAll(id: string, repos: RepoDiff[]) {
  const m = seenMap(id);
  for (const r of repos) for (const c of r.files) m[key(r.name, c.path)] = stamp(c);
  saveSeen(id);
}

export const unseen = (id: string, repos: RepoDiff[]) =>
  repos.reduce((n, r) => n + r.files.filter((c) => !isSeen(id, r.name, c)).length, 0);

/// Remove seen state when its workspace leaves the board.
export function pruneSeen(alive: Set<string>) {
  const gone: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (k?.startsWith("prometeu:visto:") && !alive.has(k.slice("prometeu:visto:".length))) gone.push(k);
  }
  for (const k of gone) {
    localStorage.removeItem(k);
    seenOf.delete(k.slice("prometeu:visto:".length));
  }
}


const reader = diffView({ isSeen, setSeen });
export const render = reader.render;
export const invalidate = reader.invalidate;
export const foldAll = reader.foldAll;
