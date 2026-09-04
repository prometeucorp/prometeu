import { listen } from "@tauri-apps/api/event";
import { invoke } from "./ipc";
import { icon } from "./icons";
import { fromBack, t, type Key } from "./i18n";
import * as menu from "./menu";
import type { Plugin } from "./types";
import { $, h, template } from "./util";

/// O hub de plugins na tela: a lista em Configurações, e o seletor que o
/// lançador e a conversa abrem.
///
/// O que um plugin é, e por que a escolha existe, está em
/// `src-tauri/src/plugins.rs`. Aqui só o que é tela — e ela é a mesma do hub de
/// MCP de propósito: mesma lista, mesmo seletor, mesma folha de cadastro. São
/// duas escolhas do mesmo tipo (o que o agente tem na mão, e como ele
/// trabalha), e aprender uma tem que ser aprender a outra.
///
/// O cadastro mora no back e esta é a cópia que a janela desenha; ela é refeita
/// a cada gravação, porque o back devolve a lista inteira depois de gravar —
/// nunca há duas verdades.
///
/// Instalar também é daqui: a folha pede o endereço de um repositório, o back
/// clona numa pasta do Prometeu e cadastra o que veio. Criar um plugin do
/// zero é o outro caminho, e a folha dele fica ao lado.

let hub: Plugin[] = [];
let loaded = false;
const watchers = new Set<() => void>();

/// A lista que a tela tem. Vazia antes de carregar — os seletores desenham
/// vazio e se refazem quando ela chega.
export const list = () => hub;

export const onChange = (fn: () => void) => {
  watchers.add(fn);
  return () => watchers.delete(fn);
};

function announce() {
  for (const fn of watchers) fn();
}

/// Carrega uma vez por sessão do app. O cadastro só muda por aqui, e quem o
/// muda já recebe a lista nova de volta.
export async function load() {
  if (loaded) return;
  loaded = true;
  try {
    hub = await invoke<Plugin[]>("plugin_hub");
    announce();
  } catch {
    // Sem back (ou back velho) a tela fica sem hub, e os seletores somem.
  }
}

/// O nome de um plugin que já não existe mais no hub continua gravado no
/// workspace — apagar do cadastro não pode mexer em quadro. O seletor mostra o
/// que sobrou como escolhido, para o buraco ter explicação.
export const known = (id: string) => hub.some((p) => p.id === id);

/* ---------- o seletor ---------- */

type Pick = {
  /// Quem está marcado agora. `null` é workspace que nunca escolheu.
  chosen: () => string[] | null;
  /// Devolve a lista nova. `null` nunca sai daqui — escolher é escolher.
  set: (ids: string[]) => void;
  /// Onde o menu cai.
  at: () => { x: number; y: number };
  /// Desligado enquanto o agente trabalha: plugin entra quando a sessão sobe, e
  /// derrubá-la no meio de um turno jogaria o turno fora.
  locked?: () => string;
};

/// `live` é o que a pessoa acabou de marcar, e ainda não voltou do back.
/// Marcar derruba o processo da conversa e republica o quadro inteiro; até
/// isso dar a volta, `p.chosen()` ainda responde o de antes — e o menu, que se
/// redesenha a cada clique, nascia com a marca no lugar velho. A tela passava
/// a impressão de que clicar não fazia nada.
export function openPicker(p: Pick, live?: string[]) {
  const lock = p.locked?.() ?? "";
  const chosen = live ?? p.chosen() ?? [];
  const items: menu.Item[] = [];
  if (lock) {
    items.push({ label: lock, disabled: true }, "sep");
  }
  if (!hub.length) {
    items.push({ label: t("plugin.none"), disabled: true });
  }
  for (const plugin of hub) {
    const on = chosen.includes(plugin.id);
    items.push({
      label: plugin.id,
      hint: plugin.note.trim(),
      checked: on,
      disabled: !!lock,
      run: () => {
        const next = on ? chosen.filter((id) => id !== plugin.id) : [...chosen, plugin.id];
        p.set(next);
        // O menu do app fecha ao escolher; marcar vários é reabrir — com o que
        // ela acabou de marcar, e não com o que o back ainda não confirmou.
        openPicker(p, next);
      },
    });
  }
  // Nomes gravados que o hub não tem mais: aparecem para poder sair.
  for (const id of chosen.filter((c) => !known(c))) {
    items.push({
      label: t("plugin.gone", { name: id }),
      checked: true,
      disabled: !!lock,
      run: () => {
        const next = chosen.filter((c) => c !== id);
        p.set(next);
        openPicker(p, next);
      },
    });
  }
  if (hub.length && !lock) {
    items.push("sep", {
      label: t("plugin.clear"),
      disabled: !chosen.length,
      run: () => {
        p.set([]);
        openPicker(p, []);
      },
    });
  }
  menu.openAt(p.at(), items);
}

