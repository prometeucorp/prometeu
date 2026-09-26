import { expect, it } from "vitest";
import { gitMarks, latestOnly } from "./tree-git";

const marks = gitMarks([
  { path: "Gemfile", status: "M" },
  { path: "app/models/user.rb", status: "A" },
  { path: "app/models/old.rb", status: "D" },
  { path: "lib/tasks/merge.rake", status: "U" },
  { path: "lib/new.rb", status: "A" },
  { path: "docs/guide/intro.md", status: "A" },
  { path: "vendor/engine/", status: "A" },
  { path: "legacy/api/v1.rb", status: "D" },
  { path: "legacy/api/v2.rb", status: "D" },
  { path: "legacy/README.md", status: "D" },
]);

it("marks changed files by their own status", () => {
  expect(marks.mark("Gemfile", false)).toBe("M");
  expect(marks.mark("app/models/user.rb", false)).toBe("A");
  expect(marks.mark("app/models/old.rb", false)).toBe("D");
  expect(marks.mark("README.md", false)).toBeNull();
});

it("gives folders the strongest change below them, with deletions as modifications", () => {
  expect(marks.mark("app", true)).toBe("M");
  expect(marks.mark("app/models", true)).toBe("M");
  expect(marks.mark("lib", true)).toBe("U");
  expect(marks.mark("config", true)).toBeNull();
});

it("marks a new folder from its listed files only", () => {
  expect(marks.mark("docs", true)).toBe("A");
  expect(marks.mark("docs/guide", true)).toBe("A");
  expect(marks.mark("docs/guide/.env", false)).toBeNull();
  expect(marks.mark("docsite", true)).toBeNull();
  expect(marks.mark("vendor/engine", true)).toBe("A");
  expect(marks.mark("vendor/engine/lib.rb", false)).toBeNull();
});

it("lists deleted children per folder, folding deleted subtrees into one folder row", () => {
  const names = (rel: string) => marks.gone(rel).map(entry => `${entry.path}${entry.dir ? "/" : ""}`).sort();
  // Folders that still exist come back too; the tree drops names list_dir already returned.
  expect(names("")).toEqual(["app/", "legacy/"]);
  expect(names("app/models")).toEqual(["app/models/old.rb"]);
  expect(names("legacy")).toEqual(["legacy/README.md", "legacy/api/"]);
  expect(names("legacy/api")).toEqual(["legacy/api/v1.rb", "legacy/api/v2.rb"]);
  expect(names("lib")).toEqual([]);
});

it("changes its list key only when the new or deleted set changes", () => {
  const same = gitMarks([{ path: "b", status: "D" }, { path: "a", status: "D" }, { path: "c", status: "M" }]);
  expect(same.listKey).toBe(gitMarks([{ path: "a", status: "D" }, { path: "b", status: "D" }]).listKey);
  expect(same.listKey).not.toBe(gitMarks([{ path: "a", status: "D" }]).listKey);
  // A file an agent created must appear in the tree, while edits only repaint rows.
  const created = gitMarks([{ path: "a", status: "D" }, { path: "b", status: "D" }, { path: "src/new.ts", status: "A" }]);
  expect(created.listKey).not.toBe(same.listKey);
  expect(gitMarks([{ path: "src/new.ts", status: "A" }, { path: "c", status: "M" }]).listKey)
    .toBe(gitMarks([{ path: "src/new.ts", status: "A" }]).listKey);
  expect(gitMarks([{ path: "x", status: "A" }]).listKey).not.toBe(gitMarks([{ path: "x", status: "D" }]).listKey);
});

it("applies only the latest scan when overlapping scans finish out of order", async () => {
  const scan = latestOnly();
  const applied: string[] = [];
  let finishOld!: (value: string) => void, finishNew!: (value: string) => void;
  const older = scan(new Promise<string>((done) => (finishOld = done)), (v) => applied.push(v));
  const newer = scan(new Promise<string>((done) => (finishNew = done)), (v) => applied.push(v));
  finishNew("new");
  await newer;
  finishOld("old");
  await older;
  expect(applied).toEqual(["new"]);
  // A later scan on its own still applies.
  await scan(Promise.resolve("next"), (v) => applied.push(v));
  expect(applied).toEqual(["new", "next"]);
});
