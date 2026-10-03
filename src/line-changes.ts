/// Line changes of an edited buffer against its committed text, for the editor's change gutter.
/// `start` is a 0-based line of the buffer. Added and modified ranges cover `count` lines; a
/// deletion has no lines of its own and sits at the boundary above line `start`, which is the line
/// count when the removed lines were at the end.
export type LineChange = { kind: "A" | "M" | "D"; start: number; count: number };

/// Past this many edits the middle of the file is reported as one modified range instead of paying
/// for an exact comparison on every keystroke. The trace grows with its square: 500 edits keep it
/// near 250k integers per pass.
const MAX_EDITS = 500;

const split = (text: string) => (text === "" ? [] : text.split("\n"));

export function lineChanges(base: string, text: string): LineChange[] {
  const a = split(base);
  const b = split(text);
  // An empty trailing line is the buffer's last editable line, not content Git compares.
  if (a[a.length - 1] === "") a.pop();
  if (b[b.length - 1] === "") b.pop();
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  const old = a.slice(head, a.length - tail);
  const now = b.slice(head, b.length - tail);
  if (!old.length && !now.length) return [];
  const pairs = matches(old, now);
  const out: LineChange[] = [];
  let i = 0;
  let j = 0;
  for (const [x, y] of [...(pairs ?? []), [old.length, now.length] as const]) {
    const removed = x - i;
    const added = y - j;
    const start = head + j;
    if (removed && added) out.push({ kind: "M", start, count: added });
    else if (added) out.push({ kind: "A", start, count: added });
    else if (removed) out.push({ kind: "D", start, count: 0 });
    i = x + 1;
    j = y + 1;
  }
  return out;
}

/// Matching line pairs `[old, now]` in order, from Myers' O(ND) diff, or null past `MAX_EDITS`.
function matches(a: string[], b: string[]): (readonly [number, number])[] | null {
  const ids = new Map<string, number>();
  const id = (line: string) => {
    let n = ids.get(line);
    if (n === undefined) ids.set(line, (n = ids.size));
    return n;
  };
  const x = a.map(id);
  const y = b.map(id);
  const n = x.length;
  const m = y.length;
  const max = Math.min(n + m, MAX_EDITS);
  const off = max + 1;
  const v = new Int32Array(2 * max + 3);
  // Each step keeps only its reachable diagonals, so the trace costs O(D²) rather than O(D·(N+M)).
  const trace: Int32Array[] = [];
  for (let d = 0; d <= max; d++) {
    trace.push(v.slice(off - d, off + d + 1));
    for (let k = -d; k <= d; k += 2) {
      let i = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let j = i - k;
      while (i < n && j < m && x[i] === y[j]) {
        i++;
        j++;
      }
      v[off + k] = i;
      if (i >= n && j >= m) return walk(trace, d, n, m, x, y);
    }
  }
  return null;
}

function walk(trace: Int32Array[], last: number, n: number, m: number, x: number[], y: number[]) {
  const pairs: (readonly [number, number])[] = [];
  let i = n;
  let j = m;
  for (let d = last; d >= 0; d--) {
    // trace[d] holds diagonals -d..d as they were before step d ran.
    const before = trace[d];
    const at = (k: number) => before[k + d];
    const k = i - j;
    let prev: number;
    if (d === 0) prev = 0;
    else prev = k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
    const start = d === 0 ? 0 : prev === k + 1 ? at(k + 1) : at(k - 1) + 1;
    const startJ = start - k;
    while (i > start && j > startJ && x[i - 1] === y[j - 1]) pairs.push([--i, --j]);
    if (d === 0) break;
    i = at(prev);
    j = i - prev;
  }
  return pairs.reverse();
}
