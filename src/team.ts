import { listen } from "@tauri-apps/api/event";
import {
  DOWN_FRAME_MAX,
  PROTO,
  SNAPSHOT,
  decodeBinary,
  encodeLive,
  encodeSnapshot,
  formatInvite,
  normalizeName,
  parseDown,
  parseInvite,
  parseMembership,
  type Down,
  type Inbox,
  type Member,
  type Note,
  type Segment,
  type Share,
  type Shared,
  type Up,
  type Watching,
} from "../relay/src/protocol";
import { t } from "./i18n";
import { AttachLifecycle } from "./attach-lifecycle";
import { invoke } from "./ipc";
import { Mirror } from "./mirror";
import { remoteControl } from "./team-control";
import {
  defaultTransport,
  wsUrl as transportWsUrl,
  type SocketLike,
  type Transport,
} from "./team-transport";
import type { Board, Workspace } from "./types";

export type { SocketLike, Transport } from "./team-transport";

/// O time: a conexão com o relay e o que ele conta — quem está online, o que
/// está compartilhado, a caixa de comentários. Vive aqui, no front, e não no Rust,
/// porque tudo de que o compartilhamento precisa já passa por aqui: as linhas
/// de toda conversa chegam pelo evento `chat`, e falar numa conversa é um
/// `invoke`. O back só guarda o `team.json`.
///
/// Uma conexão por app, sempre de pé enquanto houver time: cai, volta sozinha
/// com espera crescente; o `welcome` que o relay manda ao conectar é a verdade
/// e refaz o estado inteiro.
///
/// Dois papéis, no mesmo módulo, porque o mesmo app faz os dois ao mesmo
/// tempo: **dono** do que compartilhou (anuncia o workspace, repassa as linhas
/// das abas que alguém está olhando, recebe as falas) e **colega** do que os
/// outros compartilharam (workspaces remotos no quadro, uma aba aberta por
/// vez, o espelho das linhas de cada uma).
///
/// O relay não sabe o que carrega: para ele são bytes numerados, e eram bytes
/// de terminal antes de serem linhas de JSON. O formato dos frames é o mesmo.

export type TeamConfig = {
  /// URL do relay quando não é a padrão do app.
  relay: string | null;
  team: string;
  secret: string;
  /// Identidade e prova individuais, emitidas pelo relay na matrícula.
  member: string;
  credential: string;
  name: string;
};

export type Phase = "off" | "connecting" | "online";

/// O relay que `npm run relay:deploy` publicou. É o padrão do app: quem não
/// informar outro em Configurações (ou `VITE_RELAY` no dev) entra por ele.
const RELAY = "wss://prometeu-relay.prometheus-capim.workers.dev";

/// O tamanho de cada parte da conversa que vai a quem acabou de abrir uma
/// aba. O relay limita a mensagem a 1 MB; a conversa inteira (até 4 MB, o que
/// o back guarda) vai em quantas partes precisar, cortadas em linha inteira —
/// meia linha de JSON não é nada.
const SNAPSHOT_PART = 512 * 1024;
/// Quanto a saída espera antes de sair num frame só. O relay cobra por
/// mensagem recebida; a tela do colega não distingue 40 ms.
const COALESCE = 40;
const FRAME_MAX = 32 * 1024;
/// Quanto o colega espera pela conversa ao abrir uma aba. Passou disso, abre
/// com o espelho que tiver — o dono sumiu no meio.
const SNAPSHOT_WAIT = 10_000;

const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env;

/* ---------- o que fala com o mundo ---------- */

const enc = new TextEncoder();

let transport: Transport = defaultTransport();

export function useTransport(next: Transport) {
  transport = next;
}

/// Para onde vão as linhas da conversa remota que está na tela.
export type GuestSink = {
  live: (tab: string, bytes: Uint8Array) => void;
  /// As linhas chegaram de novo (o dono voltou, ou a conexão caiu e voltou): a
  /// tela renasce delas.
  reset: (tab: string, bytes: Uint8Array) => void;
};

let guest: GuestSink = { live: () => {}, reset: () => {} };
export const setSink = (sink: GuestSink) => void (guest = sink);

/* ---------- estado ---------- */

