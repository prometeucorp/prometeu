import { describe, expect, it } from "vitest";
import catalog from "./catalog.json";
import { stories } from "./stories";

// Vite supplies source text without introducing Node globals into the browser typecheck.
const normalize = (path: string) => {
  const parts: string[] = [];
  for (const part of path.split("/")) {
    if (part === "..") parts.pop(); else if (part !== "." && part) parts.push(part);
  }
  return parts.join("/");
};
const sources = new Map(Object.entries(import.meta.glob<string>(["../**/*.ts", "../../packages/design-system/src/*.ts"],
  { query: "?raw", import: "default", eager: true })).map(([path, text]) => [normalize(`src/components/${path}`), text]));
const dependencies = new Map<string, string[]>();
function imports(file: string): string[] {
  if (dependencies.has(file)) return dependencies.get(file)!;
  const text = sources.get(file) ?? "";
  const result = [...text.matchAll(/(?:from\s*|import\s*)["'](\.[^"']+)["']/g)].flatMap(([, path]) => {
    const base = normalize(`${file.slice(0, file.lastIndexOf("/"))}/${path.replace(/\.js$/, "")}`);
    const target = [base, base + ".ts", base + "/index.ts"].find(candidate => sources.has(candidate));
    return target ? [target] : [];
  });
  dependencies.set(file, result); return result;
}
function reaches(from: string, target: string, visited = new Set<string>()): boolean {
  if (from === target) return true;
  if (visited.has(from)) return false;
  visited.add(from); return imports(from).some(dependency => reaches(dependency, target, visited));
}

describe("Desktop component catalog", () => {
  it("keeps every entry executable and reachable from a production consumer", () => {
    expect(new Set(catalog.map(entry => entry.id)).size).toBe(catalog.length);
    expect(Object.keys(stories).sort()).toEqual(catalog.map(entry => entry.id).sort());
    for (const entry of catalog) {
      expect(entry.states.length, entry.id).toBeGreaterThan(0);
      expect(new Set(entry.states).size, entry.id).toBe(entry.states.length);
      expect(sources.has(entry.source), entry.source).toBe(true);
      expect(entry.consumers.length, entry.id).toBeGreaterThan(0);
      for (const consumer of entry.consumers) {
        expect(consumer).not.toMatch(/(?:stories|gallery)\.ts$/);
        expect(reaches(consumer, entry.source), `${consumer} must use ${entry.source}`).toBe(true);
      }
    }
  });
});
