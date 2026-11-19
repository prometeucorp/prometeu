// Build and verify the private terminal helper; never start an agent or publish an artifact.
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const root = fileURLToPath(new URL("../", import.meta.url));
const helper = path.join(root, "packages/terminal-state");

export function run(command, args, env = process.env, cwd = root) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: "inherit" });
    child.once("error", reject);
    child.once("exit", (code, signal) => code === 0 ? resolve() : reject(new Error(`Command failed (${signal ?? code})`)));
  });
}

async function install(env) {
  // The private package has its own lock; root frontend installation does not need native code.
  await run("npm", ["ci", "--prefix", helper], env);
}
async function build(env) {
  await run(process.execPath, [path.join(helper, "scripts/build-sea.mjs")], env);
  const manifest = JSON.parse(await readFile(path.join(helper, "dist/manifest.json"), "utf8"));
  return path.join(helper, "dist", manifest.binary);
}
export async function prepareTerminalState(env = process.env) {
  await install(env);
  return build(env);
}

async function check() {
  await install(process.env);
  await run("npm", ["run", "typecheck", "--prefix", helper]);
  await run("npm", ["test", "--prefix", helper]);
  const binary = await build(process.env);
  await run(process.execPath, [path.join(helper, "scripts/smoke-sea.mjs"), binary]);
  const env = { ...process.env, PROMETEU_TEST_TERMINAL_STATE_HELPER: binary, PROMETEU_TERMINAL_STATE_HELPER: binary };
  // Rustup discovers the pinned toolchain from cwd, not Cargo's --manifest-path.
  const nativeRoot = path.join(root, "src-tauri");
  await run("cargo", ["test", "-p", "prometeu-process", "terminal_state::tests::", "--", "--include-ignored"], env, nativeRoot);
  // Explicit names keep credentialed opt-in provider smokes out of the build gate.
  for (const name of [
    "chat::terminal::screen::tests::native_screen_tracks_chunks_replies_resize_and_freezes_without_a_helper",
    "chat::terminal::tests::native_discovery_replies_keep_psp_before_the_emulator_da_barrier",
    "chat::terminal::tests::cursor_submission_waits_for_painted_paste_and_retirement_cannot_send_enter",
    "chat::terminal::tests::native_local_input_releases_only_after_a_new_verified_empty_composer",
    "chat::terminal::tests::antigravity_initial_composer_admits_one_input_without_releasing_a_later_turn",
    "chat::terminal::tests::antigravity_interrupted_repaint_releases_escape_fence_and_admits_followup",
  ]) await run("cargo", ["test", "--bin", "Prometeu", name, "--", "--ignored", "--exact"], env, nativeRoot);
}

async function verifyBundle(bundle) {
  if (process.platform !== "darwin" || !bundle) throw new Error("A macOS .app path is required");
  await run("codesign", ["--verify", "--deep", "--strict", bundle]);
  for (const name of ["node-LICENSE", "xterm-LICENSE", "THIRD_PARTY_NOTICES.md"]) {
    const packaged = await readFile(path.join(bundle, "Contents/Resources/terminal-state-licenses", name));
    const expected = await readFile(path.join(helper, "dist/licenses", name));
    if (!packaged.equals(expected)) throw new Error(`Packaged terminal helper notice differs: ${name}`);
  }
  await run(process.execPath, [path.join(helper, "scripts/smoke-sea.mjs"), path.join(bundle, "Contents/MacOS/prometeu-terminal-state")]);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [mode, bundle] = process.argv.slice(2);
    if (mode === "build") await prepareTerminalState();
    else if (mode === "check") await check();
    else if (mode === "verify-bundle") await verifyBundle(bundle);
    else throw new Error("Usage: node scripts/terminal-state.mjs build|check|verify-bundle [app path]");
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
