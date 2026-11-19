import assert from "node:assert/strict";
import test from "node:test";
import { spawn } from "node:child_process";
import { Readable, Writable } from "node:stream";
import { LIMITS, type Response, type Snapshot } from "../src/protocol.ts";
import { TerminalStateService } from "../src/service.ts";
import { run } from "../src/stdio.ts";

const envelope = { v: 1, id: "request", session: "session", generation: "generation" };
const request = (op: string, args = {}) => ({ ...envelope, op, ...args });
function success(response: Response) { if (!response.ok) assert.fail(response.error.code); return response.result; }

test("concurrent requests retain create/write/resize/snapshot order", async () => {
  const service = new TerminalStateService();
  const data = Buffer.from("hello");
  const responses = await Promise.all([
    service.request(request("create", { cols: 20, rows: 4 })),
    service.request(request("write", { data: data.toString("base64"), seq: data.length })),
    service.request(request("resize", { cols: 10, rows: 5, seq: data.length })),
    service.request(request("snapshot")),
  ]);
  responses.forEach(success);
  const snapshot = success(responses[3]) as Snapshot;
  assert.equal(snapshot.cols, 10);
  assert.equal(snapshot.seq, 5);
  assert.match(snapshot.normal.ansi, /hello/);
  await service.dispose();
});

test("stale generations, replayed writes and output gaps never mutate current state", async () => {
  const service = new TerminalStateService();
  success(await service.request(request("create", { cols: 20, rows: 4 })));
  for (const op of ["write", "resize", "snapshot", "dispose"]) {
    const response = await service.request(request(op, { generation: "old", data: "eA==", seq: 1, cols: 10, rows: 3 }));
    assert.deepEqual(response, { v: 1, id: "request", ok: false, error: { code: "stale_generation" } });
  }
  success(await service.request(request("write", { data: "eA==", seq: 1 })));
  for (const seq of [1, 3]) {
    const response = await service.request(request("write", { data: "eA==", seq }));
    assert.equal(response.ok ? "success" : response.error.code, "sequence");
  }
  success(await service.request(request("dispose")));
  success(await service.request(request("create", { generation: "new", cols: 20, rows: 4 })));
  const response = await service.request(request("write", { data: "eA==", seq: 1 }));
  assert.equal(response.ok ? "success" : response.error.code, "stale_generation");
  const snapshot = success(await service.request(request("snapshot", { generation: "new" }))) as Snapshot;
  assert.equal(snapshot.seq, 0);
  await service.dispose();
});

test("invalid base64, oversized grids and unsupported versions fail before creation or writes", async () => {
  const service = new TerminalStateService();
  for (const invalid of [request("create", { cols: LIMITS.cols + 1, rows: 4 }), request("create", { cols: 500, rows: 300, scrollback: 5000 }), { ...request("snapshot"), v: 2 }]) {
    assert.equal((await service.request(invalid)).ok, false);
  }
  success(await service.request(request("create", { cols: 20, rows: 4 })));
  assert.equal((await service.request(request("write", { data: "%%%", seq: 1 }))).ok, false);
  assert.equal((success(await service.request(request("snapshot"))) as Snapshot).seq, 0);
  await service.dispose();
  assert.equal((await service.request(request("snapshot"))).ok, false);
});

test("stdio carries only framed responses and handles split input and a final line without newline", async () => {
  const child = spawn(process.execPath, [new URL("../src/cli.ts", import.meta.url).pathname], { stdio: ["pipe", "pipe", "pipe"] });
  let out = "", err = "";
  child.stdout.on("data", data => { out += data; });
  child.stderr.on("data", data => { err += data; });
  const done = new Promise<number | null>(resolve => child.on("close", resolve));
  const frames = [
    request("create", { id: "create", cols: 20, rows: 4 }),
    request("write", { id: "write", data: Buffer.from("é\x1b[6n").toString("base64"), seq: 6 }),
    request("snapshot", { id: "snapshot" }),
  ].map(v => JSON.stringify(v)).join("\n");
  child.stdin.write(frames.slice(0, 13));
  child.stdin.end(frames.slice(13));
  assert.equal(await done, 0);
  assert.equal(err, "");
  const responses = out.trim().split("\n").map(v => JSON.parse(v) as Response);
  assert.deepEqual(responses.map(r => r.id), ["create", "write", "snapshot"]);
  responses.forEach(success);
  assert.equal((success(responses[2]) as Snapshot).seq, 6);
});

test("malformed lines recover, while oversized input closes without processing its suffix", async () => {
  const capture = async (chunks: Buffer[]) => {
    let text = "";
    const output = new Writable({ write(chunk, _encoding, callback) { text += chunk; callback(); } });
    await run(Readable.from(chunks), output);
    return text.trim().split("\n").map(line => JSON.parse(line) as Response);
  };
  const create = JSON.stringify(request("create", { cols: 20, rows: 4 }));
  const recovered = await capture([Buffer.from(`bad json\n${create}\n`)]);
  assert.equal(recovered[0].ok, false);
  success(recovered[1]);
  const rejected = await capture([Buffer.alloc(LIMITS.lineBytes, 32), Buffer.from(`x\n${create}\n`)]);
  assert.deepEqual(rejected, [{ v: 1, id: null, ok: false, error: { code: "limit" } }]);
});
