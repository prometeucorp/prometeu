import { DurableObject } from "cloudflare:workers";
import { smallJson } from "./http";

export type RemoteEnv = { REMOTE: DurableObjectNamespace<RemoteDevice>; CLOUD_URL?: string };
type Access = { device: string; client: string; projects: string[]; scope: string; expires_at: number };
type Ticket = { device: string; expires_at: number };
type Pending = { socket: WebSocket; finish: (value: unknown) => void; timer: ReturnType<typeof setTimeout> };
type Event = { id: number; key: string; client: string; projects: string[]; at: number; value: Record<string, unknown> };
const ID = /^[A-Za-z0-9_-]{1,128}$/;
const MAX = 1024 * 1024;
const READ = new Set(["list_projects", "list_models", "list_delegations", "get_delegation", "get_execution", "read_conversation", "list_files", "read_file", "read_workspace_script_log"]);
const WRITE = new Set(["delegate", "send_message", "interrupt", "run_workspace_script", "open_workspace_preview"]);
const json = (value: unknown, status = 200) => Response.json(value, { status, headers: { "Cache-Control": "no-store" } });
const result = (value: unknown, error = false) => ({ content: [{ type: "text", text: JSON.stringify(value) }], ...(error ? { isError: true } : { structuredContent: value }) });

function cloudOrigin(env: RemoteEnv): string {
  const url = new URL(env.CLOUD_URL || "https://app.prometeu.co");
  if (url.username || url.password || url.pathname !== "/" || url.search || url.hash ||
    !(url.protocol === "https:" || url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname))) throw Error("cloud_origin");
  return url.origin;
}

