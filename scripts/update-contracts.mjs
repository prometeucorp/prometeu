import { spawnSync } from "node:child_process";

const result = spawnSync("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--bin", "Prometeu",
  "boundary_contract::serialized_boundary_fixture_is_current", "--", "--exact"], {
  stdio: "inherit",
  env: { ...process.env, PROMETEU_UPDATE_CONTRACT_FIXTURES: "1" },
});
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
