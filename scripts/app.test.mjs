import assert from "node:assert/strict";
import test from "node:test";
import { plan } from "./app.mjs";

const devUrl = (port) => JSON.stringify({ build: { devUrl: `http://localhost:${port}` } });

test("terminal launches keep the default port and development root", () => {
  const launch = plan({ env: {}, home: "/Users/a" });
  assert.deepEqual(launch, {
    port: "1420",
    root: "/Users/a/.prometeu-dev",
    env: { PORT: "1420", PROMETEU_ROOT: "/Users/a/.prometeu-dev" },
    args: ["dev", "--config", "src-tauri/tauri.dev.conf.json", "--config", devUrl("1420")],
  });
});

test("workspace launches isolate the port and state root by workspace name", () => {
  const env = { PROMETEU_PORT: "1500", PROMETEU_WORKSPACE_NAME: "fix-a" };
  const unix = plan({ env, home: "/home/a" });
  assert.equal(unix.root, "/home/a/.prometeu-dev-fix-a");
  assert.deepEqual(unix.env, { PORT: "1500", PROMETEU_ROOT: "/home/a/.prometeu-dev-fix-a" });
  assert.equal(unix.args.at(-1), devUrl("1500"));
});

test("an explicit state root wins over the derived development root", () => {
  assert.equal(plan({ env: { PROMETEU_ROOT: "/tmp/root" }, home: "/home/a" }).root, "/tmp/root");
});