let cfg: TeamConfig | null = null;
let defaultName = "";
/// Relay digitado antes de haver time — vai para o `team.json` quando houver.
let relayDraft = "";
let phase: Phase = "off";
let sock: SocketLike | null = null;
let you: string | null = null;
let members: Member[] = [];
let shares = new Map<string, Shared>();
let inbox: Inbox[] = [];
let comments = false;
/// Os comentários de cada workspace, como o relay os contou. Só o que já foi pedido
/// (`notes`) está aqui; o resto chega quando alguém abre o painel.
const notes = new Map<string, Note[]>();
let attempt = 0;
let retry = 0;
let pinger = 0;

const listeners = new Set<() => void>();
export const onChange = (cb: () => void) => {
  listeners.add(cb);
  return () => void listeners.delete(cb);
};
const changed = () => listeners.forEach((cb) => cb());

let fail: ((text: string) => void) | null = null;
export const onError = (cb: (text: string) => void) => void (fail = cb);

export type TeamStatus = {
  config: TeamConfig | null;
  phase: Phase;
  you: string | null;
  members: Member[];
  defaultName: string;
  /// O que está escrito como relay (vazio é "o padrão"), o padrão, e o que
  /// vale de fato.
  relay: string;
  relayDefault: string;
  relayEffective: string;
};

export const status = (): TeamStatus => ({
  config: cfg,
  phase,
  you,
  members,
  defaultName,
  relay: cfg ? (cfg.relay ?? "") : relayDraft,
  relayDefault: env?.VITE_RELAY || RELAY,
  relayEffective: relayOf(cfg),
});

export const nameOf = (member: string) => members.find((m) => m.id === member)?.name ?? member.slice(0, 8);
export const invite = () => (cfg ? formatInvite(cfg.team, cfg.secret) : null);
export const inboxItems = () => inbox;
export const supportsThreads = () => comments;

/* ---------- ciclo de vida ---------- */

export async function init() {
  try {
    const file = await invoke<{ config: unknown; default_name: string }>("team_config");
    defaultName = file.default_name;
    const stored = storedConfig(file.config);
    if (stored) relayDraft = stored.relay ?? "";
    if (stored?.credential) {
      cfg = { ...stored, credential: stored.credential };
    } else if (stored) {
      // v2 usava o segredo compartilhado como identidade. Tenta trocar o
      // convite por uma matrícula v3 sem apagar o arquivo antigo se o relay
      // ainda não tiver sido recriado.
      const base = relayOf(stored);
      try {
        if (!base) throw new Error(t("err.team.noRelay"));
        const membership = await transport.enroll(base, stored.team, stored.secret);
        cfg = { ...stored, ...membership };
        await invoke("team_config_set", { config: cfg });
      } catch {
        cfg = null;
        fail?.(t("err.team.legacy"));
      }
    }
  } catch {
    // Sem back (ou back velho) não há time; a tela segue de pé.
  }
  // Toda linha de toda conversa passa aqui; o que é de aba que alguém está
  // olhando vai para o relay.
  listen<[string, string, number]>("chat", ({ payload: [key, line, seq] }) => output(key, line, seq));
  if (cfg) connect();
}

/// O relay que vale para uma configuração: o dela, o do ambiente de dev, o
/// padrão do app — e, no mock do navegador, um endereço qualquer, porque lá
/// não há relay e o socket é fingido.
type StoredTeamConfig = Omit<TeamConfig, "credential"> & { credential?: string };

function storedConfig(value: unknown): StoredTeamConfig | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;
  if (raw.relay !== null && typeof raw.relay !== "string") return null;
  if (typeof raw.team !== "string" || typeof raw.secret !== "string" || typeof raw.member !== "string") return null;
  const name = normalizeName(raw.name);
  if (!name) return null;
  if (!parseInvite(formatInvite(raw.team, raw.secret))) return null;
  if (!parseMembership({ member: raw.member, credential: raw.credential ?? "" }) && raw.credential !== undefined) return null;
  return {
    relay: raw.relay,
    team: raw.team,
    secret: raw.secret,
    member: raw.member,
    credential: raw.credential as string | undefined,
    name,
  };
}

const relayOf = (c: Pick<TeamConfig, "relay"> | StoredTeamConfig | null) =>
  (c ? c.relay || "" : relayDraft) || env?.VITE_RELAY || RELAY || (transport.needsRelay ? "" : "ws://mock");

/// `https://x` vira `wss://x`, `http://x` vira `ws://x`; `ws(s)://` fica.
const wsUrl = (relay: string) => transportWsUrl(relay, transport.needsRelay);

