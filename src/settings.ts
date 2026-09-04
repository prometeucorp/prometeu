import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import * as alert from "./alert";
import { avatar, icon } from "./icons";
import { LANGS, choose, chosen, current as locale, fromBack, fromSystem, t, tn, type Key, type Lang } from "./i18n";
import {
  defaultEffort,
  defaultMcp,
  defaultModel,
  defaultPlugins,
  effortLadder,
  modelGroups,
  modelLabel,
  setDefaultEffort,
  setDefaultMcp,
  setDefaultModel,
  setDefaultPlugins,
} from "./launcher";
import * as mcp from "./mcp";
import * as menu from "./menu";
import * as plugins from "./plugins";
import * as news from "./news";
import * as team from "./team";
import type { LegacyImportPlan, LinearStatus } from "./types";
import { settingsRow } from "./update";
import { $, h, template } from "./util";

/// Configurações do app — o que não é do repositório (isso é o
/// `settings.toml`) nem de um workspace: a conexão com o Linear, a linha de
/// atualização (que mora em `update.ts`) e o idioma da tela. A página é a
/// uma das telas do app e entra no histórico ← → como as outras.
///
/// A conexão mora no back: o token nunca chega aqui. O que a tela sabe é o
/// `LinearStatus`, que chega no `init` e depois pelo evento `linear` toda vez
/// que muda — conectou, desconectou, começou a esperar o navegador. A linha
/// do Linear é quem diz em que pé está; a barra de cima só fala quando deu
/// erro, que é o que você precisa ler.

type Ctx = { say: (text: string, isError?: boolean) => void };

let ctx: Ctx;
let status: LinearStatus = { connected: false, who: null, busy: false };
let legacy: LegacyImportPlan | null = null;

export async function init(context: Ctx) {
  ctx = context;
  listen<LinearStatus>("linear", ({ payload }) => {
    status = payload;
    draw();
  });
  try {
    status = await invoke<LinearStatus>("linear_status");
  } catch {
    // Sem back (ou back velho) a tela continua de pé, só desconectada.
  }
  try {
    legacy = await invoke<LegacyImportPlan>("legacy_import_plan");
  } catch {
    // Back anterior à feature: a linha temporária simplesmente não aparece.
  }
  // Cadastrar, importar ou remover um servidor muda a lista desta página.
  mcp.onChange(() => {
    if (!$("settingsView").hidden) draw();
  });
  plugins.onChange(() => {
    if (!$("settingsView").hidden) draw();
  });
  // Presença muda sozinha; a linha do time acompanha — menos enquanto você
  // digita num campo dela, que refazer a página apagaria.
  team.onChange(() => {
    if ($("settingsView").hidden) return;
    const active = document.activeElement;
    if (active instanceof HTMLInputElement && $("settingsView").contains(active)) return;
    draw();
  });
}

/// O que o resto do app pergunta: tem Linear para puxar issue?
export const linear = () => status;

/// As páginas de Configurações. Uma lista à esquerda, uma página de cada vez à
/// direita — o mesmo desenho do Conductor, e pelo mesmo motivo: numa rolagem
/// só, o que se procura fica embaixo de coisa que não se procurava.
///
/// A ordem é a de quem chega: o que se mexe primeiro em cima, o que se mexe
/// uma vez na vida embaixo.
type Page = { id: string; title: Key; glyph: Parameters<typeof icon>[0]; rows: () => HTMLElement[] };

const PAGES: Page[] = [
  {
    id: "geral",
    title: "settings.page.general",
    glyph: "settings",
    rows: () => [langRow(), alert.settingsRow()],
  },
  {
    id: "padroes",
    title: "settings.defaults",
    glyph: "sparkles",
    rows: defaultsRows,
  },
  {
    id: "ferramentas",
    title: "settings.mcp",
    glyph: "plug",
    rows: () => mcp.settingsRows(),
  },
  {
    id: "plugins",
    title: "settings.plugins",
    glyph: "puzzle",
    rows: () => plugins.settingsRows(),
  },
  {
    id: "integracoes",
    title: "settings.integrations",
    glyph: "linear",
    rows: () => [linearRow()],
  },
  {
    id: "time",
    title: "settings.team",
    glyph: "users",
    rows: teamRows,
  },
  {
    id: "app",
    title: "settings.app",
    glyph: "flame",
    rows: appRows,
  },
];