/// O que o botão escreve: quantos entram. Nenhum é escolha e se diz por
/// extenso — "sem plugin" não é o mesmo que não ter escolhido.
export function label(chosen: string[] | null): string {
  if (chosen === null) return t("plugin.default");
  if (!chosen.length) return t("plugin.zero");
  if (chosen.length === 1) return chosen[0];
  return t("plugin.count", { n: String(chosen.length) });
}

/* ---------- a lista em Configurações ---------- */

type Ctx = { say: (text: string, isError?: boolean) => void };
let ctx: Ctx;

export function init(context: Ctx) {
  ctx = context;
}

/// As linhas da página "Plugins": uma por plugin, e a primeira é o que esta
/// página é e o que se faz nela.
export function settingsRows(): HTMLElement[] {
  return [aboutRow(), ...(hub.length ? hub.map(pluginRow) : [emptyRow()])];
}

function aboutRow(): HTMLElement {
  const row = template(
    "div",
    "setrow head",
    `<div class="txt"><span></span></div><div class="act"></div>`,
  );
  row.querySelector(".txt span")!.textContent = t("settings.plugins.body");

  // Instalar o que já existe é o que quase todo mundo vem fazer aqui; criar um
  // do zero e apontar para uma pasta são os dois casos raros, e ficam ao lado.
  const get = template("button", "outline md", `<span></span>`) as HTMLButtonElement;
  get.children[0].textContent = t("plugin.install");
  get.addEventListener("click", () => installer());

  const make = template("button", "ghost md", `<span></span>`) as HTMLButtonElement;
  make.children[0].textContent = t("plugin.make");
  make.addEventListener("click", () => maker());

  const add = template("button", "ghost md", `<span></span>`) as HTMLButtonElement;
  add.children[0].textContent = t("plugin.add");
  add.addEventListener("click", () => editor(null));

  row.querySelector(".act")!.append(get, make, add);
  return row;
}

function emptyRow(): HTMLElement {
  const row = h("div", "setrow none", "");
  row.textContent = t("plugin.empty");
  return row;
}