function connect() {
  if (!cfg) return;
  const base = relayOf(cfg);
  if (!base) {
    phase = "off";
    changed();
    return;
  }
  const c = cfg;
  let url: string;
  try {
    const endpoint = new URL(`${wsUrl(base)}/team/${encodeURIComponent(c.team)}`);
    endpoint.searchParams.set("c", c.credential);
    endpoint.searchParams.set("m", c.member);
    endpoint.searchParams.set("n", c.name);
    endpoint.searchParams.set("p", String(PROTO));
    url = endpoint.toString();
  } catch (e) {
    phase = "off";
    fail?.(String(e));
    changed();
    return;
  }
  phase = "connecting";
  changed();
  let s: SocketLike;
  try {
    s = transport.socket(url);
  } catch (error) {
    phase = "off";
    fail?.(t("err.team.relay", { cause: String(error) }));
    changed();
    return;
  }
  s.binaryType = "arraybuffer";
  sock = s;
  s.onopen = () => {
    attempt = 0;
    // O edge derruba socket parado; o relay responde sem acordar.
    pinger = setInterval(() => s.send("ping"), 30_000);
  };
  s.onmessage = (ev) => {
    if (typeof ev.data === "string") {
      if (ev.data === "pong") return;
      // O relay é configurável. Mesmo um endpoint hostil não pode entregar um
      // JSON sem teto e obrigar a webview a materializá-lo inteiro em objetos.
      if (ev.data.length > DOWN_FRAME_MAX || enc.encode(ev.data).byteLength > DOWN_FRAME_MAX) {
        s.close();
        return;
      }
      let raw: unknown;
      try {
        raw = JSON.parse(ev.data);
      } catch {
        return;
      }
      const frame = parseDown(raw);
      if (!frame) return;
      handle(frame);
    } else if (ev.data instanceof ArrayBuffer) {
      binary(ev.data);
    }
  };
  s.onclose = () => {
    if (sock !== s) return;
    sock = null;
    attachLife.completeCurrent();
    clearInterval(pinger);
    members = members.map((m) => ({ ...m, online: false }));
    for (const sh of shares.values()) sh.online = false;
    announced.clear();
    watchers.clear();
    phase = cfg ? "connecting" : "off";
    changed();
    if (cfg) retry = setTimeout(connect, backoff());
  };
  s.onerror = () => {
    // O `close` vem logo atrás, e é ele que remarca.
  };
}

/// 1 s, 2 s, 4 s… até 30 s, com um pouco de acaso para dois apps do mesmo
/// time não baterem no relay no mesmo instante.
function backoff(): number {
  const base = Math.min(30_000, 1000 * 2 ** attempt++);
  return Math.round(base * (0.8 + Math.random() * 0.4));
}

function disconnect() {
  clearTimeout(retry);
  clearInterval(pinger);
  attachLife.completeCurrent();
  const s = sock;
  sock = null;
  s?.close();
  phase = "off";
}

function send(frame: Up): boolean {
  if (!sock || phase !== "online") return false;
  sock.send(JSON.stringify(frame));
  return true;
}

function sendBinary(data: Uint8Array): boolean {
  if (!sock || phase !== "online") return false;
  sock.send(data);
  return true;
}

/* ---------- o que chega ---------- */

/// Erro do relay que a pessoa não pediu e não resolve: o workspace ou a aba
/// não estão mais lá. O app pergunta por eles sozinho — ao reconectar, ao
/// desenhar os comentários — e cada pergunta dessas virava um aviso vermelho no
/// topo sobre algo que ninguém fez.
const QUIET = new Set(["noShare", "noTab"]);