/// Em qual página se estava. Gruda neste Mac: quem veio ajustar o MCP três
/// vezes numa tarde não quer passar pela lista toda a cada vez.
const PAGE_KEY = "prometeu:configuracoes";
let open = localStorage.getItem(PAGE_KEY) ?? PAGES[0].id;

export function draw() {
  const view = $("settingsView");
  const page = PAGES.find((p) => p.id === open) ?? PAGES[0];

  const nav = h("nav", "setnav");
  nav.append(
    ...PAGES.map((item) => {
      const btn = template("button", "setnavitem", `<span class="ic"></span><span></span>`);
      btn.querySelector(".ic")!.innerHTML = icon(item.glyph, 16);
      btn.children[1].textContent = t(item.title);
      btn.classList.toggle("on", item.id === page.id);
      btn.addEventListener("click", () => {
        open = item.id;
        localStorage.setItem(PAGE_KEY, item.id);
        draw();
      });
      return btn;
    }),
  );

  const body = template("div", "setpage", `<h1></h1>`);
  body.children[0].textContent = t(page.title);
  body.append(...page.rows());

  const wrap = h("div", "setwrap");
  wrap.append(nav, body);
  view.replaceChildren(wrap);
}

/* ---------- importação temporária do Prometheus ---------- */

function appRows(): HTMLElement[] {
  const rows = [settingsRow(), news.settingsRow()];
  if (legacy && legacy.state !== "missing") rows.unshift(migrationRow(legacy));
  return rows;
}

function importSummary(plan: LegacyImportPlan): string {
  return [
    tn(plan.counts.projects, "migration.projects"),
    tn(plan.counts.workspaces, "migration.workspaces"),
    tn(plan.counts.tabs, "migration.tabs"),
  ].join(" · ");
}

function migrationRow(plan: LegacyImportPlan): HTMLElement {
  const row = template(
    "div",
    "setrow",
    `<span class="glyph">${icon("archive-restore", 18)}</span><div class="txt"><b></b><span></span></div><div class="act"></div>`,
  );
  row.querySelector(".txt b")!.textContent = t("migration.title");
  const body = row.querySelector(".txt span")!;
  const action = row.querySelector(".act")!;

  if (plan.state === "ready") {
    body.textContent = t("migration.row.ready", { summary: importSummary(plan) });
    const review = h("button", "outline md", t("migration.review"));
    review.addEventListener("click", () => openMigration(plan));
    action.append(review);
  } else if (plan.state === "imported") {
    const date = plan.importedAt
      ? new Intl.DateTimeFormat(locale(), { dateStyle: "medium" }).format(new Date(plan.importedAt * 1000))
      : "";
    body.innerHTML = `<span class="ok"></span>`;
    body.querySelector(".ok")!.textContent = t("migration.row.imported", { date });
  } else if (plan.state === "targetNotEmpty") {
    body.textContent = t("migration.row.targetNotEmpty");
  } else {
    body.innerHTML = `<span class="bad"></span>`;
    body.querySelector(".bad")!.textContent = plan.problem ? fromBack(plan.problem) : t("migration.row.invalid");
  }
  return row;
}

