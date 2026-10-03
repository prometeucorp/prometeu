import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
const root = new URL("../", import.meta.url);
const env = { ...process.env };
async function run(command, args, options) {
  const child = spawn(command, args, { stdio: "inherit", ...options });
  return new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", code => code === 0 ? resolve() : reject(new Error(`${command} exited with ${code}`)));
  });
}
try {
  // Linux builds ship their matching host in the Windows executable. Other builders
  // supply the Linux CI artifact explicitly; dependency-only builds remain possible.
  if (!env.PROMETEU_WSL_RUNTIME && process.platform === "linux") {
    await run("cargo", ["build", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "-p", "prometeu-runtime", "--release"], { cwd: fileURLToPath(root), env });
    env.PROMETEU_WSL_RUNTIME = fileURLToPath(new URL("src-tauri/target/release/prometeu-runtime", root));
  }
  if (process.argv.includes("--bundles") && !env.PROMETEU_WSL_RUNTIME) {
    throw new Error("Windows installers require PROMETEU_WSL_RUNTIME pointing to the matching Linux runtime.");
  }
  if (env.PROMETEU_WSL_RUNTIME) env.PROMETEU_WSL_RUNTIME = resolve(env.PROMETEU_WSL_RUNTIME);
  await run(process.execPath, [fileURLToPath(new URL("node_modules/@tauri-apps/cli/tauri.js", root)), ...process.argv.slice(2)], {
    cwd: fileURLToPath(new URL("src-tauri/crates/wsl-desktop", root)), env,
  });
} catch (error) { console.error(error.message); process.exitCode = 1; }