function handle(frame: Down) {
  switch (frame.t) {
    case "welcome":
      you = frame.you;
      members = frame.members;
      shares = new Map(frame.shares.map((s) => [s.id, s]));
      inbox = frame.inbox;
      comments = frame.comments === 1;
      phase = "online";
      // A verdade veio; o que é meu vai de novo, e quem já olhava minhas abas
      // ganha a rolagem inteira — o que saiu enquanto eu estava fora não
      // chegou a ninguém. Vai antes de qualquer pedido: o relay ainda não
      // sabe o que é meu, e perguntar sobre um workspace que ele não tem é
      // ganhar um erro em vez de uma resposta.
      announced.clear();
      if (lastBoard) boardChanged(lastBoard);
      // O que eu tinha em cache pode ter envelhecido enquanto eu estava fora.
      // Só se repete o pedido do que ainda existe daqui: o cache guarda todo
      // workspace pelo qual já se perguntou, inclusive o que parou de ser
      // compartilhado meses atrás, e pedir os comentários dele de novo a cada
      // reconexão era o que fazia o erro voltar sozinho.
      const asked = [...notes.keys()].filter(known);
      notes.clear();
      for (const ws of asked) send({ t: "notes", ws });
      rewatch(frame.watching);
      if (attached) send({ t: "attach", ws: attached.ws, tab: attached.tab });
      break;
    case "presence":
      members = frame.members;
      break;
    case "share":
      shares.set(frame.share.id, frame.share);
      break;
    case "unshare":
      shares.delete(frame.ws);
      notes.delete(frame.ws);
      inbox = inbox.filter((item) => item.ws !== frame.ws);
      for (const [id, r] of remoteIds) if (r.ws === frame.ws) remoteIds.delete(id);
      if (attached?.ws === frame.ws) {
        attachLife.cancel();
        attached = null;
      }
      break;
    case "watch":
      watched(frame.tab, frame.members, frame.added);
      break;
    case "write":
      typed(frame.ws, frame.tab, frame.data, frame.from);
      return;
    // Tamanho de terminal, de quando a conversa era um. Nada a fazer.
    case "size":
      return;
    case "inbox":
      inbox = frame.items;
      break;
    case "note": {
      const list = notes.get(frame.note.ws) ?? [];
      // Criação e resposta chegam com id novo; resolver atualiza a raiz com o
      // mesmo id. Um caminho cobre os dois e mantém a ordem do histórico.
      const at = list.findIndex((n) => n.id === frame.note.id);
      if (at === -1) list.push(frame.note);
      else list[at] = frame.note;
      list.sort((a, b) => a.ts - b.ts);
      notes.set(frame.note.ws, list);
      break;
    }
    case "notes":
      notes.set(frame.ws, frame.items);
      break;
    case "error": {
      // O erro do relay não diz a que pedido responde. Os que só contam que
      // algo saiu de lá respondem, quase sempre, a pedido que o app fez
      // sozinho — e a barra de cima é a resposta ao que a pessoa acabou de
      // fazer, não um lugar onde o app conversa consigo mesmo. Quem conserta
      // a tela é o `unshare`, que vem por conta própria.
      if (QUIET.has(frame.code)) return;
      // Relay mais novo que o app pode mandar um código que este catálogo não
      // tem; dizer a chave crua é pior que dizer que algo não passou.
      const key = `err.team.${frame.code}` as Parameters<typeof t>[0];
      const text = t(key);
      fail?.(text === key ? t("err.team.bad") : text);
      return;
    }
    default:
      return;
  }
  changed();
}

/* ---------- ações do time ---------- */

async function adopt(next: TeamConfig) {
  await invoke("team_config_set", { config: next });
  disconnect();
  reset();
  cfg = next;
  attempt = 0;
  connect();
  changed();
}

function reset() {
  attachLife.cancel();
  you = null;
  members = [];
  shares = new Map();
  inbox = [];
  comments = false;
  announced.clear();
  watchers.clear();
  queue.clear();
  attached = null;
  mirror.clear();
  remoteIds.clear();
  notes.clear();
}

const cleanName = (name: string) => {
  const n = normalizeName(name);
  if (!n) throw t("err.team.name");
  return n;
};

export async function create(name: string) {
  const n = cleanName(name);
  const base = relayOf(null);
  if (!base) throw t("err.team.noRelay");
  const { team, secret, member, credential } = await transport.create(base);
  await adopt({ relay: relayDraft || null, team, secret, member, credential, name: n });
}

export async function join(code: string, name: string) {
  const n = cleanName(name);
  const parsed = parseInvite(code);
  if (!parsed) throw t("err.team.badCode");
  const base = relayOf(null);
  if (!base) throw t("err.team.noRelay");
  const membership = await transport.enroll(base, parsed.team, parsed.secret);
  await adopt({ relay: relayDraft || null, team: parsed.team, secret: parsed.secret, ...membership, name: n });
}

export async function leave() {
  disconnect();
  reset();
  cfg = null;
  await invoke("team_config_set", { config: null });
  changed();
}

export async function setName(name: string) {
  if (!cfg) return;
  const n = cleanName(name);
  cfg = { ...cfg, name: n };
  await invoke("team_config_set", { config: cfg });
  send({ t: "me", name: n });
  changed();
}

