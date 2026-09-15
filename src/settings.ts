import * as actions from "./actions";
import * as actionSettings from "./action-settings";
import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { avatar, icon } from "./icons";
import { LANGS, choose, chosen, fromBack, fromSystem, t, type Key, type Lang } from "./i18n";
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
import * as catalog from "./catalog";
import * as skills from "./skills";
import * as menu from "./menu";
import * as plugins from "./plugins";
import * as news from "./news";
import * as team from "./team";
import type { Board, LinearStatus, Selection, Tools } from "./types";
import { settingsRow } from "./update";
import { $, h, template } from "./util";
import { button, field, select } from "./ui";

/// Application preferences include Linear, updates, defaults, and interface language.
/// Connection secrets remain in the backend; this view receives LinearStatus updates.

type Ctx = { say: (text: string, isError?: boolean) => void };

let ctx: Ctx;
let status: LinearStatus = { connected: false, who: null, busy: false };
/// The board's global tool layer (ADR 0043), pushed from the board event so the defaults page can edit it.
let globalTools: Tools | undefined;

/// Receive each board snapshot; redraw the defaults page when it is open so global picks stay current.
export function boardChanged(board: Board) {
  // Board events fire on every agent turn. The settings page only renders the global tool layer,
  // so redraw just when it changes instead of disturbing an open page on every churn.
  const unchanged = JSON.stringify(board.tools ?? null) === JSON.stringify(globalTools ?? null);
  globalTools = board.tools;
  if (!unchanged && !$("settingsView").hidden) draw();
}

export async function init(context: Ctx) {
  ctx = context;
  skills.init(ctx.say);
  skills.onChange(() => { if (!$("settingsView").hidden) draw(); });
  actions.onChange(() => { if (!$("settingsView").hidden) draw(); });
  listen<LinearStatus>("linear", ({ payload }) => {
    status = payload;
    draw();
  });
  try {
    status = await invoke("linear_status");
  } catch {
    // Keep the page usable and disconnected if the backend is unavailable or older.
  }
  // Server registration, import, and removal update this page.
  mcp.onChange(() => {
    if (!$("settingsView").hidden) draw();
  });
  plugins.onChange(() => {
    if (!$("settingsView").hidden) draw();
  });
  // Cloud catalogs include entries that are not installed locally.
  catalog.init(async () => {
    await Promise.all([plugins.refresh(), mcp.refresh(), skills.refresh()]);
  });
  catalog.onChange(() => {
    if (!$("settingsView").hidden) draw();
  });
  // Refresh presence without replacing an input the user is editing.
  team.onChange(() => {
    if ($("settingsView").hidden) return;
    const active = document.activeElement;
    if (active instanceof HTMLInputElement && $("settingsView").contains(active)) return;
    draw();
  });
}

/// Whether Linear is available to load issues.
export const linear = () => status;

/// Group settings into separate pages, ordered from frequent choices to occasional maintenance.
type Page = { id: string; title: Key; glyph: Parameters<typeof icon>[0]; rows: () => HTMLElement[] };

const PAGES: Page[] = [
  {
    id: "geral",
    title: "settings.page.general",
    glyph: "settings",
    rows: () => [langRow()],
  },
  {
    id: "padroes",
    title: "settings.defaults",
    glyph: "sparkles",
    rows: defaultsRows,
  },
  {
    id: "acoes", title: "actions.title", glyph: "list-tree", rows: () => actionSettings.settingsRows(draw, ctx.say),
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
  { id: "skills", title: "skill.title", glyph: "sparkles", rows: () => skills.settingsRows() },
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

/// Remember the last settings page on this Mac.
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

function appRows(): HTMLElement[] {
  return [settingsRow(), news.settingsRow()];
}

/// Store language preference locally; without an override, show the resolved system language.
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

// Defaults.

/// Launcher defaults are explicit preferences. Experimenting within one workspace
/// does not change the starting configuration of future workspaces.
function defaultsRows(): HTMLElement[] {
  return [modelRow(), effortRow(), mcpRow(), pluginRow(), ...globalToolsRows()];
}

/// The board's global tool layer: the base every project and workspace inherits (ADR 0043). Editing it
/// writes one axis at a time through `set_tools_global`; a workspace with no layer of its own follows it.
function globalToolsRows(): HTMLElement[] {
  const head = h("div", "setrow head");
  head.append(h("div", "txt", t("settings.global.body")));
  const axis = (
    glyph: Parameters<typeof icon>[0],
    title: Key,
    axisName: "mcp" | "plugins" | "skills",
    label: (sel: Selection | null) => string,
    open: (p: { current: () => Selection | null; set: (sel: Selection | null) => void; at: () => { x: number; y: number } }) => void,
    write: (sel: Selection | null) => void,
  ): HTMLElement => {
    const { row, btn } = pickRow(glyph, title, "settings.global.axis");
    const current = (): Selection | null => globalTools?.[axisName] ?? null;
    btn.children[0].textContent = label(current());
    btn.addEventListener("click", () => {
      const at = btn.getBoundingClientRect();
      open({ current, set: write, at: () => ({ x: at.left, y: at.bottom + 4 }) });
    });
    return row;
  };
  const send = (args: { mcp?: Selection | null; plugins?: Selection | null; skills?: Selection | null }) =>
    invoke("set_tools_global", args).catch((e) => ctx.say(fromBack(e), true));
  return [
    head,
    axis("plug", "settings.mcp", "mcp", mcp.label, mcp.openGlobalPicker, (sel) => send({ mcp: sel })),
    axis("puzzle", "settings.plugins", "plugins", plugins.label, plugins.openGlobalPicker, (sel) => send({ plugins: sel })),
    axis("sparkles", "skill.title", "skills", (sel) => plugins.label(sel, plugins.SKILL_WORDS), plugins.openSkillGlobalPicker, (sel) => send({ skills: sel })),
  ];
}

/// Return the picker button so multiple selections can update its label
/// without replacing the open menu or its anchor.
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
            // The new model can change available effort levels, so redraw both rows.
            draw();
          },
        });
      }
    });
    menu.openAt({ x: at.left, y: at.bottom + 4 }, items);
  });
  return row;
}

