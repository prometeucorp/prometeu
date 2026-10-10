import { execFileSync } from "node:child_process";
import { readFile, readdir } from "node:fs/promises";
import { checkDependencies } from "./architecture-dependencies.mjs";

/// ADR 0003 fitness check: presentation consumes capabilities and ProviderId. Provider dispatch belongs
/// in the catalog or Rust adapters.
const presentation = [
  "src/chat.ts",
  "src/accounts.ts",
  "src/launcher.ts",
  "src/main.ts",
  "src/settings.ts",
  "src/statusbar.ts",
  "src/workspace.ts",
];

const forbidden = [
  /(?:===|!==)\s*["'](?:claude|codex|gemini|antigravity)["']|["'](?:claude|codex|gemini|antigravity)["']\s*(?:===|!==)/g,
  /\bcase\s+["'](?:claude|codex|gemini|antigravity)["']/g,
];

const sources = new Map();
for (const root of ["src", "relay/src", "packages/design-system/src"]) {
  // Windows returns backslash-separated entries; the checker resolves POSIX paths.
  const files = (await readdir(root, { recursive: true })).map((file) => file.replaceAll("\\", "/")).sort()
    .filter((file) => /\.[cm]?[jt]sx?$/.test(file) && !/\.(?:test|spec|d)\.[cm]?[jt]sx?$/.test(file));
  for (const file of files) sources.set(`${root}/${file}`, await readFile(`${root}/${file}`, "utf8"));
}
const failures = checkDependencies(sources);
const actionSettings = await readFile("src/action-settings.ts", "utf8");
if (!actionSettings.includes('from "./ui"') || /createElement\(["'](?:select|input|textarea)["']\)/.test(actionSettings)) {
  failures.push("src/action-settings.ts: reutilize os controles de src/ui.ts");
}
for (const file of presentation) {
  const source = await readFile(file, "utf8");
  for (const pattern of forbidden) {
    for (const match of source.matchAll(pattern)) {
      const line = source.slice(0, match.index).split("\n").length;
      failures.push(`${file}:${line}: decisão de UI por nome do provider: ${match[0]}`);
    }
  }
}

const timeline = await readFile("src/timeline.ts", "utf8");
for (const token of ["stream_event", "control_request", "control_response", "tool_use", "rate_limit_event", "step_update", "tool_info", "session/update"]) {
  if (timeline.includes(token)) {
    failures.push(`src/timeline.ts: protocolo de provider no reducer canônico: ${token}`);
  }
}

/// ADR 0002 fitness check: the Codex adapter emits canonical V1 directly. Legacy stream-json belongs
/// only in the Claude adapter and transcript compatibility path.
const codexPath = "src-tauri/crates/protocols/src/codex.rs";
const codex = await readFile(codexPath, "utf8");
const legacyCodex = [
  /"(?:stream_event|control_request|control_response|tool_use|tool_result|content_block_(?:start|delta|stop))"/g,
  /"type"\s*:\s*"(?:user|assistant|result)"/g,
  /\bLegacyAdapter\b/g,
];
for (const pattern of legacyCodex) {
  for (const match of codex.matchAll(pattern)) {
    const line = codex.slice(0, match.index).split("\n").length;
    failures.push(`${codexPath}:${line}: formato legado entre Codex e core: ${match[0]}`);
  }
}

/// The canonical contract must not depend on provider protocols. Rollback projection stays in its own
/// module outside core dependencies.
const conversation = await readFile("src-tauri/src/conversation.rs", "utf8");
for (const pattern of [
  /"(?:stream_event|control_request|control_response|tool_use|tool_result|rate_limit_event)"/g,
  /"type"\s*:\s*"(?:user|assistant|result|system|prometheus)"/g,
  /\b(?:LegacyAdapter|claude_command|legacy_mirror)\b/g,
]) {
  for (const match of conversation.matchAll(pattern)) {
    const line = conversation.slice(0, match.index).split("\n").length;
    failures.push(`src-tauri/src/conversation.rs:${line}: protocolo externo no core canônico: ${match[0]}`);
  }
}

const chat = await readFile("src-tauri/src/chat.rs", "utf8");
for (const match of chat.matchAll(/Command::new\s*\(\s*"(?:claude|codex|gemini|agy)"/g)) {
  const line = chat.slice(0, match.index).split("\n").length;
  failures.push(`src-tauri/src/chat.rs:${line}: processo de provider fora do adapter: ${match[0]}`);
}

/// ADR 0084: the Windows shell reaches execution through WSL, so it must not link the Unix execution
/// adapters, and the portable crates stay free of Tauri. cargo tree resolves the graph without building.
function cargoPackages(args) {
  const tree = execFileSync(
    "cargo",
    ["tree", "--manifest-path", "src-tauri/Cargo.toml", "--locked", "--edges", "normal", "--prefix", "none", ...args],
    { encoding: "utf8" },
  );
  return new Set(tree.split("\n").map((line) => line.split(" ")[0]).filter(Boolean));
}
const windowsShell = cargoPackages([
  "-p", "prometeu-wsl-desktop", "--features", "desktop", "--target", "x86_64-pc-windows-msvc",
]);
for (const crate of ["prometeu-process", "prometeu-profiles", "prometeu-tools", "prometeu-runtime"]) {
  if (windowsShell.has(crate)) failures.push(`prometeu-wsl-desktop: Unix execution dependency in the Windows shell: ${crate}`);
}
for (const crate of ["prometeu-core", "prometeu-protocols", "prometeu-bridge"]) {
  if (cargoPackages(["-p", crate, "--target", "all"]).has("tauri")) failures.push(`${crate}: portable crate depends on tauri`);
}

if (failures.length) {
  console.error(failures.join("\n"));
  console.error("Architecture checks failed. Keep dependencies within the documented boundaries.");
  process.exitCode = 1;
} else {
  console.log(
    `${sources.size} source modules without runtime import cycles; portable dependencies and canonical boundaries checked`,
  );
}