export async function setRelay(url: string) {
  const u = url.trim().replace(/\/+$/, "");
  if (u) wsUrl(u);
  relayDraft = u;
  if (cfg) {
    cfg = { ...cfg, relay: u || null };
    await invoke("team_config_set", { config: cfg });
    disconnect();
    attempt = 0;
    connect();
  }
  changed();
}

/* ---------- dono: anunciar e repassar ---------- */

/// O último quadro que o app viu — é dele que sai o anúncio, e é ele que se
/// reanuncia quando a conexão volta.
let lastBoard: Board | null = null;
/// O que anunciei de cada workspace meu, serializado: só vai de novo se mudou.
const announced = new Map<string, string>();
/// Quem está olhando cada aba minha, pelo que o relay contou.
const watchers = new Map<string, string[]>();
/// Saída esperando para sair num frame só, por aba.
const queue = new Map<string, Segment[]>();
let queued = 0;
let flushTimer = 0;
/// Abas com snapshot a caminho: a saída delas espera, para nenhum pedaço
/// sair na frente do snapshot que já o contém.
const holding = new Map<string, number>();

/// O tamanho de terminal que o protocolo ainda pede por aba. A conversa não
/// tem mais um, e o relay não olha o valor: vai um qualquer.
const NO_SIZE: [number, number] = [80, 24];

function toShare(w: Workspace): Share {
  return {
    id: w.id,
    title: w.title,
    repo_name: w.repo_name,
    branch: w.branch,
    stage: w.stage,
    issue: w.issue ? { identifier: w.issue.identifier, title: w.issue.title, url: w.issue.url } : null,
    active: w.active,
    tabs: w.tabs.map((tab) => ({ id: tab.id, title: tab.title, status: tab.status, note: tab.note, tokens: tab.tokens })),
    sizes: Object.fromEntries(w.tabs.map((tab) => [tab.id, NO_SIZE])),
    audience: w.audience,
  };
}

/// O quadro mudou: o que é meu e está marcado vai ao relay se mudou; o que
/// deixou de estar (arquivado, devolvido, tirado do quadro) sai de lá.
export function boardChanged(board: Board) {
  lastBoard = board;
  if (!cfg) return;
  const seen = new Set<string>();
  for (const w of board.workspaces) {
    if (!w.shared || w.remote) continue;
    if (w.archived || w.cleaned) {
      void invoke("set_shared", { id: w.id, shared: false });
      continue;
    }
    seen.add(w.id);
    const share = toShare(w);
    const json = JSON.stringify(share);
    if (announced.get(w.id) === json) continue;
    if (send({ t: "share", share })) announced.set(w.id, json);
  }
  for (const id of [...announced.keys()]) {
    if (seen.has(id)) continue;
    announced.delete(id);
    send({ t: "unshare", ws: id });
    for (const tab of [...watchers.keys()]) if (!tabOwnedBy(tab, seen)) watchers.delete(tab);
  }
}

const tabOwnedBy = (tab: string, ids: Set<string>) =>
  !!lastBoard?.workspaces.some((w) => ids.has(w.id) && w.tabs.some((t) => t.id === tab));

/// A aba pertence a um workspace meu, anunciado agora.
const mine = (tab: string) => tabOwnedBy(tab, new Set(announced.keys()));

/// Compartilha com o time inteiro (`null`), com alguns (ids de membros), ou
/// para (`false`). Lista vazia é parar: não há "compartilhado com ninguém".
export async function share(id: string, audience: string[] | null | false) {
  const on = audience !== false && (audience === null || audience.length > 0);
  await invoke("set_shared", { id, shared: on, audience: on ? audience : null });
}

export const watchersOf = (tab: string): string[] => (watchers.get(tab) ?? []).map(nameOf);

function watched(tab: string, who: string[], added: string[]) {
  if (who.length) watchers.set(tab, who);
  else watchers.delete(tab);
  for (const member of added) void snapshot(tab, member);
}

/// O dono voltou: quem já olhava ganha a rolagem inteira de novo.
function rewatch(watching: Watching) {
  watchers.clear();
  for (const tabs of Object.values(watching)) {
    for (const [tab, who] of Object.entries(tabs)) {
      if (!who.length) continue;
      watchers.set(tab, who);
      for (const member of who) void snapshot(tab, member);
    }
  }
}

