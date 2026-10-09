import { empty, reduce, type Effect } from "../relay/src/logic";
import { downForMember, encodeSnapshot, type Down, type Share, type Up } from "../relay/src/protocol";
import { TeamSecurity } from "./team-security";
import { TeamChannel } from "./team-channel";
import { verifyIdentity } from "./team-crypto";
import type { SocketLike } from "./team-transport";
import type { ShareRights } from "./types";

/** Browser demo uses real encrypted endpoints and the production relay reducer. Unless told otherwise, the demo owner
 *  lets the local member send messages and answer requests, as a teammate granted both rights (ADR 0090). */
export function simulatedSocket(url: string, sample: string, share: Share, online: () => boolean, rights?: ShareRights): SocketLike & { presence(): void } {
  const endpoint = new URL(url);
  const self = endpoint.searchParams.get("m") ?? "eu_mock";
  const name = endpoint.searchParams.get("n") ?? "You";
  const cfg = JSON.parse(localStorage.getItem("mock:team") ?? "null");
  const scope = JSON.stringify(cfg?.cloud ? ["organization", cfg.cloud.origin, cfg.team] : ["team", cfg?.relay || "wss://prometeu-relay.prometheus-capim.workers.dev", cfg?.team]);
  const state = empty();
  const challenge = crypto.randomUUID();
  let closed = false, seeded = false;
  let peer: TeamChannel;
  let queue = Promise.resolve();
  const emit = (frame: Down) => { if (!closed) socket.onmessage?.({ data: JSON.stringify(frame) }); };
  const enqueue = (work: () => Promise<void>) => {
    queue = queue.then(async () => { if (!closed) await work(); }).catch(() => { socket.onerror?.(); socket.close(); });
  };
  const run = (frame: Up, sender: string): Effect[] => reduce(state, { k: "text", sock: sender, frame, now: Date.now(), rand: crypto.randomUUID().slice(0, 8) });
  const peerSend = async (frame: Up) => deliver(run(await peer.outgoing(frame), "marcus"));
  async function deliver(effects: Effect[]): Promise<void> {
    for (const effect of effects) {
      if (effect.e === "sendBinary") {
        if (effect.sock === self && !closed) socket.onmessage?.({ data: effect.data.slice().buffer });
      } else if (effect.e === "send") {
        const frame = downForMember(effect.frame, effect.sock);
        if (effect.sock === self) {
          emit(frame.t === "welcome" ? { ...frame, e2ee: 1, challenge } : frame);
        } else if (effect.sock === "marcus" && peer) {
          const plain = await peer.incoming(frame.t === "welcome" ? { ...frame, e2ee: 1, challenge: "mock-marcus" } : frame);
          if (plain?.t === "watch") {
            for (const member of plain.added) {
              for (const bytes of await peer.outgoingBinary(encodeSnapshot(plain.tab, member, 1, new TextEncoder().encode(sample)), [member])) {
                await deliver(reduce(state, { k: "binary", sock: "marcus", data: bytes }));
              }
            }
          }
        }
      }
    }
  }
  const socket: SocketLike & { presence(): void } = {
    binaryType: "arraybuffer", onopen: null, onmessage: null, onclose: null, onerror: null,
    presence() { enqueue(async () => {
      if (online()) await deliver(reduce(state, { k: "open", sock: "marcus", member: "marcus", name: "Marcus Hale", now: Date.now() }));
      else await deliver(reduce(state, { k: "close", sock: "marcus", now: Date.now() }));
    }); },
    send(data) { enqueue(async () => {
      if (data === "ping") { socket.onmessage?.({ data: "pong" }); return; }
      if (typeof data !== "string") { await deliver(reduce(state, { k: "binary", sock: self, data: new Uint8Array(data as ArrayBuffer) })); return; }
      const frame: Up = JSON.parse(data);
      if (frame.t === "identity" && !(await verifyIdentity(frame.key, [self, challenge], frame.proof))) throw new Error("Invalid demo identity");
      await deliver(run(frame, self));
      if (frame.t === "identity" && !seeded) {
        seeded = true;
        peer.own(share, false, rights ?? { send: [self], control: [self] });
        await peerSend({ t: "share", share });
        await peerSend({ t: "note", ws: share.id, tab: "mt1", anchor: "w3.0",
          text: `Completing a todo now sets completed_at instead of deleting the row. @${name}, the remaining call is yours.`,
          mentions: [self], quote: "edit migrations/0007_todo_completed_at.sql · +11" });
      }
    }); },
    close() { if (!closed) { closed = true; socket.onclose?.(); } },
  };
  setTimeout(() => enqueue(async () => {
    const storageKey = `mock:peer-security:${scope}`;
    const security = await TeamSecurity.load("marcus", async () => JSON.parse(localStorage.getItem(storageKey) ?? "null"), async state => localStorage.setItem(storageKey, JSON.stringify(state)));
    peer = new TeamChannel(security, scope, "marcus");
    socket.onopen?.();
    await deliver(reduce(state, { k: "roster", members: [{ id: self, name }, { id: "marcus", name: "Marcus Hale" }, { id: "john", name: "John Okafor" }], now: Date.now() }));
    await deliver(reduce(state, { k: "open", sock: "marcus", member: "marcus", name: "Marcus Hale", now: Date.now() }));
    await deliver(run(await peer.identity("mock-marcus"), "marcus"));
    await deliver(reduce(state, { k: "open", sock: self, member: self, name, now: Date.now() }));
  }), 50);
  return socket;
}
