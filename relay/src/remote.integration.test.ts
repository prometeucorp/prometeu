import { createServer, type Server } from "node:http";
import { readFileSync } from "node:fs";
import { URL as NodeURL } from "node:url";
import { createRequire } from "node:module";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createTestHarness } from "wrangler";

// Wrangler already uses ws; exercise its actual HTTP upgrade with an Authorization header.
const DesktopSocket = createRequire(import.meta.url)("ws") as new (url: string, options: { headers: Record<string, string> }) => WebSocket;
const CONTRACT = JSON.parse(readFileSync(new NodeURL("../../fixtures/cloud-api.json", import.meta.url), "utf8"));
const TOKEN = "a".repeat(43);
const CLIENT = "11111111-1111-4111-8111-111111111111";
let cloud: Server;
let harness: ReturnType<typeof createTestHarness>;
let origin: string;
let online: WebSocket;
let revoked = false;
let tickets = 0;

async function rpc(method: string, params: unknown = {}, token = TOKEN) {
  const response = await harness.getWorker().fetch("/mcp", { method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  return { status: response.status, body: await response.json().catch(() => null) as any };
}
const tool = (name: string, args: unknown = {}, token = TOKEN) => rpc("tools/call", { name, arguments: args }, token);

describe("personal remote MCP in the Worker runtime", () => {
  beforeAll(async () => {
    cloud = createServer((req, res) => {
      let body = "";
      req.on("data", chunk => { body += chunk; });
      req.on("end", () => {
        let value: unknown;
        if (req.url === "/api/remote/consume" && JSON.parse(body).ticket === "t".repeat(43)) {
          tickets++;
          value = { device: "device1", expires_at: Date.now() + 60_000 };
        } else if (req.url === "/api/remote/authorize" && !revoked && [TOKEN, "b".repeat(43), "r".repeat(43)].some(t => req.headers.authorization === `Bearer ${t}`)) {
          value = { device: "device1", client: req.headers.authorization === `Bearer ${"b".repeat(43)}` ? "other-client" : CLIENT, projects: ["project1"], scope: req.headers.authorization === `Bearer ${"r".repeat(43)}` ? "prometeu:read" : "prometeu:read prometeu:write", expires_at: Date.now() + 3600_000 };
        }
        res.writeHead(value ? 200 : 401, { "Content-Type": "application/json" });
        res.end(JSON.stringify(value || { error: "unauthorized" }));
      });
    });
    await new Promise<void>(resolve => cloud.listen(0, "127.0.0.1", resolve));
    const address = cloud.address() as { port: number };
    harness = createTestHarness({ workers: [{ configPath: "relay/wrangler.toml", vars: { CLOUD_URL: `http://127.0.0.1:${address.port}` } }] });
    const { url } = await harness.listen();
    origin = url.origin;
  }, 30_000);

  afterAll(async () => { online?.close(); await harness?.close(); await new Promise<void>(resolve => cloud?.close(() => resolve())); });

  it("advertises OAuth discovery and rejects missing authorization", async () => {
    const metadata = await harness.getWorker().fetch("/.well-known/oauth-protected-resource/mcp");
    expect(metadata.status).toBe(200);
    expect((await metadata.json() as any).authorization_servers).toHaveLength(1);
    const denied = await harness.getWorker().fetch("/mcp", { method: "POST", body: "{}" });
    expect(denied.status).toBe(401);
    expect(denied.headers.get("WWW-Authenticate")).toContain("resource_metadata=");
    expect((await tool("get_device")).body.result.structuredContent.online).toBe(false);
  });

  it("forwards only authenticated scope and returns the desktop tool result", async () => {
    online = new DesktopSocket(`${origin.replace(/^http/, "ws")}/remote/connect`, { headers: { Authorization: `Bearer ${"t".repeat(43)}` } });
    await new Promise<void>((resolve, reject) => { online.addEventListener("open", () => resolve()); online.addEventListener("error", reject); });
    expect(tickets).toBe(1);
    online.send(JSON.stringify({ type: "tools", tools: [
      { name: "delegate", inputSchema: { type: "object" } }, { name: "get_delegation", inputSchema: { type: "object" } },
    ] }));
    await expect.poll(async () => (await rpc("tools/list")).body.result.tools.length).toBe(4);
    const request = new Promise<any>(resolve => online.addEventListener("message", event => {
      const value = JSON.parse(String(event.data));
      if (value.type !== "call") return;
      online.send(JSON.stringify({ type: "result", id: value.id, result: { agent_id: "delegated1" } }));
      resolve(value);
    }));
    const response = await tool("delegate", { request_key: "stable-key", task: "Work" });
    expect(response.body.result.structuredContent.agent_id).toBe("delegated1");
    const forwarded = await request;
    expect(forwarded.client).toBe(CLIENT);
    expect(forwarded.projects).toEqual(["project1"]);
    expect(JSON.stringify(forwarded)).not.toContain(TOKEN);
    expect((await tool("delegate", {}, "r".repeat(43))).body.result.isError).toBe(true);
    expect((await tool("run_arbitrary_shell")).body.result.isError).toBe(true);
  });

  it("retains and deduplicates events across reconnect, scoped to the creating integration", async () => {
    const event = { ...CONTRACT.remote_event, key: "event1", client: CLIENT };
    online.send(JSON.stringify(event)); online.send(JSON.stringify(event));
    await expect.poll(async () => (await tool("read_events")).body.result.structuredContent.events.length).toBe(1);
    const read = (await tool("read_events")).body.result.structuredContent;
    expect(read.events[0].value.state).toBe("completed");
    expect((await tool("read_events", { after: read.next_cursor })).body.result.structuredContent.events).toEqual([]);
    expect((await tool("read_events", {}, "b".repeat(43))).body.result.structuredContent.events).toEqual([]);
    online.close();
    await expect.poll(async () => (await tool("get_device")).body.result.structuredContent.online).toBe(false);
    expect((await tool("delegate", { request_key: "stable-key" })).body.result.content[0].text).toContain("device_offline");
    expect((await tool("read_events")).body.result.structuredContent.events).toHaveLength(1);
    revoked = true;
    expect((await rpc("tools/list")).status).toBe(401);
  });

  it("pushes standard resource notifications and rechecks authorization when a stream reconnects", async () => {
    revoked = false;
    online = new DesktopSocket(`${origin.replace(/^http/, "ws")}/remote/connect`, { headers: { Authorization: `Bearer ${"t".repeat(43)}` } });
    await new Promise<void>((resolve, reject) => { online.addEventListener("open", () => resolve()); online.addEventListener("error", reject); });
    await rpc("resources/subscribe", { uri: "prometeu://events" });
    const response = await harness.getWorker().fetch("/mcp", { headers: { Authorization: `Bearer ${TOKEN}`, Accept: "text/event-stream" } });
    expect(response.headers.get("Content-Type")).toBe("text/event-stream");
    const reader = response.body!.getReader();
    await reader.read();
    online.send(JSON.stringify({ type: "event", key: "input-event", client: CLIENT, projects: ["project1"], value: { agent_id: "delegated1", execution_id: "run2", workspace_state: "ready", state: "running", outcome: null, input_required: true, request_ids: ["question1"] } }));
    const pushed = await reader.read();
    expect(new TextDecoder().decode(pushed.value)).toContain("notifications/resources/updated");
    await reader.cancel();
    const resource = await rpc("resources/read", { uri: "prometeu://events" });
    expect(resource.body.result.contents[0].text).toContain("input-event");
    revoked = true;
    const denied = await harness.getWorker().fetch("/mcp", { headers: { Authorization: `Bearer ${TOKEN}` } });
    expect(denied.status).toBe(401);
    online.close();
  });
});