/// A conversa inteira de uma aba minha para um colega, em partes. Enquanto
/// ela não sai toda, as linhas ao vivo da aba ficam presas: uma que saísse na
/// frente e não estivesse na conversa seria ignorada do outro lado — e perdida.
async function snapshot(tab: string, member: string) {
  holding.set(tab, (holding.get(tab) ?? 0) + 1);
  try {
    const s = await invoke<{ text: string; seq: number }>("chat_snapshot", { session: tab });
    const parts = split(s.text);
    parts.forEach((part, i) => sendBinary(encodeSnapshot(tab, member, s.seq, enc.encode(part), i < parts.length - 1)));
  } catch {
    // Sessão que já não existe: o colega abre com o que tiver.
  } finally {
    const left = (holding.get(tab) ?? 1) - 1;
    if (left > 0) holding.set(tab, left);
    else holding.delete(tab);
    flush();
  }
}

/// A conversa em partes do tamanho de uma mensagem do relay, cortadas em linha
/// inteira. Sempre ao menos uma — vazia, se a conversa ainda não falou.
function split(text: string): string[] {
  const out: string[] = [];
  let rest = text;
  while (rest.length > SNAPSHOT_PART) {
    const cut = rest.lastIndexOf("\n", SNAPSHOT_PART);
    const at = cut === -1 ? SNAPSHOT_PART : cut + 1;
    out.push(rest.slice(0, at));
    rest = rest.slice(at);
  }
  out.push(rest);
  return out;
}

/// Uma linha de alguma conversa. Só interessa se alguém está olhando a aba —
/// o resto do tempo isto custa uma busca num mapa vazio.
function output(key: string, line: string, seq: number) {
  if (!watchers.has(key)) return;
  const list = queue.get(key) ?? [];
  const bytes = enc.encode(line + "\n");
  list.push({ seq, bytes });
  queue.set(key, list);
  queued += bytes.length;
  if (queued >= FRAME_MAX) flush();
  else if (!flushTimer) flushTimer = setTimeout(flush, COALESCE);
}

function flush() {
  clearTimeout(flushTimer);
  flushTimer = 0;
  for (const [tab, segments] of queue) {
    if (holding.has(tab)) continue;
    queue.delete(tab);
    queued -= segments.reduce((n, s) => n + s.bytes.length, 0);
    if (!watchers.has(tab)) continue;
    sendBinary(encodeLive(tab, segments));
  }
  if (queue.size && !flushTimer) flushTimer = setTimeout(flush, COALESCE);
}

/// Um colega falou numa aba minha — ou respondeu a um card dela: a resposta
/// vem como a linha de controle inteira, em JSON (ver `chat.ts`). Só vale para
/// aba de workspace que eu anunciei: o relay já filtra, mas o que chega vai
/// para um processo de verdade.
function typed(ws: string, tab: string, data: string, from: string) {
  if (!announced.has(ws) || !mine(tab)) return;
  const parsed = remoteControl(data);
  if (parsed.recognized) {
    if (parsed.frame) void invoke("chat_control_remote", { session: tab, frame: parsed.frame }).catch(() => {});
    return;
  }
  const text = t("team.remotePrompt", { name: nameOf(from), text: data });
  void invoke("chat_send", { session: tab, text }).catch(() => {});
}

/* ---------- colega: o que os outros compartilharam ---------- */

/// O id de um workspace de colega na tela deste app. Prefixado porque o
/// quadro passa a ter os dois, e um id que colidisse com um workspace daqui
/// faria a tela desenhar um e falar do outro — e mandar ao back um id que não
/// é dele. O que o relay conhece fica no mapa.
const PREFIX = "@time:";
const remoteIds = new Map<string, { ws: string; owner: string }>();
const remoteId = (owner: string, ws: string) => `${PREFIX}${owner}/${ws}`;

/// A aba de um colega que está na tela, se alguma.
let attached: { ws: string; tab: string } | null = null;
/// As linhas de cada aba remota que já abri: as que o dono mandou mais o que
/// veio ao vivo. É daqui que a tela renasce ao voltar para a aba. A regra de
/// juntar as duas está em `mirror.ts`, testada sem rede nem tela.
const mirror = new Map<string, Mirror>();
const attachLife = new AttachLifecycle();