async function cloud(env: RemoteEnv, path: string, body: unknown, authorization?: string): Promise<unknown> {
  const response = await fetch(`${cloudOrigin(env)}/api/remote/${path}`, {
    method: "POST", headers: { "Content-Type": "application/json", ...(authorization ? { Authorization: authorization } : {}) },
    body: JSON.stringify(body), redirect: "manual", signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw Error("unauthorized");
  const parsed = await smallJson(response as unknown as Request, 32768);
  if (parsed.error) throw Error("invalid_response");
  return parsed.value;
}

function ticket(value: unknown): value is Ticket {
  const v = value as Ticket;
  return !!v && typeof v.device === "string" && ID.test(v.device) && Number.isSafeInteger(v.expires_at) && v.expires_at > Date.now() && v.expires_at <= Date.now() + 65_000;
}

function access(value: unknown): value is Access {
  const v = value as Access;
  return !!v && typeof v.device === "string" && ID.test(v.device) && typeof v.client === "string" && ID.test(v.client) && Number.isSafeInteger(v.expires_at) && v.expires_at > Date.now() &&
    Array.isArray(v.projects) && v.projects.length > 0 && v.projects.length <= 100 && v.projects.every(p => typeof p === "string" && ID.test(p)) &&
    typeof v.scope === "string" && v.scope.split(" ").every(s => ["prometeu:read", "prometeu:write"].includes(s));
}

function eventMetadata(value: unknown): value is Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  return Object.keys(v).sort().join(",") === "agent_id,execution_id,input_required,outcome,request_ids,state,workspace_state" &&
    typeof v.workspace_state === "string" && ["preparing", "failed", "ready", "unavailable"].includes(v.workspace_state) &&
    typeof v.agent_id === "string" && v.agent_id.length <= 128 && typeof v.execution_id === "string" && v.execution_id.length <= 128 &&
    typeof v.state === "string" && ["queued", "running", "completed", "stopped"].includes(v.state) &&
    [null, "ok", "error", "interrupted"].includes(v.outcome as string | null) && typeof v.input_required === "boolean" &&
    Array.isArray(v.request_ids) && v.request_ids.every(id => typeof id === "string" && id.length <= 128) && JSON.stringify(v).length <= 4096;
}

/** Public routes authenticate before choosing a device. No user-supplied routing identity is trusted. */
export async function remote(req: Request, env: RemoteEnv): Promise<Response | null> {
  const url = new URL(req.url);
  if (url.pathname === "/.well-known/oauth-protected-resource" || url.pathname === "/.well-known/oauth-protected-resource/mcp") {
    return json({ resource: `${url.origin}/mcp`, authorization_servers: [cloudOrigin(env)], scopes_supported: ["prometeu:read", "prometeu:write"], bearer_methods_supported: ["header"] });
  }
  if (!["/mcp", "/remote/connect"].includes(url.pathname)) return null;
  if (req.headers.has("Origin") && req.headers.get("Origin") !== url.origin && req.headers.get("Origin") !== cloudOrigin(env)) return new Response("origin", { status: 403 });
  if (url.pathname === "/remote/connect") {
    if (req.headers.get("Upgrade")?.toLowerCase() !== "websocket") return new Response("upgrade", { status: 426 });
    const credential = req.headers.get("Authorization")?.replace(/^Bearer /, "");
    if (!credential || !/^[A-Za-z0-9_-]{43}$/.test(credential)) return new Response("unauthorized", { status: 401 });
    try {
      const auth = await cloud(env, "consume", { ticket: credential });
      if (!ticket(auth)) throw Error("unauthorized");
      return env.REMOTE.get(env.REMOTE.idFromName(auth.device)).fetch(new Request("https://device/connect", {
        headers: { Upgrade: "websocket", "X-Access": JSON.stringify(auth) },
      }));
    } catch { return new Response("unauthorized", { status: 401 }); }
  }
  const authorization = req.headers.get("Authorization") || "";
  let auth: Access;
  try {
    const value = await cloud(env, "authorize", {}, authorization);
    if (!access(value)) throw Error("unauthorized");
    auth = value;
  } catch {
    return new Response("unauthorized", { status: 401, headers: {
      "WWW-Authenticate": `Bearer resource_metadata="${url.origin}/.well-known/oauth-protected-resource/mcp"`, "Cache-Control": "no-store",
    } });
  }
  if (req.method === "GET") return env.REMOTE.get(env.REMOTE.idFromName(auth.device)).fetch("https://device/events", { headers: { "X-Access": JSON.stringify(auth) } });
  if (req.method !== "POST") return new Response(null, { status: 405, headers: { Allow: "POST, GET" } });
  if (!req.headers.get("Content-Type")?.startsWith("application/json")) return new Response("content type", { status: 415 });
  const parsed = await smallJson(req, MAX);
  if (parsed.error) return parsed.error;
  return env.REMOTE.get(env.REMOTE.idFromName(auth.device)).fetch("https://device/mcp", {
    method: "POST", headers: { "X-Access": JSON.stringify(auth), "Content-Type": "application/json" }, body: parsed.raw,
  });
}

const EXTRA_TOOLS = [
  { name: "get_device", description: "Read device availability and last connection time. Offline does not mean a task completed.", inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "read_events", description: "Read durable task events after a cursor. Poll this tool to receive completion or input-required events. Store next_cursor. A gap requires reconciling with get_delegation when online.", inputSchema: { type: "object", properties: { after: { type: "integer", minimum: 0 } }, additionalProperties: false } },
];

/** One personal device, isolated from organization collaboration and its encrypted protocol. */
export class RemoteDevice extends DurableObject<RemoteEnv> {
  private pending = new Map<string, Pending>();
  private rates = new Map<string, { at: number; count: number }>();
  private listeners = new Map<ReadableStreamDefaultController<Uint8Array>, Access>();

  private socket(): WebSocket | undefined {
    return this.ctx.getWebSockets().find(ws => {
      const auth = ws.deserializeAttachment() as Ticket;
      return auth?.expires_at > Date.now() && ws.readyState === WebSocket.OPEN;
    });
  }

  async fetch(req: Request): Promise<Response> {
    const auth = JSON.parse(req.headers.get("X-Access") || "null") as Access;
    if (new URL(req.url).pathname === "/connect") {
      for (const ws of this.ctx.getWebSockets()) ws.close(1000, "replaced");
      this.finishPending("connection_lost");
      const pair = new WebSocketPair();
      this.ctx.acceptWebSocket(pair[1]);
      pair[1].serializeAttachment(auth);
      await this.ctx.storage.put("last_seen", Date.now());
      await this.ctx.storage.setAlarm(Date.now() + 60_000);
      pair[1].send(JSON.stringify({ type: "connected" }));
      return new Response(null, { status: 101, webSocket: pair[0] });
    }
    if (!access(auth)) return new Response("unauthorized", { status: 401 });
    if (new URL(req.url).pathname === "/events") {
      if (this.listeners.size >= 16) return new Response("stream limit", { status: 429 });
      let controller: ReadableStreamDefaultController<Uint8Array>;
      let timer: ReturnType<typeof setTimeout>;
      const stream = new ReadableStream<Uint8Array>({
        start: c => {
          controller = c;
          this.listeners.set(c, auth);
          c.enqueue(new TextEncoder().encode(": connected; reconnect within 30 seconds to reauthorize\n\n"));
          // A stream is a short lease, not a credential. Reconnect rechecks revocation in Rails.
          timer = setTimeout(() => { this.listeners.delete(c); try { c.close(); } catch { /* canceled */ } }, Math.min(30_000, auth.expires_at - Date.now()));
        },
        cancel: () => { clearTimeout(timer); this.listeners.delete(controller); },
      });
      return new Response(stream, { headers: { "Content-Type": "text/event-stream", "Cache-Control": "no-store" } });
    }
    for (const [client, rate] of this.rates) if (Date.now() - rate.at >= 60_000) this.rates.delete(client);
    const current = this.rates.get(auth.client);
    const rate = current && Date.now() - current.at < 60_000 ? current : { at: Date.now(), count: 0 };
    this.rates.set(auth.client, rate);
    if (++rate.count > 120) return new Response("rate limit", { status: 429 });
    const message = await req.json() as { jsonrpc?: string; id?: unknown; method?: string; params?: Record<string, unknown> };
    const id = message?.id ?? null;
    const error = (code: number, text: string) => json({ jsonrpc: "2.0", id, error: { code, message: text } });
    const ok = (value: unknown) => json({ jsonrpc: "2.0", id, result: value });
    if (!message || Array.isArray(message) || message.jsonrpc !== "2.0" || typeof message.method !== "string" ||
      (id !== null && typeof id !== "string" && typeof id !== "number")) return error(-32600, "Invalid Request");
    if (message.id === undefined) return new Response(null, { status: 202 });
    if (message.method === "initialize") return ok({ protocolVersion: "2025-06-18", capabilities: { tools: {}, resources: { subscribe: true } }, serverInfo: { name: "prometeu", version: "1.0.0" }, instructions: "Delegations run on the owner's Mac. Poll read_events for completion and input requests, or subscribe to prometeu://events. Approvals remain in Prometeu." });
    if (message.method === "ping") return ok({});
    const canRead = auth.scope.split(" ").includes("prometeu:read");
    if (message.method.startsWith("resources/") && !canRead) return error(-32000, "Insufficient scope");
    if (message.method === "resources/list") return ok({ resources: [{ uri: "prometeu://events", name: "Task events", mimeType: "application/json" }] });
    if (["resources/subscribe", "resources/unsubscribe", "resources/read"].includes(message.method)) {
      if (message.params?.uri !== "prometeu://events") return error(-32602, "Unknown resource");
      if (message.method === "resources/read") {
        const events = (await this.ctx.storage.get<Event[]>("events") || []).filter(e => e.at > Date.now() - 30 * 86400_000);
        return ok({ contents: [{ uri: "prometeu://events", mimeType: "application/json", text: JSON.stringify({ events: events.filter(e => e.client === auth.client && e.projects.every(p => auth.projects.includes(p))).slice(-100), next_cursor: await this.ctx.storage.get<number>("cursor") || 0 }) }] });
      }
      await this.ctx.storage.put(`subscription:${auth.client}`, message.method === "resources/subscribe");
      return ok({});
    }
    const tools = (await this.ctx.storage.get<Record<string, unknown>[]>("tools") || []).filter(t => this.allowed(auth, String(t.name)));
    if (message.method === "tools/list") return ok({ tools: [...EXTRA_TOOLS.filter(t => canRead || t.name === "get_device"), ...tools] });
    if (message.method !== "tools/call") return error(-32601, "Method not found");
    const name = message.params?.name;
    const args = message.params?.arguments ?? {};
    if (typeof name !== "string" || !args || typeof args !== "object" || Array.isArray(args)) return error(-32602, "Invalid params");
    if (name === "get_device") return ok(result({ online: !!this.socket(), last_seen_at: await this.ctx.storage.get("last_seen") ?? null }));
    if (name === "read_events") {
      if (!canRead) return ok(result({ error: "tool_not_allowed" }, true));
      const after = (args as { after?: number }).after ?? 0;
      if (!Number.isSafeInteger(after) || after < 0) return error(-32602, "Invalid cursor");
      const events = (await this.ctx.storage.get<Event[]>("events") || []).filter(e => e.at > Date.now() - 30 * 86400_000);
      const cursor = await this.ctx.storage.get<number>("cursor") || 0;
      const page = events.filter(e => e.id > after && e.client === auth.client && e.projects.every(p => auth.projects.includes(p))).slice(0, 100);
      return ok(result({ events: page, next_cursor: page.length === 100 ? page[99].id : cursor, gap: after < (events[0]?.id ?? cursor + 1) - 1 }));
    }
    if (!this.allowed(auth, name) || !tools.some(t => t.name === name)) return ok(result({ error: "tool_not_allowed" }, true));
    const ws = this.socket();
    if (!ws) return ok(result({ error: "device_offline", last_seen_at: await this.ctx.storage.get("last_seen") ?? null }, true));
    if (this.pending.size >= 16) return ok(result({ error: "busy" }, true));
    const call = crypto.randomUUID();
    const value = await new Promise(resolve => {
      const timer = setTimeout(() => { this.pending.delete(call); resolve(result({ error: "outcome_unknown", retry: "Reconcile using the original request_key; do not create a new key." }, true)); }, 25_000);
      this.pending.set(call, { socket: ws, finish: resolve, timer });
      try { ws.send(JSON.stringify({ type: "call", id: call, client: auth.client, projects: auth.projects, scope: auth.scope, name, arguments: args })); }
      catch { clearTimeout(timer); this.pending.delete(call); resolve(result({ error: "connection_lost" }, true)); }
    });
    return ok(value);
  }

  private allowed(auth: Access, name: string): boolean {
    return READ.has(name) ? auth.scope.split(" ").includes("prometeu:read") : WRITE.has(name) && auth.scope.split(" ").includes("prometeu:write");
  }

  async webSocketMessage(ws: WebSocket, raw: string | ArrayBuffer): Promise<void> {
    if (typeof raw !== "string" || new TextEncoder().encode(raw).length > MAX) { ws.close(1009, "frame size"); return; }
    const old = ws.deserializeAttachment() as Ticket;
    if (!old || old.expires_at <= Date.now()) { ws.close(1008, "expired"); return; }
    let value: Record<string, unknown>;
    try { value = JSON.parse(raw); } catch { ws.close(1008, "invalid frame"); return; }
    if (!value || Array.isArray(value)) { ws.close(1008, "invalid frame"); return; }
    if (value.type === "renew") {
      try {
        const auth = await cloud(this.env, "consume", { ticket: value.ticket });
        if (!ticket(auth) || auth.device !== old.device) throw Error("unauthorized");
        ws.serializeAttachment(auth);
        await this.ctx.storage.put("last_seen", Date.now());
        await this.ctx.storage.setAlarm(Date.now() + 60_000);
        ws.send(JSON.stringify({ type: "renewed" }));
      } catch { ws.close(1008, "unauthorized"); }
    } else if (value.type === "tools" && Array.isArray(value.tools) && value.tools.length <= 30) {
      await this.ctx.storage.put("tools", value.tools.filter(t => t && typeof t === "object" && (READ.has(t.name) || WRITE.has(t.name))));
    } else if (value.type === "result" && typeof value.id === "string") {
      const pending = this.pending.get(value.id);
      if (pending && pending.socket === ws) {
        this.pending.delete(value.id); clearTimeout(pending.timer);
        pending.finish(typeof value.error === "string" ? result({ error: value.error }, true) : result(value.result));
      }
    } else if (value.type === "event" && typeof value.key === "string" && value.key.length <= 128 && typeof value.client === "string" && ID.test(value.client) &&
      Array.isArray(value.projects) && value.projects.length <= 100 && value.projects.every(p => typeof p === "string" && ID.test(p)) &&
      eventMetadata(value.value)) {
      await this.ctx.storage.transaction(async storage => {
        const events = await storage.get<Event[]>("events") || [];
        if (!events.some(e => e.key === value.key)) {
          const cursor = (await storage.get<number>("cursor") || 0) + 1;
          events.push({ id: cursor, key: value.key as string, client: value.client as string, projects: value.projects as string[], at: Date.now(), value: value.value as Record<string, unknown> });
          await storage.put("events", events.filter(e => e.at > Date.now() - 30 * 86400_000).slice(-1000));
          await storage.put("cursor", cursor);
        }
      });
      ws.send(JSON.stringify({ type: "ack", key: value.key }));
      if (await this.ctx.storage.get(`subscription:${value.client}`)) {
        for (const [listener, auth] of this.listeners) {
          if (auth.client !== value.client || !(value.projects as string[]).every(p => auth.projects.includes(p))) continue;
          try { listener.enqueue(new TextEncoder().encode(`event: message\ndata: ${JSON.stringify({ jsonrpc: "2.0", method: "notifications/resources/updated", params: { uri: "prometeu://events" } })}\n\n`)); }
          catch { this.listeners.delete(listener); }
        }
      }
    }
  }

  private finishPending(error: string, socket?: WebSocket) {
    for (const [id, p] of this.pending) {
      if (socket && p.socket !== socket) continue;
      clearTimeout(p.timer); p.finish(result({ error, outcome: "unknown" }, true)); this.pending.delete(id);
    }
  }

  webSocketClose(ws: WebSocket) { this.finishPending("connection_lost", ws); }
  webSocketError(ws: WebSocket) { this.finishPending("connection_lost", ws); }
  async alarm() {
    const events = (await this.ctx.storage.get<Event[]>("events") || []).filter(e => e.at > Date.now() - 30 * 86400_000);
    await this.ctx.storage.put("events", events);
    for (const ws of this.ctx.getWebSockets()) {
      const auth = ws.deserializeAttachment() as Ticket;
      if (!auth || auth.expires_at <= Date.now()) ws.close(1008, "expired");
    }
    if (this.socket()) await this.ctx.storage.setAlarm(Date.now() + 60_000);
    else {
      this.finishPending("connection_lost");
      if (events.length) await this.ctx.storage.setAlarm(Date.now() + 86400_000);
    }
  }
}
