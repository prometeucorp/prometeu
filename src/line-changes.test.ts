import { expect, it } from "vitest";
import { lineChanges } from "./line-changes";

const lines = (...rows: string[]) => rows.join("\n") + "\n";

it("reports nothing for an unchanged buffer", () => {
  expect(lineChanges(lines("a", "b"), lines("a", "b"))).toEqual([]);
  expect(lineChanges("", "")).toEqual([]);
});

it("marks added, modified and deleted lines by their place in the buffer", () => {
  expect(lineChanges(lines("a", "b", "c"), lines("a", "new", "b", "c"))).toEqual([{ kind: "A", start: 1, count: 1 }]);
  expect(lineChanges(lines("a", "b", "c"), lines("a", "B", "c"))).toEqual([{ kind: "M", start: 1, count: 1 }]);
  expect(lineChanges(lines("a", "b", "c"), lines("a", "c"))).toEqual([{ kind: "D", start: 1, count: 0 }]);
});

it("places deletions at the top and at the end of the file", () => {
  expect(lineChanges(lines("a", "b"), lines("b"))).toEqual([{ kind: "D", start: 0, count: 0 }]);
  expect(lineChanges(lines("a", "b"), lines("a"))).toEqual([{ kind: "D", start: 1, count: 0 }]);
});

it("keeps separate hunks apart", () => {
  const base = lines("1", "2", "3", "4", "5", "6", "7");
  const text = lines("1", "two", "3", "4", "5", "6", "7", "8", "9");
  expect(lineChanges(base, text)).toEqual([
    { kind: "M", start: 1, count: 1 },
    { kind: "A", start: 7, count: 2 },
  ]);
  expect(lineChanges(lines("a", "x", "b", "y", "c"), lines("a", "b", "c", "z"))).toEqual([
    { kind: "D", start: 1, count: 0 },
    { kind: "D", start: 2, count: 0 },
    { kind: "A", start: 3, count: 1 },
  ]);
});

it("reads every line of a file Git does not know yet as new", () => {
  expect(lineChanges("", lines("a", "b"))).toEqual([{ kind: "A", start: 0, count: 2 }]);
  expect(lineChanges("", "a")).toEqual([{ kind: "A", start: 0, count: 1 }]);
});

it("ignores only the final newline, which is the buffer's editable last line", () => {
  expect(lineChanges("a\n", "a")).toEqual([]);
});

it("falls back to one modified range when the edit is too large to compare exactly", () => {
  const base = Array.from({ length: 5000 }, (_, i) => `old ${i}`);
  const text = Array.from({ length: 5000 }, (_, i) => `new ${i}`);
  expect(lineChanges(lines("keep", ...base, "end"), lines("keep", ...text, "end"))).toEqual([
    { kind: "M", start: 1, count: 5000 },
  ]);
});

it("finds the minimal edits in a mixed change", () => {
  const base = lines("1", "2", "3", "4", "5", "6", "7", "8", "9", "10");
  const text = lines("1", "2", "4", "5", "six", "7", "8", "new", "9", "10");
  expect(lineChanges(base, text)).toEqual([
    { kind: "D", start: 2, count: 0 },
    { kind: "M", start: 4, count: 1 },
    { kind: "A", start: 7, count: 1 },
  ]);
});
