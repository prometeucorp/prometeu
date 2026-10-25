/// Relay entry point for team creation and routing connections to each team's Durable Object.

import { randomToken, sha256, TeamRoom, type Env } from "./room";
import { parseMembership, type CreatedTeam } from "./protocol";
import { smallJson } from "./http";
import { remote, type RemoteEnv } from "./remote";
export { RemoteDevice } from "./remote";

export { TeamRoom };

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

export default {
  async fetch(req: Request, env: Env & RemoteEnv): Promise<Response> {
    const response = await remote(req, env);
    if (response) return response;
    const url = new URL(req.url);

    const organization = /^\/organization\/([A-Za-z0-9_-]{8,64})$/.exec(url.pathname);
    if (organization) {
      // Separate namespace: legacy enrollment can never grant organization access.
      return env.TEAM.get(env.TEAM.idFromName(`organization:${organization[1]}`)).fetch(req);
    }

    if (url.pathname === "/teams") {
      if (req.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
      if (req.method !== "POST") return new Response("method", { status: 405, headers: CORS });
      // Cloudflare supplies this trusted header at the edge. Local Wrangler and mocks can omit it.
      const ip = req.headers.get("CF-Connecting-IP");
      if (ip) {
        const hour = Math.floor(Date.now() / 3_600_000);
        const limiter = env.TEAM.get(env.TEAM.idFromName(`create:${await sha256(ip)}`));
        const permit = await limiter.fetch(`https://team/permit-create?h=${hour}`, { method: "POST" });
        if (!permit.ok) return new Response("rate limit", { status: 429, headers: CORS });
      }
      const team = randomToken(12);
      const secret = randomToken(24);
      const stub = env.TEAM.get(env.TEAM.idFromName(team));
      const init = await stub.fetch("https://team/init", {
        method: "POST",
        body: JSON.stringify({ invite_hash: await sha256(secret) }),
      });
      if (!init.ok) return new Response("init", { status: 500, headers: CORS });
      const enrollment = await stub.fetch("https://team/enroll", {
        method: "POST",
        body: JSON.stringify({ secret }),
      });
      if (!enrollment.ok) return new Response("enroll", { status: 500, headers: CORS });
      const membership = parseMembership(await enrollment.json());
      if (!membership) return new Response("enroll", { status: 500, headers: CORS });
      return Response.json({ team, secret, ...membership } satisfies CreatedTeam, {
        headers: { ...CORS, "Cache-Control": "no-store" },
      });
    }

    const enroll = /^\/team\/([A-Za-z0-9_-]{8,64})\/enroll$/.exec(url.pathname);
    if (enroll) {
      if (req.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
      if (req.method !== "POST") return new Response("method", { status: 405, headers: CORS });
      const stub = env.TEAM.get(env.TEAM.idFromName(enroll[1]));
      // Finish reading at the public boundary before forwarding. A downstream early rejection can
      // otherwise leave fetch reading this request stream after its response has been sent.
      const parsed = await smallJson(req, 1024);
      const response = parsed.error ?? await stub.fetch("https://team/enroll", {
        method: "POST", headers: { "Content-Type": "application/json" }, body: parsed.raw,
      });
      const headers = new Headers(response.headers);
      for (const [key, value] of Object.entries(CORS)) headers.set(key, value);
      headers.set("Cache-Control", "no-store");
      return new Response(response.body, { status: response.status, headers });
    }

    const m = /^\/team\/([A-Za-z0-9_-]{8,64})$/.exec(url.pathname);
    if (m) {
      const stub = env.TEAM.get(env.TEAM.idFromName(m[1]));
      return stub.fetch(req);
    }

    return new Response("prometeu-relay", { status: 404 });
  },
} satisfies ExportedHandler<Env & RemoteEnv>;
