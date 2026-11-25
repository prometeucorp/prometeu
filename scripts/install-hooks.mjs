import { spawnSync } from "node:child_process";

// Use Git's argument interface so hook setup does not depend on shell syntax.
const repository = spawnSync("git", ["rev-parse", "--git-dir"], { stdio: "ignore" });
if (repository.status === 0) {
  spawnSync("git", ["config", "core.hooksPath", ".githooks"], { stdio: "ignore" });
}
