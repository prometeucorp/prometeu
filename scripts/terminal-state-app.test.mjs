import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { terminalAppPlan } from "./terminal-state-app.mjs";
import { root } from "./terminal-state.mjs";

test("macOS carries the helper without dropping worktree roots, ports or caller config", () => {
  const args = ["dev", "--config", "src-tauri/tauri.dev.conf.json", "--config", '{"build":{"devUrl":"http://localhost:1515"}}'];
  const env = { PROMETEU_ROOT: "/tmp/work tree", PORT: "1515", PROMETEU_TERMINAL_STATE_HELPER: "obsolete" };
  const binary = "/tmp/work tree/helper";
  const launch = terminalAppPlan(args, env, "darwin", binary);
  assert.deepEqual(launch.args, [...args, "--config", path.join(root, "src-tauri/tauri.terminal-state.conf.json")]);
  assert.deepEqual(launch.env, { ...env, PROMETEU_TERMINAL_STATE_HELPER: binary });
  assert.equal(env.PROMETEU_TERMINAL_STATE_HELPER, "obsolete");
  assert.equal(args.length, 5);
});

test("a macOS build cannot silently omit a failed helper preparation", () => {
  assert.throws(() => terminalAppPlan(["build"], {}, "darwin"), /must be built/);
});

test("other platform builds retain their current arguments and environment", () => {
  const args = ["build", "--bundles", "appimage"];
  const env = { PROMETEU_ROOT: "/tmp/state" };
  assert.deepEqual(terminalAppPlan(args, env, "linux"), { args, env });
  assert.throws(() => terminalAppPlan(["signer"], {}, "linux"), /Expected Tauri/);
});
