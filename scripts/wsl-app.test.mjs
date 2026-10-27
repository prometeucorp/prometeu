import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { BUILD_IN_WSL } from "./wsl-app.mjs";

const flock = spawnSync("sh", ["-c", "command -v flock"]).status === 0;

/// Runs the WSL build script with stand-ins for cargo and wslpath; cargo writes the checkout's revision into the shared target.
function build(bin, cache, checkout, output) {
  const child = spawn("sh", ["-c", BUILD_IN_WSL, "sh", checkout, output], {
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, XDG_CACHE_HOME: cache },
    stdio: "inherit",
  });
  return new Promise((resolve, reject) => child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`build exited with ${code}`)))));
}

test("concurrent runtime builds each copy their own checkout's binary", { skip: !flock && "flock (util-linux) is not installed" }, async () => {
  const dir = mkdtempSync(join(tmpdir(), "wsl-runtime-"));
  const bin = join(dir, "bin");
  mkdirSync(bin);
  writeFileSync(join(bin, "wslpath"), '#!/bin/sh\nprintf %s "$2"\n');
  // The pause after writing stands for the moment Cargo has released its lock but the copy has not run.
  writeFileSync(join(bin, "cargo"), '#!/bin/sh\nmkdir -p "$CARGO_TARGET_DIR/release"\ncat revision > "$CARGO_TARGET_DIR/release/prometeu-runtime"\nsleep 0.3\n');
  chmodSync(join(bin, "wslpath"), 0o755);
  chmodSync(join(bin, "cargo"), 0o755);
  const checkouts = ["a", "b"].map((name) => {
    const checkout = join(dir, name);
    mkdirSync(checkout);
    writeFileSync(join(checkout, "revision"), name);
    return { name, checkout, output: join(dir, `${name}.out`) };
  });
  await Promise.all(checkouts.map(({ checkout, output }) => build(bin, join(dir, "cache"), checkout, output)));
  for (const { name, output } of checkouts) assert.equal(readFileSync(output, "utf8"), name);
});
