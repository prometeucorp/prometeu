import { expect, it } from "vitest";
import { decisionSurfaceOf, isNativeCommand } from "./conversation-capabilities";
import type { Tab, Workspace } from "./types";

it.each(["/", "/compact", " /context detail", "\t/plugin:command\nargument", "/command-name", "/ With argument", "\u00a0/model\tname"])(
  "recognizes native command syntax %j", text => expect(isNativeCommand(text)).toBe(true),
);

it.each(["", "   ", "Continue", "Read /compact in the guide", "/Users/file", " /tmp/project/file ", "//server/share", "///", "/plugin/path command"])(
  "preserves ordinary prompts and absolute paths %j", text => expect(isNativeCommand(text)).toBe(false),
);

it.each(["cursor", "antigravity"] as const)("keeps decision ownership specific to saved %s terminal conversations", agent => {
  const workspace = { agent } as Workspace;
  const terminal = { conversation_mode: "terminal" } as Tab;
  expect(decisionSurfaceOf(workspace, terminal)).toBe("terminal");
  expect(decisionSurfaceOf(workspace, {} as Tab)).toBeUndefined();
  expect(decisionSurfaceOf(workspace, { ...terminal, conversation_mode: "chat" })).toBeUndefined();
  expect(decisionSurfaceOf(workspace, { ...terminal, choice: { agent: "claude", model: "", effort: "" } })).toBeUndefined();
  expect(decisionSurfaceOf({ agent: "claude" } as Workspace, { ...terminal, choice: { agent, model: "", effort: "" } })).toBe("terminal");
});

it("uses authenticated remote capability instead of the facade's placeholder provider", () => {
  const remote = { agent: "claude", remote: { owner: "alice", online: true } } as Workspace;
  expect(decisionSurfaceOf(remote, { decision_surface: "terminal" } as Tab)).toBe("terminal");
  expect(decisionSurfaceOf(remote, {} as Tab)).toBeUndefined();
  expect(decisionSurfaceOf({ agent: "claude" } as Workspace, { decision_surface: "terminal" } as Tab)).toBeUndefined();
});
