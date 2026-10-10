import { spawn, spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { resolve } from "node:path";
const root = new URL("../", import.meta.url);
async function run(command, args, options) {
  const child = spawn(command, args, { stdio: "inherit", ...options });
  return new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", code => code === 0 ? resolve() : reject(new Error(`${command} exited with ${code}`)));
  });
}

// Windows checkouts build the host inside the default distribution, the same one the
// app attaches to. A login shell loads the PATH rustup writes to ~/.profile, and a
// Linux-side target directory keeps Linux artifacts away from the MSVC target and
// avoids building through /mnt/c.
const BUILD_IN_WSL = `set -eu
if ! command -v cargo >/dev/null; then
  echo "Rust is not installed in the default WSL distribution. Install it there with rustup (https://rustup.rs), plus build-essential, then retry." >&2
  exit 1
fi
cd "$(wslpath -a "$1")"
target="\${XDG_CACHE_HOME:-$HOME/.cache}/prometeu/runtime-target"
CARGO_TARGET_DIR="$target" cargo build --locked --manifest-path src-tauri/Cargo.toml -p prometeu-runtime --release
cp "$target/release/prometeu-runtime" "$(wslpath -a "$2")"
`;

async function wsl(script, args) {
  try {
    await run("wsl.exe", ["--exec", "sh", "-lc", script, "sh", ...args]);
  } catch (error) {
    if (error.code === "ENOENT") throw new Error("WSL is required to run Prometeu on Windows. Install it with `wsl --install` and retry.");
    throw error;
  }
}

/** Reads the Linux home of the default WSL distribution. */
export function wslHome() {
  const result = spawnSync("wsl.exe", ["--exec", "sh", "-c", 'printf %s "$HOME"'], { encoding: "utf8" });
  if (result.error?.code === "ENOENT") throw new Error("WSL is required to run Prometeu on Windows. Install it with `wsl --install` and retry.");
  if (result.status !== 0 || !result.stdout.startsWith("/")) throw new Error("Could not read the home directory of the default WSL distribution.");
  return result.stdout;
}

async function buildRuntime(env) {
  if (process.platform === "linux") {
    await run("cargo", ["build", "--locked", "--manifest-path", "src-tauri/Cargo.toml", "-p", "prometeu-runtime", "--release"], { cwd: fileURLToPath(root), env });
    return fileURLToPath(new URL("src-tauri/target/release/prometeu-runtime", root));
  }
  if (process.platform === "win32") {
    const output = fileURLToPath(new URL("src-tauri/target/wsl-runtime/prometeu-runtime", root));
    mkdirSync(resolve(output, ".."), { recursive: true });
    await wsl(BUILD_IN_WSL, [fileURLToPath(root), output]);
    return output;
  }
}

/** Runs the Tauri CLI for the Windows shell with arguments such as `dev` or `build`. */
export async function wslApp(args, env = { ...process.env }) {
  if (process.platform === "win32" && spawnSync("cargo", ["--version"], { stdio: "ignore" }).error) {
    throw new Error("Rust is not installed on Windows. Install rustup with the MSVC toolchain and the Visual Studio C++ Build Tools, then retry.");
  }
  // Linux and Windows builds ship their matching host in the Windows executable. Other
  // builders supply the Linux CI artifact explicitly; dependency-only builds remain possible.
  if (!env.PROMETEU_WSL_RUNTIME) {
    const runtime = await buildRuntime(env);
    if (runtime) env.PROMETEU_WSL_RUNTIME = runtime;
  }
  if (args.includes("--bundles") && !env.PROMETEU_WSL_RUNTIME) {
    throw new Error("Windows installers require PROMETEU_WSL_RUNTIME pointing to the matching Linux runtime.");
  }
  if (env.PROMETEU_WSL_RUNTIME) env.PROMETEU_WSL_RUNTIME = resolve(env.PROMETEU_WSL_RUNTIME);
  await run(process.execPath, [fileURLToPath(new URL("node_modules/@tauri-apps/cli/tauri.js", root)), ...args], {
    cwd: fileURLToPath(new URL("src-tauri/crates/wsl-desktop", root)), env,
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await wslApp(process.argv.slice(2)); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
