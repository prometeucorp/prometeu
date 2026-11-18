import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const dist = new URL("../dist/", import.meta.url);
const manifest = JSON.parse(await readFile(new URL("manifest.json", dist), "utf8"));
const binary = process.argv[2] ?? fileURLToPath(new URL(manifest.binary, dist));
const child = spawn(binary, [], {
  // Proves the helper neither resolves installed Node nor honors injected Node execution flags.
  env: { PATH: "", NODE_OPTIONS: "--require=/prometeu-missing-injected-module", NODE_PATH: "/prometeu-missing-node-path" },
  stdio: ["pipe", "pipe", "pipe"],
});
let output = "", diagnostics = "";
child.stdout.on("data", data => { output += data; });
child.stderr.on("data", data => { diagnostics += data; });
const finished = new Promise((resolve, reject) => { child.on("error", reject); child.on("close", resolve); });
const envelope = { v: 1, session: "smoke", generation: "generation" };
const bytes = Buffer.from("normal\x1b[?1049h\x1b[?2004h\x1b]8;;https://example.test\x1b\\é\x1b[6n\x1b]11;?\x1b\\");
const requests = [
  { ...envelope, id: "create", op: "create", cols: 20, rows: 4, scrollback: 100 },
  { ...envelope, id: "write", op: "write", data: bytes.toString("base64"), seq: bytes.length },
  { ...envelope, id: "snapshot", op: "snapshot" },
  { ...envelope, id: "resize", op: "resize", cols: 30, rows: 6, seq: bytes.length },
  { ...envelope, id: "dispose", op: "dispose" },
];
const started = performance.now();
child.stdin.end(requests.map(request => JSON.stringify(request)).join("\n") + "\n");
const timeout = setTimeout(() => child.kill("SIGKILL"), 10_000);
try {
  assert.equal(await finished, 0, diagnostics);
  const responses = output.trim().split("\n").map(line => JSON.parse(line));
  assert.deepEqual(responses.map(response => response.id), requests.map(request => request.id));
  for (const response of responses) assert.equal(response.ok, true, JSON.stringify(response.error));
  const snapshot = responses[2].result;
  assert.equal(snapshot.active, "alternate");
  assert.equal(snapshot.modes.bracketedPasteMode, true);
  assert.match(snapshot.normal.ansi, /normal/);
  assert.equal(snapshot.links[0].uri, "https://example.test");
  assert.equal(snapshot.seq, bytes.length);
  assert.equal(snapshot.viewportLines.length, snapshot.rows);
  assert.equal(snapshot.viewportStyles.length, snapshot.rows);
  assert.equal(snapshot.viewportCharacterColumns.length, snapshot.rows);
  assert.ok(snapshot.viewportCharacterColumns.every((columns, row) => columns.length === Array.from(snapshot.viewportLines[row]).length));
  assert.ok(snapshot.viewportStyles.every(runs => runs[0].start === 0 && runs.at(-1).end === snapshot.cols));
  assert.ok(snapshot.viewportLines.some(line => line.includes("é")));
  assert.equal(snapshot.appearance.background, "#282c34");
  assert.deepEqual(responses[1].result.replies.map(reply => Buffer.from(reply, "base64").toString()).slice(-1), ["\x1b]11;rgb:2828/2c2c/3434\x1b\\"]);
  assert.equal(responses[1].result.replies.length, 2);
  process.stdout.write(`SEA smoke passed without PATH/Node options (${Math.round(performance.now() - started)} ms, ${manifest.target}).\n`);
} finally { clearTimeout(timeout); }
