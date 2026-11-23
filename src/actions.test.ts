import { beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { emptyCatalog, initializeDefaults, commandNames, expand, findCommand, init, start, type Action } from "./actions";

vi.mock("./analytics", () => ({ track: vi.fn() }));
vi.mock("./ipc", () => ({ invoke: vi.fn() }));
import { track } from "./analytics";
import { invoke } from "./ipc";
// `vi.mocked(invoke)` makes the checker relate the whole IPC registry to the mock type, which costs minutes.
const invokeMock = invoke as unknown as Mock;

beforeEach(() => vi.clearAllMocks());

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
    seeded.profiles[0].choice.model = "sonnet";
    expect(initializeDefaults(seeded).profiles[0].choice.model).toBe("sonnet");
    seeded.commands = []; seeded.profiles = [];
    expect(initializeDefaults(JSON.parse(JSON.stringify(seeded))).commands).toEqual([]);
    const custom = { ...emptyCatalog(), commands: [{ ...prompt, name: "review" }] };
    expect(initializeDefaults(custom).commands).toEqual(custom.commands);
    expect(initializeDefaults(custom).profiles).toEqual([]);
  });
});


describe("action usage", () => {
  it("counts a successful launch once even if opening its view fails", async () => {
    invokeMock.mockResolvedValueOnce({ id: "tab" });
    init(async () => { throw new Error("View unavailable"); });
    await expect(start("workspace", task, "Private context")).rejects.toThrow("View unavailable");
    expect(track).toHaveBeenCalledExactlyOnceWith({ name: "action_used", kind: "agent" });
  });

  it("does not report rejected launches", async () => {
    invokeMock.mockRejectedValueOnce(new Error("Launch unavailable"));
    await expect(start("workspace", task)).rejects.toThrow("Launch unavailable");
    expect(track).not.toHaveBeenCalled();
  });
});
