import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmod, copyFile, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import postject from "postject";

export const NODE_VERSION = "24.14.0";
// Official release archive hashes: https://nodejs.org/download/release/v24.14.0/SHASUMS256.txt
const targets = {
  "darwin-arm64": { triple: "aarch64-apple-darwin", sha256: "a1a54f46a750d2523d628d924aab61758a51c9dad3e0238beb14141be9615dd3" },
  "linux-x64": { triple: "x86_64-unknown-linux-gnu", sha256: "dbf5b8665dec15e59e6359a517fefb47b23fdb9152d8def975b9bca3dfc6d355" },
};
const root = fileURLToPath(new URL("../", import.meta.url));
const platform = `${process.platform}-${process.arch}`;
const target = targets[platform];
if (!target) throw new Error(`Unsupported terminal helper build target: ${platform}`);
const dist = join(root, "dist");
const cache = join(dist, "cache");
const distribution = `node-v${NODE_VERSION}-${platform}`;
const archive = join(cache, `${distribution}.tar.gz`);
const runtime = join(cache, distribution, "bin", "node");
const binary = join(dist, `prometeu-terminal-state-${target.triple}`);
const temporaryBinary = `${binary}.building`;
const env = { ...process.env };
delete env.NODE_OPTIONS;
delete env.NODE_PATH;
const run = (file, args) => execFileSync(file, args, { env, stdio: ["ignore", "pipe", "pipe"], maxBuffer: 2 * 1024 * 1024 });
const hash = data => createHash("sha256").update(data).digest("hex");

await mkdir(cache, { recursive: true });
let bytes;
try { bytes = await readFile(archive); } catch (error) { if (error.code !== "ENOENT") throw error; }
if (!bytes) {
  const response = await fetch(`https://nodejs.org/download/release/v${NODE_VERSION}/${distribution}.tar.gz`);
  if (!response.ok) throw new Error(`Node archive download failed: ${response.status}`);
  bytes = Buffer.from(await response.arrayBuffer());
}
if (hash(bytes) !== target.sha256) throw new Error("Node archive checksum does not match the pinned release");
await writeFile(archive, bytes);
// Reextract the verified archive; a modified cached executable must not change the build input.
await rm(join(cache, distribution), { recursive: true, force: true });
run("tar", ["-xzf", archive, "-C", cache]);
if (run(runtime, ["--version"]).toString().trim() !== `v${NODE_VERSION}`)
  throw new Error("Node executable version does not match the SEA builder pin");

const script = join(dist, "terminal-state.cjs");
await build({
  absWorkingDir: root,
  entryPoints: ["src/cli.ts"], outfile: script,
  bundle: true, platform: "node", format: "cjs", target: "node24",
  sourcemap: false, legalComments: "eof", logLevel: "warning",
});
const blob = join(dist, "terminal-state.blob");
const config = join(dist, "sea-config.json");
await writeFile(config, JSON.stringify({
  main: script, output: blob, disableExperimentalSEAWarning: true,
  useSnapshot: false, useCodeCache: false,
  // xterm's parser needs no generated executable memory. This permits Tauri's existing
  // hardened-runtime entitlements to remain unchanged for the app and its sidecar.
  execArgv: ["--jitless"], execArgvExtension: "none",
}));
run(runtime, ["--experimental-sea-config", config]);
await copyFile(runtime, temporaryBinary);
await chmod(temporaryBinary, 0o755);
if (process.platform === "darwin") run("codesign", ["--remove-signature", temporaryBinary]);
await postject.inject(temporaryBinary, "NODE_SEA_BLOB", await readFile(blob), {
  sentinelFuse: "NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2",
  ...(process.platform === "darwin" ? { machoSegmentName: "NODE_SEA" } : {}),
});
if (process.platform === "darwin") {
  run("codesign", ["--force", "--sign", "-", "--identifier", "co.prometeu.terminal-state", "--options", "runtime", temporaryBinary]);
  run("codesign", ["--verify", "--strict", temporaryBinary]);
}

const licenses = join(dist, "licenses");
await mkdir(licenses, { recursive: true });
for (const [source, name] of [
  [join(cache, distribution, "LICENSE"), "node-LICENSE"],
  [join(root, "THIRD_PARTY_NOTICES.md"), "THIRD_PARTY_NOTICES.md"],
  // Headless and serialize ship the same upstream MIT code; their npm archives omit LICENSE.
  [join(root, "node_modules/@xterm/xterm/LICENSE"), "xterm-LICENSE"],
]) await copyFile(source, join(licenses, name));
await rename(temporaryBinary, binary);
await writeFile(join(dist, "manifest.json"), JSON.stringify({
  v: 1, nodeVersion: NODE_VERSION, platform, target: target.triple,
  nodeArchiveSha256: target.sha256, binary: binary.slice(dirname(binary).length + 1),
  binarySha256: hash(await readFile(binary)), jitless: true,
}, null, 2) + "\n");
process.stdout.write(`${binary}\n`);