/// Workspaces dos colegas, como o quadro os desenha. Não são do Rust: só
/// existem na tela, e o que os distingue é `remote`.
export function remotes(): Workspace[] {
  const out: Workspace[] = [];
  for (const s of shares.values()) {
    if (s.owner === you) continue;
    const id = remoteId(s.owner, s.id);
    remoteIds.set(id, { ws: s.id, owner: s.owner });
    out.push({
      id,
      title: s.title,
      project: "@time",
      repo: "",
      repo_name: s.repo_name,
      branch: s.branch,
      worktree: "",
      repos: [],
      stage: s.stage,
      archived: false,
      pinned: false,
      unread: false,
      // O protocolo do relay não conta com que agente o colega trabalha, e o
      // card remoto não mostra modelo: nada aqui é decidido por isto.
      agent: "claude",
      model: "",
      effort: "",
      mcp: null,
      plugins: null,
      port: null,
      issue: s.issue ? { id: "", identifier: s.issue.identifier, title: s.issue.title, url: s.issue.url } : null,
      cleaned: false,
      shared: false,
      audience: null,
      // O colega só anuncia o que já montou: nada aqui nasce montando.
      preparing: false,
      failed: null,
      remote: { owner: s.owner, online: s.online },
      tabs: s.tabs.map((tab) => ({ ...tab })),
      active: s.active,
    });
  }
  return out;
}

/// Este id de workspace é de um colega? Pelo prefixo, e não por busca: a
/// resposta não pode mudar porque um share chegou ou saiu no meio.
export const isRemote = (id: string) => id.startsWith(PREFIX);

/// Abrir a aba de um colega: pede ao relay, espera as linhas chegarem e
/// devolve o que a tela desenha. Uma aba por vez — abrir outra solta a
/// anterior.
export async function attach(id: string, tab: string): Promise<{ bytes: Uint8Array } | null> {
  const found = remoteIds.get(id);
  const s = found && shares.get(found.ws);
  if (!s) throw t("err.team.noShare");
  attached = { ws: s.id, tab };
  // Também invalida uma espera anterior quando o novo destino já está
  // offline. Sem isto a Promise antiga ainda podia acordar dez segundos
  // depois de a tela ter mudado de aba.
  if (!s.online) attachLife.cancel();
  if (s.online) {
    const ticket = attachLife.start(s.id, tab, SNAPSHOT_WAIT);
    if (!send({ t: "attach", ws: s.id, tab })) {
      attachLife.cancel();
      return attached?.ws === s.id && attached.tab === tab ? { bytes: mirrorOf(tab) } : null;
    }
    await ticket.wait;
    // `detach`, outra aba ou outra tela ganhou enquanto o snapshot viajava.
    // A continuação antiga termina aqui, sem tocar no ChatView.
    if (!attachLife.current(ticket) || attached?.ws !== s.id || attached.tab !== tab) return null;
  }
  return { bytes: mirrorOf(tab) };
}

export function detach() {
  attachLife.cancel();
  if (attached) send({ t: "detach" });
  attached = null;
}

/// A fala de quem está olhando vai ao dono — se ele estiver aí. Vale para a
/// aba na tela, que é a única em que se escreve. Uma resposta a card vai pelo
/// mesmo caminho, como a linha de controle em JSON.
export function write(data: string) {
  if (!attached) return;
  const s = shares.get(attached.ws);
  if (!s) return;
  if (!s.online) {
    fail?.(t("err.team.offline"));
    return;
  }
  send({ t: "write", ws: s.id, tab: attached.tab, data });
}

function mirrorOf(tab: string): Uint8Array {
  return mirror.get(tab)?.bytes() ?? new Uint8Array(0);
}

function mirrorFor(tab: string): Mirror {
  let m = mirror.get(tab);
  if (!m) {
    m = new Mirror();
    mirror.set(tab, m);
  }
  return m;
}

function binary(data: ArrayBuffer) {
  const bin = decodeBinary(data);
  if (!bin) return;
  if (bin.kind === SNAPSHOT) {
    if (bin.to !== you) return;
    // Parte do meio: guarda e espera a última.
    if (!mirrorFor(bin.tab).seed(bin.bytes, bin.seq, bin.more)) return;
    const completed = attachLife.completeTab(bin.tab);
    if (!completed && attached?.tab === bin.tab) {
      // Ninguém pediu: o dono voltou (ou a conexão), e a tela renasce.
      guest.reset(bin.tab, mirrorOf(bin.tab));
    }
    return;
  }
  const m = mirrorFor(bin.tab);
  for (const seg of bin.segments) {
    const fresh = m.absorb(seg.seq, seg.bytes);
    if (fresh && attached?.tab === bin.tab) guest.live(bin.tab, fresh);
  }
}

/* ---------- comentários ---------- */

