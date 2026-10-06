// Start this worktree's development app independently of other running worktrees.
// Separate Vite ports avoid strictPort conflicts; separate state roots avoid sharing board state
// and worktrees.
// Workspace scripts supply PROMETEU_PORT and PROMETEU_WORKSPACE_NAME. Terminal launches retain the
// default port and development root.
// Windows has no Unix desktop backend: it runs the WSL shell (ADR 0084) with a development runtime
// root inside the default distribution.
import { spawn } from "node:child_process";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { wslApp, wslHome } from "./wsl-app.mjs";

/**
 * Resolves the port, state root, environment and Tauri arguments for a platform without touching
 * the system. On Windows, `home` is the Linux home of the default WSL distribution.
 */
export function plan({ platform, env, home }) {
  const name = env.PROMETEU_WORKSPACE_NAME || "";
  const suffix = name ? `-${name}` : "";
  const windows = platform === "win32";
  const port = env.PROMETEU_PORT || (windows ? "1421" : "1420");
  const root = windows
    ? env.PROMETEU_WINDOWS_RUNTIME_ROOT || `${home}/.local/share/prometeu-windows-dev${suffix}/state`
    : env.PROMETEU_ROOT || path.posix.join(home, `.prometeu-dev${suffix}`);
  // The last --config merges a JSON patch over the previous ones, replacing the devUrl port.
  const devUrl = JSON.stringify({ build: { devUrl: `http://localhost:${port}` } });
  return {
    port,
    root,
    env: { PORT: port, [windows ? "PROMETEU_WINDOWS_RUNTIME_ROOT" : "PROMETEU_ROOT"]: root },
    args: windows ? ["dev", "--config", devUrl] : ["dev", "--config", "src-tauri/tauri.dev.conf.json", "--config", devUrl],
  };
}

async function main() {
  const windows = process.platform === "win32";
  const launch = plan({ platform: process.platform, env: process.env, home: windows ? wslHome() : homedir() });
  const env = { ...process.env, ...launch.env };
  console.log(`Prometeu dev · vite em ${launch.port} · raiz em ${launch.root}`);
  if (windows) {
    await wslApp(launch.args, env);
    return;
  }
  const tauri = fileURLToPath(new URL("../node_modules/@tauri-apps/cli/tauri.js", import.meta.url));
  const child = spawn(process.execPath, [tauri, ...launch.args], { cwd: fileURLToPath(new URL("../", import.meta.url)), env, stdio: "inherit" });
  await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", code => code === 0 ? resolve() : reject(new Error(`tauri exited with ${code}`)));
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
