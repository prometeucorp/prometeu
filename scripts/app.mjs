// Start this worktree's development app independently of other running worktrees.
// Separate Vite ports avoid strictPort conflicts; separate state roots avoid sharing board state
// and worktrees.
// Workspace scripts supply PROMETEU_PORT and PROMETEU_WORKSPACE_NAME. Terminal launches retain the
// default port and development root.
import { spawn } from "node:child_process";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { terminalStateApp } from "./terminal-state-app.mjs";

/** Resolves the port, state root, environment and Tauri arguments without touching the system. */
export function plan({ env, home }) {
  const name = env.PROMETEU_WORKSPACE_NAME || "";
  const suffix = name ? `-${name}` : "";
  const port = env.PROMETEU_PORT || "1420";
  const root = env.PROMETEU_ROOT || path.posix.join(home, `.prometeu-dev${suffix}`);
  // The last --config merges a JSON patch over the previous ones, replacing the devUrl port.
  const devUrl = JSON.stringify({ build: { devUrl: `http://localhost:${port}` } });
  return {
    port,
    root,
    env: { PORT: port, PROMETEU_ROOT: root },
    args: ["dev", "--config", "src-tauri/tauri.dev.conf.json", "--config", devUrl],
  };
}

async function main() {
  const launch = plan({ env: process.env, home: homedir() });
  const env = { ...process.env, ...launch.env };
  console.log(`Prometeu dev · vite em ${launch.port} · raiz em ${launch.root}`);
  if (process.platform === "darwin") {
    await terminalStateApp(launch.args, env);
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
