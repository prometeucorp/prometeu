// Start this worktree's development app independently of other running worktrees.
// Separate Vite ports avoid strictPort conflicts; separate state roots avoid sharing board state
// and worktrees.
// Workspace scripts supply PROMETEU_PORT and PROMETEU_WORKSPACE_NAME. Terminal launches retain the
// default port and development root.
// Windows has no Unix desktop backend: it runs the WSL shell (ADR 0084) with a development runtime
// root inside the default distribution.
import { spawn } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { wslApp, wslHome } from "./wsl-app.mjs";

/** Resolves the port and state root for a platform without touching the system. */
export function plan({ platform, env, home }) {
  const name = env.PROMETEU_WORKSPACE_NAME || "";
  const suffix = name ? `-${name}` : "";
  if (platform === "win32") {
    return {
      port: env.PROMETEU_PORT || "1421",
      root: env.PROMETEU_WINDOWS_RUNTIME_ROOT || `${home}/.local/share/prometeu-windows-dev${suffix}/state`,
    };
  }
  return { port: env.PROMETEU_PORT || "1420", root: env.PROMETEU_ROOT || join(home, `.prometeu-dev${suffix}`) };
}

async function main() {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const windows = process.platform === "win32";
  const { port, root: stateRoot } = plan({ platform: process.platform, env: process.env, home: windows ? wslHome() : homedir() });
  const env = { ...process.env, PORT: port, [windows ? "PROMETEU_WINDOWS_RUNTIME_ROOT" : "PROMETEU_ROOT"]: stateRoot };
  console.log(`Prometeu dev · vite em ${port} · raiz em ${stateRoot}`);

  // The last --config merges a JSON patch over the previous ones, replacing the devUrl port.
  const devUrl = JSON.stringify({ build: { devUrl: `http://localhost:${port}` } });
  if (windows) {
    await wslApp(["dev", "--config", devUrl], env);
    return;
  }
  const tauri = fileURLToPath(new URL("../node_modules/@tauri-apps/cli/tauri.js", import.meta.url));
  const child = spawn(process.execPath, [tauri, "dev", "--config", "src-tauri/tauri.dev.conf.json", "--config", devUrl], { cwd: root, env, stdio: "inherit" });
  await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", code => code === 0 ? resolve() : reject(new Error(`tauri exited with ${code}`)));
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