function pluginRow(plugin: Plugin): HTMLElement {
  const row = template(
    "div",
    "setrow",
    `<span class="glyph"></span><div class="txt"><b></b><span></span></div><div class="act"></div>`,
  );
  row.querySelector(".glyph")!.innerHTML = icon(remote(plugin.source) ? "globe" : "puzzle", 18);
  row.querySelector(".txt b")!.textContent = plugin.id;
  row.querySelector(".txt span")!.textContent = subtitle(plugin);

  const act = row.querySelector(".act")!;
  // Atualizar é o `git pull` da pasta clonada: só existe para o que veio de um
  // endereço, e some para o plugin apontado à mão.
  if (plugin.made && plugin.from) {
    const up = template("button", "ghost md", `<span></span>`) as HTMLButtonElement;
    up.children[0].textContent = t("plugin.update");
    up.addEventListener("click", () => {
      up.disabled = true;
      ctx.say(t("plugin.updating", { name: plugin.id }));
      invoke<Plugin[]>("plugin_update", { id: plugin.id })
        .then((fresh) => {
          hub = fresh;
          // A lista some e nasce de novo com o `announce`; a única notícia do
          // que aconteceu é esta linha, porque atualizar não muda nada na tela.
          ctx.say(t("plugin.updated", { name: plugin.id }));
          announce();
        })
        .catch((e) => {
          up.disabled = false;
          ctx.say(fromBack(e), true);
        });
    });
    act.append(up);
  }

  const edit = template("button", "ghost md", `<span></span>`) as HTMLButtonElement;
  edit.children[0].textContent = t("plugin.edit");
  edit.addEventListener("click", () => editor(plugin));

  const drop = template("button", "ghost md", `<span></span>`) as HTMLButtonElement;
  drop.children[0].textContent = t("plugin.remove");
  drop.addEventListener("click", () => {
    // O que nasceu aqui sai do disco junto: apagar arquivo pergunta antes.
    if (!plugin.made) return void remove(plugin);
    const at = drop.getBoundingClientRect();
    menu.openAt({ x: at.left, y: at.bottom + 4 }, [
      { label: t("plugin.remove.made"), danger: true, run: () => void remove(plugin) },
    ]);
  });

  act.append(edit, drop);
  return row;
}

const remote = (source: string) => /^https?:\/\//.test(source.trim());

/// A linha de baixo: de onde ele vem, e para que serve. Para o que foi
/// instalado, "de onde" é o endereço — a pasta do clone não diz nada a
/// ninguém.
function subtitle(plugin: Plugin): string {
  const where = plugin.from?.trim() || plugin.source;
  const note = plugin.note.trim();
  return note ? `${where} · ${note}` : where;
}

async function remove(plugin: Plugin) {
  try {
    hub = await invoke<Plugin[]>("plugin_remove", { id: plugin.id });
    announce();
  } catch (e) {
    ctx.say(fromBack(e), true);
  }
}

/// Depois de uma criação: quem gravou foi o back, e a lista daqui está velha.
export async function refresh() {
  hub = await invoke<Plugin[]>("plugin_hub");
  announce();
}

async function save(plugin: Plugin) {
  hub = await invoke<Plugin[]>("plugin_save", { plugin });
  announce();
}

/* ---------- o formulário ---------- */

