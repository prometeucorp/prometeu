import { once } from "node:events";
import type { Readable, Writable } from "node:stream";
import { LIMITS, type Response } from "./protocol.ts";
import { TerminalStateService } from "./service.ts";

/** Bounded NDJSON framing, including a missing newline and split UTF-8 JSON chunks. */
export async function run(input: Readable, output: Writable): Promise<void> {
  const service = new TerminalStateService();
  let pending = Buffer.alloc(0);
  const send = async (response: Response) => {
    if (!output.write(`${JSON.stringify(response)}\n`)) await once(output, "drain");
  };
  const line = async (data: Buffer) => {
    try { await send(await service.request(JSON.parse(data.toString("utf8")))); }
    catch { await send({ v: 1, id: null, ok: false, error: { code: "invalid_request" } }); }
  };
  try {
    for await (const value of input) {
      const chunk = Buffer.isBuffer(value) ? value : Buffer.from(value);
      let offset = 0;
      while (offset < chunk.length) {
        const newline = chunk.indexOf(10, offset);
        const end = newline === -1 ? chunk.length : newline;
        if (pending.length + end - offset > LIMITS.lineBytes) {
          await send({ v: 1, id: null, ok: false, error: { code: "limit" } });
          return;
        }
        pending = Buffer.concat([pending, chunk.subarray(offset, end)]);
        if (newline !== -1) { await line(pending); pending = Buffer.alloc(0); }
        offset = end + 1;
      }
    }
    if (pending.length) await line(pending);
  } finally { await service.dispose(); }
}