/// O id que o relay conhece: o de um colega vem prefixado na tela, o seu é
/// ele mesmo.
const relayId = (id: string) => remoteIds.get(id)?.ws ?? id;

/// O relay tem este workspace agora? Já em id de relay: é meu e anunciado, ou
/// é de um colega e veio no `welcome`. Perguntar sobre o que não está aqui é
/// pedir um `noShare`.
const known = (ws: string) => announced.has(ws) || shares.has(ws);

/// Os comentários de um workspace, e o pedido ao relay se ainda não vieram. Devolve
/// o que já se sabe; o resto chega pelo `onChange`.
export function notesOf(id: string): Note[] {
  // Workspace local não anunciado não existe no relay. Além de esconder a UI
  // de comentários no `main`, esta guarda impede que qualquer chamada futura produza
  // um `noShare` para um workspace que nunca foi compartilhado.
  if (!isRemote(id) && !announced.has(id)) return [];
  const ws = relayId(id);
  const have = notes.get(ws);
  if (have) return have;
  // Guardar a lista vazia é dizer "já pedi". Marque antes de enviar: o mock
  // responde no mesmo stack; marcar depois pisaria na resposta já recebida.
  notes.set(ws, []);
  if (!send({ t: "notes", ws })) notes.delete(ws);
  return [];
}

/// Escreve um comentário. `quote` é o trecho da conversa que ele cita, se cita, e
/// `mentions` são ids de membros — o relay descarta quem não existe. Devolve
/// se o pedido saiu: sem conexão o comentário não vai a lugar nenhum, e quem
/// escreveu precisa saber disso em vez de ver o campo esvaziar.
export function addNote(
  id: string,
  tab: string | null,
  anchor: string | null,
  text: string,
  mentions: string[],
  quote: string | null,
): boolean {
  includeMentioned(id, mentions);
  return send({ t: "note", ws: relayId(id), tab, anchor, text, mentions, quote });
}

function includeMentioned(id: string, mentions: string[]) {
  // Marcar quem está fora da audiência de um workspace seu é chamar a pessoa:
  // ela entra na lista antes de o comentário sair, senão o relay descarta a menção.
  // O share vai pelo mesmo socket, na frente do comentário — esperar o Rust
  // gravar e o quadro voltar deixaria o comentário chegar primeiro.
  const w = lastBoard?.workspaces.find((x) => x.id === id);
  if (w?.shared && !w.remote && w.audience) {
    const missing = mentions.filter((m) => !w.audience!.includes(m));
    if (missing.length) {
      const grown: Share = { ...toShare(w), audience: [...w.audience, ...missing] };
      if (send({ t: "share", share: grown })) announced.set(w.id, JSON.stringify(grown));
      void share(id, grown.audience);
    }
  }
}

export function replyNote(id: string, note: string, text: string, mentions: string[]): boolean {
  includeMentioned(id, mentions);
  return send({ t: "note_reply", ws: relayId(id), note, text, mentions });
}

export const resolveNote = (id: string, note: string) =>
  send({ t: "note_resolve", ws: relayId(id), note });

/// Quantos comentários abertos esperam você.
export const inboxCount = () => inbox.length;

/// No relay atual, abrir não conclui trabalho: o comentário fica na caixa até
/// alguém resolver. Relay sem threads conserva a leitura antiga como fallback.
export function readInbox(id: string): { workspace: string; note: string; tab: string | null } | null {
  const item = inbox.find((i) => i.id === id);
  if (!item) return null;
  const owned = [...remoteIds].find(([, r]) => r.ws === item.ws);
  // Relay antigo não tem resolução. Nele, abrir continua sendo o único jeito
  // de concluir a entrada; no relay atual, somente resolver remove.
  if (!comments) {
    send({ t: "inbox_read", id });
    inbox = inbox.filter((entry) => entry.id !== id);
    changed();
  }
  return { workspace: owned?.[0] ?? item.ws, note: item.id, tab: item.tab ?? null };
}

/// O que a caixa mostra: o comentário, de quem é, e onde está.
export function inboxList(): { id: string; ws: string; author: string; ts: number; title: string; text: string }[] {
  return inbox.map((i) => {
    const note = notes.get(i.ws)?.find((n) => n.id === i.id);
    const share = shares.get(i.ws);
    return {
      id: i.id,
      ws: i.ws,
      author: nameOf(i.author),
      ts: i.ts,
      title: share?.title ?? "",
      text: i.text ?? note?.text ?? "",
    };
  });
}
