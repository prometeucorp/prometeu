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
  const files = (await readdir(root, { recursive: true })).sort()
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

if (failures.length) {
  console.error(failures.join("\n"));
  console.error("Architecture checks failed. Keep dependencies within the documented boundaries.");
  process.exitCode = 1;
} else {
  console.log(
    `${sources.size} source modules without runtime import cycles; portable dependencies and canonical boundaries checked`,
  );
}
