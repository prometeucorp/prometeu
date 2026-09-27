// Synthetic stdio peer for transport tests. No provider executable, account, or network is used.
import assert from "node:assert/strict";
import { createInterface } from "node:readline";

const mode = process.argv[2];
const send = (frame) => process.stdout.write(`${JSON.stringify(frame)}\n`);
const reply = (request, result) => send({ id: request.id, result });
const notice = (method, params) => send({ method, params: { threadId: "thread", ...params } });
let initialized = false;
let opened = false;
let turning = false;
const input = createInterface({ input: process.stdin });
function finish(status) {
  notice("turn/completed", { turn: { id: "turn", status } });
  input.close();
  process.stdin.destroy();
}
for await (const line of input) {
  const request = JSON.parse(line);
  switch (request.method) {
    case "initialize":
      assert.equal(initialized, false);
      initialized = true;
      reply(request, {});
      break;
    case "initialized":
      assert.equal(initialized, true);
      break;
    case "account/rateLimits/read":
      reply(request, {});
      break;
    case "thread/start":
    case "thread/resume":
      assert.equal(initialized, true);
      assert.equal(request.method, mode === "resume" ? "thread/resume" : "thread/start");
      if (mode === "resume") assert.equal(request.params.threadId, "thread");
      opened = true;
      reply(request, { thread: { id: "thread" } });
      break;
    case "turn/start": {
      assert.equal(opened, true);
      assert.equal(turning, false, "Queued input must be dispatched only once");
      assert.equal(request.params.threadId, "thread");
      assert.deepEqual(request.params.input, [{ type: "text", text: "Review the patch", text_elements: [] }]);
      turning = true;
      reply(request, { turn: { id: "turn" } });
      notice("turn/started", { turn: { id: "turn" } });
      notice("item/started", { turnId: "turn", item: { type: "agentMessage", id: "answer", text: "" } });
      process.stdout.write("not json\n");
      notice("future/event", {});
      const delta = Buffer.from(`${JSON.stringify({ method: "item/agentMessage/delta", params: {
        threadId: "thread", turnId: "turn", itemId: "answer", delta: "Reviewed ✓",
      } })}\n`);
      // Split a multibyte character across pipe writes; EOF may also leave an incomplete frame.
      const split = delta.indexOf(Buffer.from("✓")) + 1;
      process.stdout.write(delta.subarray(0, split));
      await new Promise(resolve => setTimeout(resolve, 10));
      process.stdout.write(delta.subarray(split));
      if (mode === "crash") {
        process.stdout.write('{"method":"turn/completed"');
        process.stderr.write("Synthetic provider exit\n");
        process.exitCode = 23;
        input.close();
        process.stdin.destroy();
      } else if (mode !== "interrupt") {
        notice("item/completed", { turnId: "turn", item: { type: "agentMessage", id: "answer", text: "Reviewed ✓" } });
        finish("completed");
      }
      break;
    }
    case "turn/interrupt":
      assert.equal(turning, true);
      assert.equal(request.params.threadId, "thread");
      assert.equal(request.params.turnId, "turn");
      reply(request, {});
      finish("interrupted");
      break;
    default:
      throw new Error(`Unexpected method: ${request.method}`);
  }
}
