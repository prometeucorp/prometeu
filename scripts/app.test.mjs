import assert from "node:assert/strict";
import test from "node:test";
import { plan } from "./app.mjs";

const devUrl = (port) => JSON.stringify({ build: { devUrl: `http://localhost:${port}` } });

test("unix terminal launches keep the default port and development root", () => {
  const launch = plan({ platform: "darwin", env: {}, home: "/Users/a" });
  assert.deepEqual(launch, {
    port: "1420",
    root: "/Users/a/.prometeu-dev",
    env: { PORT: "1420", PROMETEU_ROOT: "/Users/a/.prometeu-dev" },
    args: ["dev", "--config", "src-tauri/tauri.dev.conf.json", "--config", devUrl("1420")],
  });
});

test("workspace launches isolate the port and state root by workspace name", () => {
  const env = { PROMETEU_PORT: "1500", PROMETEU_WORKSPACE_NAME: "fix-a" };
  const unix = plan({ platform: "linux", env, home: "/home/a" });
  assert.equal(unix.root, "/home/a/.prometeu-dev-fix-a");
  assert.deepEqual(unix.env, { PORT: "1500", PROMETEU_ROOT: "/home/a/.prometeu-dev-fix-a" });
  assert.equal(unix.args.at(-1), devUrl("1500"));

  const windows = plan({ platform: "win32", env, home: "/home/a" });
  assert.equal(windows.root, "/home/a/.local/share/prometeu-windows-dev-fix-a/state");
  assert.equal(windows.env.PORT, "1500");
  assert.equal(windows.args.at(-1), devUrl("1500"));
});

test("windows runs the wsl shell with a development root inside wsl and never the unix config", () => {
  const launch = plan({ platform: "win32", env: {}, home: "/home/a" });
  assert.deepEqual(launch, {
    port: "1421",
    root: "/home/a/.local/share/prometeu-windows-dev/state",
    env: { PORT: "1421", PROMETEU_WINDOWS_RUNTIME_ROOT: "/home/a/.local/share/prometeu-windows-dev/state" },
    args: ["dev", "--config", devUrl("1421")],
  });
});

test("explicit state roots win over the derived development roots", () => {
  assert.equal(plan({ platform: "linux", env: { PROMETEU_ROOT: "/tmp/root" }, home: "/home/a" }).root, "/tmp/root");
  const windows = plan({ platform: "win32", env: { PROMETEU_WINDOWS_RUNTIME_ROOT: "/srv/state" }, home: "/home/a" });
  assert.equal(windows.root, "/srv/state");
  assert.equal(windows.env.PROMETEU_WINDOWS_RUNTIME_ROOT, "/srv/state");
});