/// Cadastrar um plugin é dizer onde ele está — o resto o próprio plugin já
/// declara. Por isso a origem vem primeiro e sair dela manda o Prometeu ler o
/// `plugin.json`: o nome e a descrição aparecem preenchidos, e quem quiser
/// muda. Uma folha só, e não os dois passos do MCP: aqui não há processo para
/// subir nem rede para atravessar.
function editor(plugin: Plugin | null) {
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet hubedit",
    `<div class="sheettop"><b class="mt"></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  const at = <T extends HTMLElement>(sel: string) => sheet.querySelector(sel) as T;
  const draft: Plugin = {
    id: plugin?.id ?? "",
    source: plugin?.source ?? "",
    note: plugin?.note ?? "",
    made: plugin?.made ?? false,
  };

  const hide = () => {
    veil.hidden = true;
    veil.replaceChildren();
  };

  const hint = h("span", "hint");
  const say = (text: string, bad = false) => {
    hint.textContent = text;
    hint.title = text;
    hint.classList.toggle("bad", bad);
  };

  /// Ler o que a origem declara. Origem inválida já se diz aqui — descobrir
  /// no fim, depois de tudo digitado, é descobrir tarde.
  async function look() {
    if (!draft.source.trim()) return;
    say(t("plugin.looking"));
    try {
      const found = await invoke<Plugin>("plugin_look", { source: draft.source });
      // O que a pessoa escreveu manda: preencher é para o campo vazio.
      if (!draft.id.trim()) draft.id = found.id;
      if (!draft.note.trim()) draft.note = found.note;
      say("");
      paint();
    } catch (e) {
      say(fromBack(e), true);
    }
  }

  function paint() {
    at(".mt").textContent = t(plugin ? "plugin.title.edit" : "plugin.title.new");
    at(".mbody").replaceChildren(
      h("p", "msay", t("plugin.intro")),
      field({
        label: "plugin.field.source",
        hint: "plugin.field.source.hint",
        value: draft.source,
        on: (v) => (draft.source = v),
        done: look,
      }),
      field({
        label: "plugin.field.name",
        hint: "plugin.field.name.hint",
        value: draft.id,
        on: (v) => (draft.id = v),
      }),
      field({
        label: "plugin.field.note",
        hint: "plugin.field.note.hint",
        value: draft.note,
        on: (v) => (draft.note = v),
      }),
    );
    const back = h("button", "ghost", t("plugin.cancel"));
    back.addEventListener("click", hide);
    const go = h("button", "pri", t("plugin.save"));
    go.addEventListener("click", store);
    at(".sheetbar").replaceChildren(back, hint, go);
  }

  function store() {
    if (!draft.id.trim() || !draft.source.trim()) return say(t("plugin.needFields"), true);
    save({ id: draft.id.trim(), source: draft.source.trim(), note: draft.note.trim(), made: draft.made })
      .then(hide)
      .catch((e) => say(fromBack(e), true));
  }

  paint();
  veil.replaceChildren(sheet);
  veil.hidden = false;
  if (!plugin) at<HTMLInputElement>("input")?.focus();
}

/// Um campo com o rótulo em cima e a explicação embaixo — o mesmo do hub de
/// MCP, e pelo mesmo motivo: `placeholder` some justamente quando serviria.
function field(o: {
  label: Key;
  hint: Key;
  value: string;
  on: (v: string) => void;
  /// Saiu do campo tendo mudado o que estava escrito.
  done?: () => void;
}): HTMLElement {
  const box = template(
    "label",
    "fld",
    `<span class="fl"></span><input spellcheck="false" /><span class="fh"></span>`,
  );
  box.querySelector(".fl")!.textContent = t(o.label);
  box.querySelector(".fh")!.textContent = t(o.hint);
  const input = box.querySelector("input")!;
  input.value = o.value;
  input.addEventListener("input", () => o.on(input.value));
  if (o.done) input.addEventListener("change", o.done);
  return box;
}

/* ---------- instalar ---------- */

/// O que um endereço trouxe, como o back conta.
type Found = { dir: string; plugins: Plugin[]; saved: boolean };

/// Instalar um plugin que já existe. Uma caixa: o endereço do repositório. O
/// repositório que é um plugin entra direto; o que traz vários vira uma lista
/// para marcar — e fechar sem marcar nada desfaz o clone, para o disco não
/// guardar o que ninguém escolheu.
function installer() {
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet hubedit",
    `<div class="sheettop"><b class="mt"></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  const at = <T extends HTMLElement>(sel: string) => sheet.querySelector(sel) as T;
  let source = "";
  let busy = false;
  let found: Found | null = null;
  const chosen = new Set<string>();

  const hide = () => {
    // O clone que ninguém escolheu não fica no disco.
    if (found && !found.saved) void invoke("plugin_scrap", { dir: found.dir });
    veil.hidden = true;
    veil.replaceChildren();
  };

  const hint = h("span", "hint");
  const say = (text: string, bad = false) => {
    hint.textContent = text;
    hint.title = text;
    hint.classList.toggle("bad", bad);
  };

  async function go() {
    if (!source.trim()) return say(t("plugin.install.needSource"), true);
    busy = true;
    say(t("plugin.install.working"));
    paint();
    try {
      const got = await invoke<Found>("plugin_install", { source });
      busy = false;
      if (got.saved) {
        await refresh();
        found = null;
        hide();
        return;
      }
      found = got;
      for (const plugin of got.plugins) chosen.add(plugin.id);
      say("");
      paint();
    } catch (e) {
      busy = false;
      say(fromBack(e), true);
      paint();
    }
  }

  /// Guardar o que foi marcado. Cada um é um cadastro, e o que sobrou no clone
  /// fica lá: é o mesmo repositório, e escolher de novo não baixa de novo.
  async function keep() {
    const picked = found?.plugins.filter((p) => chosen.has(p.id)) ?? [];
    if (!picked.length) return hide();
    try {
      for (const plugin of picked) await save(plugin);
      if (found) found.saved = true;
      hide();
    } catch (e) {
      say(fromBack(e), true);
    }
  }

  function paint() {
    at(".mt").textContent = t("plugin.install.title");
    at(".mbody").replaceChildren(...(found ? pick() : ask()));
    at(".sheetbar").replaceChildren(...bar());
  }

  function ask(): HTMLElement[] {
    return [
      h("p", "msay", t("plugin.install.intro")),
      field({
        label: "plugin.install.field",
        hint: "plugin.install.field.hint",
        value: source,
        on: (v) => (source = v),
        done: () => void go(),
      }),
    ];
  }

  /// A escolha, quando o repositório é um marketplace.
  function pick(): HTMLElement[] {
    const list = h("div", "mpick", "");
    for (const plugin of found?.plugins ?? []) {
      const line = template(
        "label",
        "mpickrow",
        `<input type="checkbox" /><div class="txt"><b></b><span></span></div>`,
      );
      const box = line.querySelector("input")!;
      box.checked = chosen.has(plugin.id);
      box.addEventListener("change", () => {
        if (box.checked) chosen.add(plugin.id);
        else chosen.delete(plugin.id);
      });
      line.querySelector("b")!.textContent = plugin.id;
      line.querySelector("span")!.textContent = plugin.note;
      list.append(line);
    }
    return [h("p", "msay", t("plugin.install.pick")), list];
  }

  function bar(): HTMLElement[] {
    const back = h("button", "ghost", t("plugin.cancel"));
    back.addEventListener("click", hide);
    const go2 = h("button", "pri", t(found ? "plugin.install.add" : "plugin.install.go"));
    go2.addEventListener("click", () => void (found ? keep() : go()));
    (go2 as HTMLButtonElement).disabled = busy;
    return [back, hint, go2];
  }

  paint();
  veil.replaceChildren(sheet);
  veil.hidden = false;
  at<HTMLInputElement>("input")?.focus();
}

