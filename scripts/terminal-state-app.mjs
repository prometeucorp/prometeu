// macOS application builds carry a self-contained helper; other hosts keep their current bundle.
import path from "node:path";
import { pathToFileURL } from "node:url";
import { prepareTerminalState, root, run } from "./terminal-state.mjs";

export function terminalAppPlan(args, env, platform, binary) {
  if (!["dev", "build"].includes(args[0])) throw new Error("Expected Tauri dev or build");
  if (platform !== "darwin") return { args: [...args], env: { ...env } };
  if (!binary) throw new Error("The macOS terminal helper must be built before Tauri starts");
  return {
    args: [...args, "--config", path.join(root, "src-tauri/tauri.terminal-state.conf.json")],
    env: { ...env, PROMETEU_TERMINAL_STATE_HELPER: binary },
  };
}
export async function terminalStateApp(args, env = process.env) {
  if (!["dev", "build"].includes(args[0])) throw new Error("Expected Tauri dev or build");
  const binary = process.platform === "darwin" ? await prepareTerminalState(env) : undefined;
  const launch = terminalAppPlan(args, env, process.platform, binary);
  await run(process.execPath, [path.join(root, "node_modules/@tauri-apps/cli/tauri.js"), ...launch.args], launch.env);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await terminalStateApp(process.argv.slice(2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
