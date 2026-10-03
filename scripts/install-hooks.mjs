import { spawnSync } from "node:child_process";

// npm runs lifecycle scripts under cmd.exe on Windows and a shell on Unix.
// Use Git's argument interface so hook setup does not depend on shell syntax.
const repository = spawnSync("git", ["rev-parse", "--git-dir"], { stdio: "ignore" });
if (repository.status === 0) {
  spawnSync("git", ["config", "core.hooksPath", ".githooks"], { stdio: "ignore" });
}