/// The default effort follows the default model; the launcher adjusts it for another model.
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
    btn.children[0].textContent = mcp.flatLabel(chosen);
    unset.hidden = chosen === null;
  };
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    mcp.openDefaultPicker({
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
    btn.children[0].textContent = plugins.flatLabel(chosen);
    unset.hidden = chosen === null;
  };
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    plugins.openDefaultPicker({
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
        status = await invoke("linear_disconnect");
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
      status = await invoke("linear_connect");
    } catch (e) {
      status = { ...status, busy: false };
      ctx.say(fromBack(e), true);
    }
    draw();
  });
  act.append(on);
  return row;
}

// Organization.

function teamRows(): HTMLElement[] {
  const st = team.status();
  const rows: HTMLElement[] = [];
  const row = template("div", "setrow", `<span class="glyph">${icon("users", 18)}</span><div class="txt"><b></b><span></span></div><div class="act"></div>`);
  row.querySelector(".txt b")!.textContent = st.config?.cloud?.name ?? t("organization.title");
  row.querySelector(".txt span")!.textContent = st.config
    ? t(st.phase === "online" ? "team.connected" : "team.connecting")
    : t("organization.hint");
  const manage = button(t("organization.manage"), () => {
    const path = st.config?.cloud ? `/orgs/${encodeURIComponent(st.config.cloud.slug)}` : "/settings/organizations";
    void invoke("open_external", { url: `${st.account.origin || "https://app.prometeu.co"}${path}` }).catch(e => ctx.say(fromBack(e), true));
  });
  row.querySelector(".act")!.append(manage);
  rows.push(row);
  if (st.organizations.length) {
    const choices = select(st.config?.cloud ? st.config.team : "", [
      ["", t("organization.none")],
      ...st.organizations.map(org => [org.id, org.name] as [string, string]),
    ]);
    choices.onchange = () => {
      const value = choices.value;
      void (value ? team.selectOrganization(value) : team.leave()).catch(e => ctx.say(fromBack(e), true));
    };
    rows.push(field(t("organization.active"), choices.control, t("organization.scopeHint")));
  }
  if (st.config && !st.config.cloud) {
    const legacy = h("div", "setrow");
    legacy.append(h("p", "ui-hint", t("organization.legacy")), button(t("team.leave"), () => {
      void team.leave().catch(e => ctx.say(fromBack(e), true));
    }, "ghost"));
    rows.push(legacy);
  }
  // One chip per person: companion devices fold into their member and only lend it their online state.
  const people = team.people();
  if (people.length) {
    const you = team.personOf(st.you);
    const list = h("div", "members");
    for (const member of people) {
      const chip = template("span", "mem" + (member.online ? "" : " off"), `${avatar(member.name)}<span class="nm"></span><i class="dot"></i>`);
      chip.querySelector(".nm")!.textContent = member.id === you ? `${member.name} (${t("team.you")})` : member.name;
      chip.title = t(member.online ? "team.online" : "team.offline");
      list.append(chip);
    }
    const membersRow = h("div", "setrow");
    membersRow.append(h("b", "", t("team.members")), list);
    rows.push(membersRow);
  }
  if (st.config) rows.push(h("p", "ui-hint", t("team.security.hint")));
  return rows;
}