/* ---------- criar ---------- */

/// Uma linha do que o agente está fazendo: `file` é um arquivo que ele acabou
/// de escrever, e a frase à volta é desta tela; `say` é palavra dele, e fica
/// como veio.
type Step = { kind: string; text: string };

/// As últimas linhas, e só: a folha mostra que ele está trabalhando, não o
/// histórico do que ele fez.
const STEPS = 8;

/// Criar um plugin aqui dentro. A folha tem dois estados: o pedido — o nome e
/// o que ele deve fazer — e o trabalho, que leva minutos e por isso mostra
/// cada arquivo que sai, em vez de um relógio.
function maker() {
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet hubedit",
    `<div class="sheettop"><b class="mt"></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  const at = <T extends HTMLElement>(sel: string) => sheet.querySelector(sel) as T;
  const draft = { name: "", ask: "" };
  /// A corrida em andamento. `null` é a folha ainda no pedido — antes de
  /// começar, e de volta a ele se o agente falhar.
  let run: number | null = null;
  let steps: Step[] = [];
  const off: (() => void)[] = [];
  let gone = false;

  const hide = () => {
    gone = true;
    for (const stop of off.splice(0)) stop();
    veil.hidden = true;
    veil.replaceChildren();
  };

  const hint = h("span", "hint");
  const say = (text: string, bad = false) => {
    hint.textContent = text;
    hint.title = text;
    hint.classList.toggle("bad", bad);
  };

  /// Os ouvintes ficam de pé enquanto a folha existe, e cada um só olha a sua
  /// corrida — qual é, a resposta do pedido conta. Fechar a folha antes de o
  /// ouvinte nascer o desliga assim que ele nasce.
  const hear = <T,>(event: string, fn: (payload: T) => void) => {
    void listen<T>(event, ({ payload }) => fn(payload)).then((stop) =>
      gone ? stop() : off.push(stop),
    );
  };
  hear<[number, Step]>("plugin-make", ([id, step]) => {
    if (id !== run) return;
    steps = [...steps, step].slice(-STEPS);
    paint();
  });
  hear<[number, string]>("plugin-made", ([id, error]) => {
    if (id !== run) return;
    // O que falhou volta para o pedido com o que estava escrito: o nome e o
    // parágrafo custaram a sair, e digitá-los de novo seria castigo.
    if (error) {
      run = null;
      steps = [];
      paint();
      say(fromBack(error), true);
      return;
    }
    void refresh().finally(hide);
  });

  async function start() {
    if (!draft.name.trim() || !draft.ask.trim()) return say(t("plugin.make.needFields"), true);
    say("");
    try {
      const made = await invoke<{ run: number; slug: string }>("plugin_make", {
        name: draft.name,
        ask: draft.ask,
      });
      run = made.run;
      paint();
    } catch (e) {
      say(fromBack(e), true);
    }
  }

  function stop() {
    if (run !== null) void invoke("plugin_make_stop", { run });
    hide();
  }

  /// O pedido: o nome, e o parágrafo que vira o plugin.
  function ask(): HTMLElement[] {
    return [
      h("p", "msay", t("plugin.make.intro")),
      field({
        label: "plugin.make.field.name",
        hint: "plugin.make.field.name.hint",
        value: draft.name,
        on: (v) => (draft.name = v),
      }),
      area({
        label: "plugin.make.field.ask",
        hint: "plugin.make.field.ask.hint",
        value: draft.ask,
        on: (v) => (draft.ask = v),
      }),
    ];
  }

  /// O trabalho, enquanto ele acontece. Sem passo nenhum ainda, a folha diz que
  /// está esperando — a primeira linha do agente demora.
  function working(): HTMLElement[] {
    const list = h("div", "mrun", "");
    if (!steps.length) list.append(h("div", "mstep wait", t("plugin.make.working")));
    for (const step of steps) {
      list.append(
        h("div", "mstep", step.kind === "file" ? t("plugin.make.wrote", { file: step.text }) : step.text),
      );
    }
    return [list];
  }

  function paint() {
    at(".mt").textContent = t("plugin.make.title");
    at(".mbody").replaceChildren(...(run === null ? ask() : working()));
    if (run === null) {
      const back = h("button", "ghost", t("plugin.cancel"));
      back.addEventListener("click", hide);
      const go = h("button", "pri", t("plugin.make.go"));
      go.addEventListener("click", () => void start());
      at(".sheetbar").replaceChildren(back, hint, go);
      return;
    }
    const halt = h("button", "ghost", t("plugin.make.stop"));
    halt.addEventListener("click", stop);
    say(t("plugin.make.working"));
    at(".sheetbar").replaceChildren(halt, hint);
  }

  paint();
  veil.replaceChildren(sheet);
  veil.hidden = false;
  at<HTMLInputElement>("input")?.focus();
}

/// O campo grande. O pedido é um parágrafo — num campo de uma linha alguém
/// escreveria uma frase, e uma frase não descreve um jeito de trabalhar.
function area(o: { label: Key; hint: Key; value: string; on: (v: string) => void }): HTMLElement {
  const box = template(
    "label",
    "fld",
    `<span class="fl"></span><textarea rows="6" spellcheck="true"></textarea><span class="fh"></span>`,
  );
  box.querySelector(".fl")!.textContent = t(o.label);
  box.querySelector(".fh")!.textContent = t(o.hint);
  const input = box.querySelector("textarea")!;
  input.value = o.value;
  input.addEventListener("input", () => o.on(input.value));
  return box;
}