function openMigration(plan: LegacyImportPlan) {
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet migration",
    `<div class="sheettop"><b></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  sheet.querySelector(".sheettop b")!.textContent = t("migration.title");
  const body = sheet.querySelector(".mbody")!;
  body.append(
    h("p", "lead", t("migration.preview", { summary: importSummary(plan) })),
    h(
      "p",
      "fact",
      t("migration.preview.history", {
        found: tn(plan.counts.transcripts, "migration.histories"),
        missing: tn(plan.counts.missingTranscripts, "migration.missingHistories"),
      }),
    ),
    h(
      "p",
      "fact",
      t("migration.preview.extras", {
        plugins: tn(plan.counts.plugins, "migration.plugins"),
        settings: tn(plan.counts.settings, "migration.settingsFiles"),
      }),
    ),
    h(
      "p",
      "fact",
      t("migration.preview.worktrees", {
        existing: plan.counts.existingWorktrees,
        total: plan.counts.worktrees,
      }),
    ),
    h("p", "fact", t("migration.preview.excluded")),
  );

  const closed = template("label", "migration-check", `<input type="checkbox"><span></span>`);
  closed.querySelector("span")!.textContent = t("migration.closed");
  const check = closed.querySelector("input") as HTMLInputElement;
  body.append(closed);

  const cancel = h("button", "ghost md", t("migration.cancel")) as HTMLButtonElement;
  const hint = h("span", "hint");
  const go = h("button", "pri md", t("migration.go")) as HTMLButtonElement;
  go.disabled = true;
  sheet.querySelector(".sheetbar")!.append(cancel, hint, go);

  let running = false;
  const hide = () => {
    if (running) return;
    veil.hidden = true;
    veil.replaceChildren();
    window.removeEventListener("keydown", key);
  };
  const key = (event: KeyboardEvent) => {
    if (event.key === "Escape") hide();
  };
  check.addEventListener("change", () => (go.disabled = !check.checked));
  cancel.addEventListener("click", hide);
  go.addEventListener("click", async () => {
    if (!check.checked || running) return;
    running = true;
    check.disabled = true;
    cancel.disabled = true;
    go.disabled = true;
    go.textContent = t("migration.doing");
    hint.textContent = "";
    try {
      const result = await invoke<LegacyImportPlan>("legacy_import_run");
      legacy = result;
      running = false;
      hide();
      try {
        await plugins.refresh();
      } catch {
        // O quadro e os arquivos já foram importados; a próxima abertura
        // relê o hub mesmo se este refresh visual falhar.
      }
      draw();
      ctx.say(t("migration.done", { backup: result.backup ?? "" }));
    } catch (error) {
      running = false;
      check.disabled = false;
      cancel.disabled = false;
      go.disabled = !check.checked;
      go.textContent = t("migration.go");
      hint.textContent = fromBack(error);
      hint.classList.add("bad");
    }
  });

  veil.onmousedown = (event) => {
    if (event.target === veil) hide();
  };
  window.addEventListener("keydown", key);
  veil.replaceChildren(sheet);
  veil.hidden = false;
}

/// O idioma da tela. Guardado neste Mac e em mais lugar nenhum; sem escolha, o
/// app segue o computador — e a linha diz em que isso dá, para "do sistema"
/// não ser uma resposta que esconde a pergunta.
function langRow(): HTMLElement {
  const row = template(
    "div",
    "setrow",
    `<span class="glyph">${icon("globe", 18)}</span><div class="txt"><b></b><span></span></div><div class="act"></div>`,
  );
  row.querySelector(".txt b")!.textContent = t("settings.lang");
  row.querySelector(".txt span")!.textContent = t("settings.lang.body");

  const system = LANGS.find(([id]) => id === fromSystem())?.[1] ?? fromSystem();
  const options: [Lang | null, string][] = [
    [null, t("settings.lang.system", { name: system })],
    ...LANGS.map(([id, name]) => [id, name] as [Lang, string]),
  ];
  const picked = chosen();

  const btn = template("button", "ghost md pick", `<span></span>${icon("chevron-down", 12)}`) as HTMLButtonElement;
  btn.children[0].textContent = options.find(([id]) => id === picked)![1];
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    menu.openAt(
      { x: at.left, y: at.bottom + 4 },
      options.map(([id, name]) => ({
        label: name,
        checked: id === chosen(),
        run: () => choose(id),
      })),
    );
  });
  row.querySelector(".act")!.append(btn);
  return row;
}

/* ---------- padrões ---------- */

/// Com o que o lançador abre: modelo, esforço, MCP e plugins. Antes isto era
/// lembrança — a última escolha do lançador virava o começo da próxima —, e
/// experimentar um modelo numa tarefa mudava calado todas as seguintes. Agora
/// é escolha, e mora aqui; o lançador continua trocando, só que para aquele
/// workspace e mais nada.
function defaultsRows(): HTMLElement[] {
  return [modelRow(), effortRow(), mcpRow(), pluginRow()];
}

/// Uma linha de Padrões: o que ela escolhe, o que isso quer dizer, e o botão
/// que abre o seletor. Devolve o botão junto porque quem marca vários (MCP,
/// plugins) reescreve o rótulo sem refazer a página — o menu fica aberto, e
/// refazer a página tiraria de baixo dele o botão em que ele se ancora.
function pickRow(
  glyph: Parameters<typeof icon>[0],
  title: Key,
  body: Key,
): { row: HTMLElement; btn: HTMLButtonElement } {
  const row = template(
    "div",
    "setrow",
    `<span class="glyph">${icon(glyph, 18)}</span><div class="txt"><b></b><span></span></div><div class="act"></div>`,
  );
  row.querySelector(".txt b")!.textContent = t(title);
  row.querySelector(".txt span")!.textContent = t(body);
  const btn = template("button", "ghost md pick", `<span></span>${icon("chevron-down", 12)}`) as HTMLButtonElement;
  row.querySelector(".act")!.append(btn);
  return { row, btn };
}

function modelRow(): HTMLElement {
  const { row, btn } = pickRow("sparkles", "settings.defaults.model", "settings.defaults.model.body");
  btn.children[0].textContent = modelLabel(defaultModel());
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    const blocks = modelGroups();
    const items: menu.Item[] = [];
    blocks.forEach((block, n) => {
      if (n) items.push("sep");
      if (block.head && blocks.length > 1) items.push({ label: block.head, disabled: true });
      for (const [id, name] of block.items) {
        items.push({
          label: name,
          checked: id === defaultModel(),
          run: () => {
            setDefaultModel(id);
            // O esforço é um degrau da escada do modelo, e a escada mudou: a
            // linha de baixo precisa se redesenhar junto.
            draw();
          },
        });
      }
    });
    menu.openAt({ x: at.left, y: at.bottom + 4 }, items);
  });
  return row;
}

/// O esforço padrão é um só, e a escada é a do modelo padrão — trocar de
/// modelo no lançador aproxima o degrau do que aquele modelo aceita.
function effortRow(): HTMLElement {
  const { row, btn } = pickRow("signal", "settings.defaults.effort", "settings.defaults.effort.body");
  const stairs = effortLadder(defaultModel());
  const now = defaultEffort(defaultModel());
  btn.children[0].textContent = stairs.find(([id]) => id === now)?.[1] ?? now;
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    menu.openAt(
      { x: at.left, y: at.bottom + 4 },
      stairs.map(([id, name]) => ({
        label: name,
        checked: id === now,
        run: () => {
          setDefaultEffort(id);
          draw();
        },
      })),
    );
  });
  return row;
}

function mcpRow(): HTMLElement {
  const { row, btn } = pickRow("plug", "settings.defaults.mcp", "settings.defaults.mcp.body");
  const unset = h("button", "ghost md", t("settings.defaults.unset")) as HTMLButtonElement;
  unset.title = t("settings.defaults.unset.title");
  const paintRow = () => {
    const chosen = defaultMcp();
    btn.children[0].textContent = mcp.label(chosen);
    unset.hidden = chosen === null;
  };
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    mcp.openPicker({
      chosen: defaultMcp,
      set: (ids) => {
        setDefaultMcp(ids);
        paintRow();
      },
      at: () => ({ x: at.left, y: at.bottom + 4 }),
    });
  });
  unset.addEventListener("click", () => {
    setDefaultMcp(null);
    paintRow();
  });
  row.querySelector(".act")!.prepend(unset);
  paintRow();
  return row;
}

function pluginRow(): HTMLElement {
  const { row, btn } = pickRow("puzzle", "settings.defaults.plugins", "settings.defaults.plugins.body");
  const unset = h("button", "ghost md", t("settings.defaults.unset")) as HTMLButtonElement;
  unset.title = t("settings.defaults.unset.title");
  const paintRow = () => {
    const chosen = defaultPlugins();
    btn.children[0].textContent = plugins.label(chosen);
    unset.hidden = chosen === null;
  };
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    plugins.openPicker({
      chosen: defaultPlugins,
      set: (ids) => {
        setDefaultPlugins(ids);
        paintRow();
      },
      at: () => ({ x: at.left, y: at.bottom + 4 }),
    });
  });
  unset.addEventListener("click", () => {
    setDefaultPlugins(null);
    paintRow();
  });
  row.querySelector(".act")!.prepend(unset);
  paintRow();
  return row;
}

function linearRow() {
  const row = template(
    "div",
    "setrow",
    `<span class="glyph">${icon("linear", 18)}</span><div class="txt"><b>Linear</b><span></span></div><div class="act"></div>`,
  );
  const text = row.querySelector(".txt span")!;
  const act = row.querySelector(".act")!;

  if (status.connected && status.who) {
    const { name, org } = status.who;
    text.innerHTML = `<span class="ok"></span> <span class="as"></span> <b></b> · <b></b>`;
    text.querySelector(".ok")!.textContent = t("linear.connected");
    text.querySelector(".as")!.textContent = t("linear.asWord");
    text.querySelectorAll("b")[0].textContent = name || status.who.email;
    text.querySelectorAll("b")[1].textContent = org || status.who.org_key;

    const off = h("button", "ghost md", t("linear.disconnect")) as HTMLButtonElement;
    off.title = t("linear.disconnect.title");
    off.addEventListener("click", async () => {
      off.disabled = true;
      try {
        status = await invoke<LinearStatus>("linear_disconnect");
      } catch (e) {
        ctx.say(fromBack(e), true);
      }
      draw();
    });
    act.append(off);
    return row;
  }

  text.textContent = t(status.busy ? "linear.waiting" : "linear.pitch");

  const on = template("button", "outline md", `<span></span> ${icon("external-link", 12)}`) as HTMLButtonElement;
  on.children[0].textContent = t("linear.connect");
  on.disabled = status.busy;
  on.title = t("linear.connect.title");
  on.addEventListener("click", async () => {
    on.disabled = true;
    status = { ...status, busy: true };
    draw();
    try {
      status = await invoke<LinearStatus>("linear_connect");
    } catch (e) {
      status = { ...status, busy: false };
      ctx.say(fromBack(e), true);
    }
    draw();
  });
  act.append(on);
  return row;
}

/* ---------- time ---------- */

/// O formulário aberto na linha do time: criar, entrar, ou nenhum. O que foi
/// digitado sobrevive a um redesenho — presença chega a qualquer hora.
type Mode = "idle" | "create" | "join";
let mode: Mode = "idle";
const draft = { name: "", code: "", relay: null as string | null };

function field(placeholder: string, value: string, onInput: (v: string) => void): HTMLInputElement {
  const input = document.createElement("input");
  input.className = "field";
  input.placeholder = placeholder;
  input.value = value;
  input.addEventListener("input", () => onInput(input.value));
  return input;
}

/// Uma ação do time: o que der errado vai para a barra, e a página é refeita
/// de qualquer jeito — o estado mudou, ou o formulário tem que voltar.
async function run(fn: () => Promise<void>) {
  try {
    await fn();
  } catch (e) {
    ctx.say(fromBack(e), true);
  }
  draw();
}

function sub(label: string): { row: HTMLElement; txt: HTMLElement; act: HTMLElement } {
  const row = template("div", "setrow sub", `<div class="txt"><b></b></div><div class="act"></div>`);
  row.querySelector("b")!.textContent = label;
  return { row, txt: row.querySelector(".txt")!, act: row.querySelector(".act")! };
}

function teamRows(): HTMLElement[] {
  const st = team.status();
  const rows: HTMLElement[] = [];
  const row = template(
    "div",
    "setrow",
    `<span class="glyph">${icon("users", 18)}</span><div class="txt"><b></b><span></span></div><div class="act"></div>`,
  );
  row.querySelector(".txt b")!.textContent = t("team.title");
  const text = row.querySelector(".txt span")!;
  const act = row.querySelector(".act")!;
  rows.push(row);


  if (!st.config) {
    text.textContent = t("team.pitch");
    if (mode === "idle") {
      const create = h("button", "outline md", t("team.create"));
      create.addEventListener("click", () => {
        mode = "create";
        draw();
      });
      const join = h("button", "ghost md", t("team.join"));
      join.addEventListener("click", () => {
        mode = "join";
        draw();
      });
      act.append(create, join);
    } else {
      const form = sub(mode === "create" ? t("team.create") : t("team.join"));
      form.row.classList.add("form");
      form.act.classList.add("form");
      const name = field(t("team.yourName"), draft.name || st.defaultName, (v) => (draft.name = v));
      const code = field(t("team.code"), draft.code, (v) => (draft.code = v));
      code.classList.add("wide");
      const go = h("button", "pri md", t(mode === "create" ? "team.go.create" : "team.go.join"));
      const cancel = h("button", "ghost md", t("team.cancel"));
      const submit = () =>
        run(async () => {
          if (mode === "create") await team.create(name.value);
          else await team.join(code.value, name.value);
          mode = "idle";
          draft.name = "";
          draft.code = "";
        });
      go.addEventListener("click", submit);
      for (const input of [name, code]) {
        input.addEventListener("keydown", (e) => {
          if (e.key === "Enter") submit();
          if (e.key === "Escape") cancel.click();
        });
      }
      cancel.addEventListener("click", () => {
        mode = "idle";
        draw();
      });
      if (mode === "join") form.act.append(code);
      form.act.append(name, go, cancel);
      rows.push(form.row);
      queueMicrotask(() => (mode === "join" ? code : name).focus());
    }
    rows.push(relayRow(st));
    return rows;
  }

  const online = st.members.filter((m) => m.online).length;
  if (st.phase === "online") {
    text.innerHTML = `<span class="ok"></span> <span class="as"></span> <b></b> · <span class="n"></span>`;
    text.querySelector(".ok")!.textContent = t("team.connected");
    text.querySelector(".as")!.textContent = t("team.asWord");
    text.querySelector("b")!.textContent = st.config.name;
    text.querySelector(".n")!.textContent = tn(online, "team.online");
  } else if (!st.relayEffective) {
    text.innerHTML = `<span class="bad"></span>`;
    text.querySelector(".bad")!.textContent = t("team.noRelay");
  } else {
    text.textContent = t("team.connecting");
  }

  const copy = template("button", "ghost md", `${icon("copy", 12)} <span></span>`);
  copy.querySelector("span")!.textContent = t("team.copyInvite");
  copy.title = t("team.copyInvite.title");
  copy.addEventListener("click", () => {
    navigator.clipboard.writeText(team.invite() ?? "");
    ctx.say(t("team.copied"));
  });
  const leave = h("button", "ghost md", t("team.leave"));
  leave.title = t("team.leave.title");
  leave.addEventListener("click", () => run(() => team.leave()));
  act.append(copy, leave);

  // Quem está no time, com quem está aí agora aceso.
  const who = sub(t("team.members"));
  const list = h("div", "members");
  for (const m of st.members) {
    const chip = template("span", "mem" + (m.online ? "" : " off"), `${avatar(m.name)}<span class="nm"></span><i class="dot"></i>`);
    chip.querySelector(".nm")!.textContent = m.id === st.you ? `${m.name} (${t("team.you")})` : m.name;
    chip.title = m.online ? "" : t("team.offline");
    list.append(chip);
  }
  who.txt.append(list);
  rows.push(who.row);

  const me = sub(t("team.yourName"));
  const name = field(t("team.yourName"), st.config.name, () => {});
  name.title = t("team.rename.title");
  const save = () => {
    if (name.value.trim() && name.value.trim() !== st.config!.name) run(() => team.setName(name.value));
  };
  name.addEventListener("keydown", (e) => e.key === "Enter" && save());
  name.addEventListener("blur", save);
  me.act.append(name);
  rows.push(me.row);

  rows.push(relayRow(st));
  return rows;
}

/// Onde o time se encontra. Quase ninguém mexe: é para quem hospeda o próprio
/// relay, e para o dev apontar para o `wrangler dev`.
function relayRow(st: team.TeamStatus): HTMLElement {
  const r = sub(t("team.relay"));
  const body = h("span", "", "");
  body.textContent = st.relayDefault ? t("team.relay.body", { url: st.relayDefault }) : t("team.relay.none");
  r.txt.append(body);
  const url = field(t("team.relay.placeholder"), draft.relay ?? st.relay, (v) => (draft.relay = v));
  url.classList.add("wide");
  const save = h("button", "ghost md", t("team.relay.save"));
  const go = () =>
    run(async () => {
      await team.setRelay(url.value);
      draft.relay = null;
    });
  save.addEventListener("click", go);
  url.addEventListener("keydown", (e) => e.key === "Enter" && go());
  r.act.append(url, save);
  return r.row;
}
