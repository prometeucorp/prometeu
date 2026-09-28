import { describe, expect, it } from "vitest";
import defaultsText from "./action-defaults.json?raw";
import { builders, emptyCatalog, initializeDefaults, commandNames, expand, findCommand, pick, taskBadges, validRules, type Action, type Catalog, type Profile, type TaskRun } from "./actions";

const prompt: Action = { name: "review", kind: "prompt", prompt: "Review the diff.", description: "", profile: null };
const task: Action = { ...prompt, name: "deliver", kind: "agent", profile: "owner" };
describe("reusable commands", () => {
  it("preserves provider commands and resolves collisions with explicit namespaces", () => {
    const provider = [{ name: "review" }];
    expect(commandNames([prompt, task], provider).map(c => c.name)).toEqual(["prometeu:review", "deliver"]);
    expect(findCommand("/review", [prompt], provider)).toBeNull();
    expect(findCommand("/prometeu:review file.ts", [prompt], provider)).toEqual({ action: prompt, rest: "file.ts" });
    expect(findCommand("/deliver context\nextra", [task], provider)).toEqual({ action: task, rest: "context\nextra" });
  });
  it("expands prompts without losing text or intercepting paths and unknown commands", () => {
    expect(expand(prompt.prompt, "  file.ts  ")).toBe("Review the diff.\n\nfile.ts");
    expect(findCommand("/Users/me/project", [prompt], [])).toBeNull();
    expect(findCommand("/compact", [prompt], [])).toBeNull();
    expect(findCommand("text /review", [prompt], [])).toBeNull();
    expect(findCommand("/prometeu:review", [prompt], [])?.action).toEqual(prompt);
  });
});


describe("built-in Code review", () => {
  it("initializes once, preserves customizations and respects removal", () => {
    const seeded = initializeDefaults(emptyCatalog());
    expect(seeded.commands.map(c => c.name)).toEqual(["review"]);
    expect(seeded.profiles[0].watch).toBeNull();
    seeded.profiles[0].choice.model = "sonnet";
    expect(initializeDefaults(seeded).profiles[0].choice.model).toBe("sonnet");
    seeded.commands = []; seeded.profiles = [];
    expect(initializeDefaults(JSON.parse(JSON.stringify(seeded))).commands).toEqual([]);
    const custom = { ...emptyCatalog(), commands: [{ ...prompt, name: "review" }] };
    expect(initializeDefaults(custom).commands).toEqual(custom.commands);
    expect(initializeDefaults(custom).profiles).toEqual([]);
  });
});

describe("provider rule and access", () => {
  const seed = JSON.parse(defaultsText) as { revision: number; profile: Profile; command: Action; previous: Profile[] };
  const old = (): Catalog => ({ ...emptyCatalog(), defaults_initialized: true, profiles: [structuredClone(seed.previous[0])], commands: [seed.command] });
  it("upgrades an untouched earlier seed exactly once and keeps customized profiles", () => {
    const upgraded = initializeDefaults(old());
    expect(upgraded.defaults_revision).toBe(seed.revision);
    expect(upgraded.profiles[0]).toMatchObject({ provider_rule: "different_from_builder", access: "read_only" });
    expect(upgraded.profiles[0].candidates.map(c => c.agent)).toEqual(["codex", "claude"]);
    upgraded.profiles[0] = structuredClone(seed.previous[0]);
    expect(initializeDefaults(upgraded).profiles[0].choice.agent).toBe("claude");
    const custom = old(); custom.profiles[0].choice.model = "opus";
    const kept = initializeDefaults(custom);
    expect(kept.profiles[0].choice.model).toBe("opus");
    expect(kept.profiles[0]).toMatchObject({ provider_rule: "fixed", candidates: [], access: "default" });
  });
  it("mirrors the backend validation", () => {
    const capable = (agent: string) => agent !== "antigravity";
    const review = initializeDefaults(emptyCatalog()).profiles[0];
    expect(validRules(review, capable)).toBe(true);
    expect(validRules({ ...review, choice: { agent: "claude", model: "", effort: "" } }, capable)).toBe(false);
    expect(validRules({ ...review, candidates: [...review.candidates, { agent: "codex", model: "", effort: "" }] }, capable)).toBe(false);
    expect(validRules({ ...review, candidates: [...review.candidates, { agent: "antigravity", model: "", effort: "" }] }, capable)).toBe(false);
    expect(validRules({ ...review, mcp: ["server"] }, capable)).toBe(false);
    expect(validRules({ ...review, mcp: [] }, capable)).toBe(true);
    expect(validRules({ ...review, provider_rule: "fixed" }, capable)).toBe(false);
    expect(validRules({ ...review, provider_rule: "fixed", candidates: [] }, capable)).toBe(true);
  });
  it("keeps plugins and skills out of read-only profiles", () => {
    const capable = () => true;
    const review = initializeDefaults(emptyCatalog()).profiles[0];
    expect(validRules({ ...review, plugins: ["plugin"] }, capable)).toBe(false);
    expect(validRules({ ...review, plugins: [] }, capable)).toBe(true);
    expect(validRules({ ...review, skills: ["review"] }, capable)).toBe(false);
    expect(validRules({ ...review, access: "default", plugins: ["plugin"], skills: ["review"] }, capable)).toBe(true);
  });
});

describe("reviewer selection mirror", () => {
  const review = () => initializeDefaults(emptyCatalog()).profiles[0];
  it("matches the backend cases", () => {
    const p = review();
    expect(pick(p, ["claude"], () => true)).toEqual({ choice: { agent: "codex", model: "", effort: "" }, same_family: false });
    expect(pick(p, ["codex"], () => true).choice.agent).toBe("claude");
    expect(pick(p, ["claude", "codex"], () => true)).toMatchObject({ choice: { agent: "codex" }, same_family: true });
    expect(pick(p, ["claude"], agent => agent !== "codex")).toMatchObject({ choice: { agent: "claude" }, same_family: true });
    expect(pick(p, ["claude"], () => false)).toMatchObject({ choice: { agent: "codex" }, same_family: false });
    const fixed = { ...p, provider_rule: "fixed" as const, candidates: [] };
    expect(pick(fixed, ["codex"], () => { throw new Error("fixed profiles do not probe providers"); }).choice).toEqual(fixed.choice);
  });
  it("counts ordinary tabs or the workspace default as builders", () => {
    const tab = (id: string, agent?: "claude" | "codex", task = false) => ({ id, title: "", status: "pronta" as const, note: null, tokens: null,
      choice: agent ? { agent, model: "", effort: "" } : null, task: task ? { command: "review" } as TaskRun : null });
    expect(builders({ agent: "codex", tabs: [] })).toEqual(["codex"]);
    expect(builders({ agent: "codex", tabs: [tab("review", "claude", true)] })).toEqual(["codex"]);
    expect(builders({ agent: "codex", tabs: [tab("a"), tab("b", "claude"), tab("c", "claude")] })).toEqual(["codex", "claude"]);
  });
});

describe("task badges", () => {
  const run = (access: "default" | "read_only", same_family?: boolean) =>
    ({ profile: { access }, same_family } as unknown as TaskRun);
  it("explains enforced read-only access and a same-family fallback", () => {
    expect(taskBadges(null)).toEqual([]);
    expect(taskBadges(run("default"))).toEqual([]);
    expect(taskBadges(run("read_only", true)).map(b => b.label)).toEqual(["actions.readOnly", "actions.sameFamily"]);
    expect(taskBadges(run("default", true)).map(b => b.title)).toEqual(["actions.sameFamilyTitle"]);
  });
});
