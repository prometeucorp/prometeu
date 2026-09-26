import type { PathEntry } from "./paths";
import type { GitFile } from "./types";

/// Git marks for the side file tree: new, modified, deleted or conflicted.
export type Mark = "A" | "M" | "D" | "U";

/// A tree row that exists only because Git reports it deleted.
export type GoneEntry = PathEntry & { gone: true };

const RANK: Record<Mark, number> = { A: 1, D: 2, M: 2, U: 3 };

export type GitMarks = {
  /// The row's mark. A folder takes the strongest change below it, with deletions shown as
  /// modifications; the deleted rows themselves carry the `D`.
  mark(path: string, dir: boolean): Mark | null;
  /// Deleted children of a tree folder (`""` is the root), so the tree can still show them.
  /// A folder holding deleted files comes back as a folder; callers drop names that still exist.
  gone(rel: string): GoneEntry[];
  /// Changes whenever the set of new or deleted paths changes, such as a file an agent created, so
  /// the tree knows to list its folders again instead of repainting existing rows.
  listKey: string;
};

/// Resolve `tree_git_status` entries into lookups by tree path.
export function gitMarks(files: GitFile[]): GitMarks {
  const own = new Map<string, Mark>();
  const folders = new Map<string, Mark>();
  const deleted: string[] = [];
  const added: string[] = [];
  for (const file of files) {
    const mark = file.status as Mark;
    if (!(mark in RANK)) continue;
    // Git still collapses a nested repository to `dir/`; mark that folder alone.
    const path = file.path.replace(/\/$/, "");
    own.set(path, mark);
    if (mark === "D") deleted.push(path);
    if (mark === "A") added.push(path);
    const up = mark === "D" ? "M" : mark;
    for (let cut = path.lastIndexOf("/"); cut > 0; cut = path.lastIndexOf("/", cut - 1)) {
      const folder = path.slice(0, cut);
      const was = folders.get(folder);
      if (!was || RANK[up] > RANK[was]) folders.set(folder, up);
    }
  }
  deleted.sort();
  added.sort();
  return {
    mark: (path, dir) => own.get(path) ?? (dir ? folders.get(path) : undefined) ?? null,
    gone(rel) {
      const prefix = rel ? `${rel}/` : "";
      const children = new Map<string, GoneEntry>();
      for (const path of deleted) {
        if (!path.startsWith(prefix)) continue;
        const rest = path.slice(prefix.length);
        const cut = rest.indexOf("/");
        const name = cut < 0 ? rest : rest.slice(0, cut);
        if (!children.get(name)?.dir) children.set(name, { name, path: prefix + name, dir: cut >= 0, gone: true });
      }
      return [...children.values()];
    },
    listKey: [deleted.join("\0"), added.join("\0")].join("\n"),
  };
}

/// Order overlapping loads: each call starts a load, and only the latest one started may apply its
/// result. A draw and the timer's repaint can scan at once; an older scan finishing last must not
/// restore stale marks or deleted rows.
export function latestOnly() {
  let started = 0;
  return async <T>(load: Promise<T>, apply: (value: T) => void) => {
    const mine = ++started;
    const value = await load;
    if (mine === started) apply(value);
  };
}
