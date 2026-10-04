import * as accountUI from "./accounts";
import * as actions from "./actions";
import * as actionSettings from "./action-settings";
import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { avatar, icon } from "./icons";
import { LANGS, choose, chosen, fromBack, fromSystem, t, type Key, type Lang } from "./i18n";
import { defaultMcp, defaultPlugins, setDefaultMcp, setDefaultPlugins } from "./launcher";
import { choiceLabel, defaultChoice, defaultEffort, effortStep, fitsEffort, setDefaultChoice, setDefaultEffort } from "./model-choice";
import { openModelPicker, openEffortPicker } from "./model-picker";
import { onCatalogChange } from "./agents";
import * as mcp from "./mcp";
import * as catalog from "./catalog";
import * as skills from "./skills";
import * as menu from "./menu";
import * as voice from "./voice";
import * as plugins from "./plugins";
import * as projects from "./projects";
import * as news from "./news";
import * as notifications from "./notification-settings";
import * as team from "./team";
import * as typesafe from "./typesafe";
import { telemetryRows } from "./telemetry-settings";
import { typesafeRows } from "./typesafe-settings";
import type { LinearStatus } from "./types";
import { settingsRow } from "./update";
import { $, h, template } from "./util";
import { button, disclosure, field, input, select } from "./ui";

import { matchesSettings, settingsDestination, type SettingsPage } from "./settings-navigation";
import { resourceSettings, setResourceFilter } from "./settings-resources";
import "./settings.css";

/// Application preferences include Linear, updates, defaults, and interface language.
/// Connection secrets remain in the backend; this view receives LinearStatus updates.

type Ctx = { say: (text: string, isError?: boolean) => void };

let ctx: Ctx;
let status: LinearStatus = { connected: false, can_assign: false, who: null, busy: false };

export async function init(context: Ctx) {
  ctx = context;
  onCatalogChange(() => { if (!$("settingsView").hidden) { paintDefaultModel(); paintDefaultEffort(); } });
  accountUI.onChange(() => refreshPages("agentes"));
  skills.init(ctx.say, async () => {
    await plugins.refresh();
    await catalog.load();
  });
  skills.onChange(() => refreshPages("recursos", "agentes"));
  actions.onChange(() => refreshPages("acoes", "trabalho"));
  listen<LinearStatus>("linear", ({ payload }) => {
    status = payload;
    refreshPages("trabalho");
  });
  // The optional TypeSafe integration starts disabled until its local configuration loads.
  typesafe.onChange(() => refreshPages("trabalho"));
  void typesafe.refresh();
  try {
    status = await invoke("linear_status");
  } catch {
    // Keep the page usable and disconnected if the backend is unavailable or older.
  }
  // Server registration, import, and removal update this page.
  mcp.onChange(() => refreshPages("recursos", "agentes"));
  plugins.onChange(() => refreshPages("recursos", "agentes"));
  // Cloud catalogs include entries that are not installed locally.
  catalog.init(async () => {
    await Promise.all([plugins.refresh(), mcp.refresh(), skills.refresh()]);
  });
  catalog.onChange(() => refreshPages("recursos", "agentes", "trabalho"));
  // Refresh presence without replacing an input the user is editing.
  team.onChange(() => {
    if ($("settingsView").hidden || open !== "trabalho") return;
    const active = document.activeElement;
    if (active instanceof HTMLInputElement && $("settingsView").contains(active)) return;
    draw();
  });
}

function refreshPages(...pages: SettingsPage[]) {
  if (!$("settingsView").hidden && pages.includes(open)) draw();
}

/// Whether Linear is available to load issues.
export const linear = () => status;

type Section = { id: string; title: Key; rows: () => HTMLElement[]; keywords?: Key[] };
type Page = { id: SettingsPage; title: Key; description: Key; glyph: Parameters<typeof icon>[0]; sections: Section[] };

const PAGES: Page[] = [
  { id: "geral", title: "settings.page.general", description: "settings.general.intro", glyph: "settings", sections: [
    { id: "language", title: "settings.languageVoice", rows: () => [langRow(), ...(voice.available() ? [voiceRow()] : [])], keywords: ["settings.lang", "settings.voice"] },
    { id: "notifications", title: "notifications.title", rows: () => [notifications.settings(ctx.say, true)], keywords: ["notifications.events", "notifications.sound", "notifications.style"] },
    { id: "app", title: "settings.app", rows: appRows, keywords: ["update.ask", "settings.news"] },
  ] },
  { id: "agentes", title: "settings.agents", description: "settings.agents.intro", glyph: "sparkles", sections: [
    { id: "defaults", title: "settings.newWorkspaces", rows: defaultsRows, keywords: ["settings.defaults", "settings.defaults.model", "settings.defaults.effort", "settings.initialResources"] },
    { id: "accounts", title: "account.title", rows: () => [accountUI.render(undefined, undefined, true)] },
  ] },
  { id: "recursos", title: "settings.resources", description: "settings.resources.intro", glyph: "plug", sections: [
    { id: "resources", title: "settings.resources", rows: () => [resourceSettings(() => navigate("agentes", "defaults"))], keywords: ["settings.mcp", "settings.plugins", "skill.title"] },
  ] },
  { id: "acoes", title: "actions.title", description: "actions.intro", glyph: "list-tree", sections: [
    { id: "actions", title: "actions.title", rows: () => actionSettings.settingsRows(draw, ctx.say), keywords: ["actions.commands", "actions.profiles"] },
  ] },
  { id: "trabalho", title: "settings.work", description: "settings.work.intro", glyph: "users", sections: [
    { id: "team", title: "settings.team", rows: teamRows },
    { id: "projects", title: "settings.localProjects", rows: projectRows, keywords: ["projects.title"] },
    { id: "telemetry", title: "telemetry.title", rows: telemetryRows },
    { id: "integrations", title: "settings.integrations", rows: () => [linearRow(), ...typesafeRows(ctx.say, draw)], keywords: ["typesafe.enabled"] },
  ] },
];

const PAGE_KEY = "prometeu:configuracoes";
const initial = settingsDestination(localStorage.getItem(PAGE_KEY));
let open: SettingsPage = initial.page;
let targetSection = initial.section;
let searchQuery = "";
if (initial.filter) setResourceFilter(initial.filter);

export function showAccounts() { open = "agentes"; targetSection = "accounts"; searchQuery = ""; localStorage.setItem(PAGE_KEY, open); }
export function showLinear() { open = "trabalho"; targetSection = "integrations"; searchQuery = ""; localStorage.setItem(PAGE_KEY, open); }

function navigate(page: SettingsPage, section?: string) {
  open = page; targetSection = section; searchQuery = "";
  localStorage.setItem(PAGE_KEY, open); draw();
}

export function draw() {
  const view = $("settingsView");
  if (view.hidden) return;
  // Close snapshot menus before recording their restored trigger focus.
  if (menu.isOpen() && !document.querySelector("dialog[open]") && $("veil").hidden) menu.close();
  const previous = document.activeElement instanceof HTMLElement && view.contains(document.activeElement) ? document.activeElement : null;
  const focus = previous?.dataset.focus;
  const selection = previous instanceof HTMLInputElement ? [previous.selectionStart, previous.selectionEnd] : null;
  const scroll = view.querySelector(".setpage")?.scrollTop ?? 0;
  const details = new Map([...view.querySelectorAll<HTMLDetailsElement>("details[data-settings-disclosure]")].map(node => [node.dataset.settingsDisclosure, node.open]));
  const samePage = view.dataset.settingsPage === open;
  view.dataset.settingsPage = open;
  const page = PAGES.find(item => item.id === open)!;
  const nav = h("nav", "setnav"); nav.setAttribute("aria-label", t("settings.title"));
  const search = input(searchQuery); search.type = "search"; search.placeholder = t("settings.search");
  search.setAttribute("aria-label", t("settings.search")); search.dataset.focus = "settings-search";
  for (const item of PAGES) {
    const control = button(t(item.title), () => navigate(item.id), "ghost");
    control.classList.add("setnavitem"); control.dataset.focus = `settings-nav-${item.id}`;
    control.insertAdjacentHTML("afterbegin", `<span class="ic">${icon(item.glyph, 16)}</span>`);
    if (item.id === page.id && !searchQuery.trim()) { control.classList.add("on"); control.setAttribute("aria-current", "page"); }
    nav.append(control);
  }
  const body = h("div", "setpage");
  const header = h("header", "settings-header");
  const breadcrumb = h("p", "settings-breadcrumb");
  const heading = h("h1", "");
  const description = h("p", "ui-hint", t(page.description));
  header.append(breadcrumb, heading, search, description);
  const content = h("div", "settings-content");
  body.append(header, content);
  function paintBody() {
    content.replaceChildren();
    heading.textContent = t(searchQuery.trim() ? "settings.searchResults" : page.title);
    breadcrumb.textContent = `${t("settings.title")} / ${heading.textContent}`;
    description.hidden = !!searchQuery.trim();
    if (searchQuery.trim()) {
      const results = h("div", "settings-panel settings-results");
      for (const candidate of PAGES) for (const section of candidate.sections) {
        const words = [t(candidate.title), t(section.title), ...(section.keywords ?? []).map(key => t(key)), section.id === "accounts" ? "Claude Codex Antigravity" : "", section.id === "integrations" ? "Linear TypeSafe" : ""];
        if (!matchesSettings(searchQuery, words.join(" "))) continue;
        const result = button("", () => navigate(candidate.id, section.id), "ghost");
        result.classList.add("settings-result");
        result.append(h("b", "", t(section.title)), h("span", "ui-hint", t(candidate.title)));
        results.append(result);
      }
      if (!results.childElementCount) results.append(h("p", "settings-empty", t("settings.searchEmpty")));
      content.append(results); return;
    }
    for (const section of page.sections) {
      const group = h("section", "settings-section"); group.id = `settings-${section.id}`;
      const title = h("h2", "", t(section.title)); title.id = `${group.id}-title`;
      group.setAttribute("aria-labelledby", title.id);
      const panel = h("div", "settings-panel"); panel.append(...section.rows());
      if (page.id === "recursos" || page.id === "acoes") { title.hidden = true; panel.classList.add("settings-unboxed"); }
      group.append(title, panel); content.append(group);
    }
  }
  search.oninput = () => {
    searchQuery = search.value;
    nav.querySelectorAll(".setnavitem").forEach(control => {
      const current = !searchQuery.trim() && control.getAttribute("data-focus") === `settings-nav-${open}`;
      control.classList.toggle("on", current);
      if (current) control.setAttribute("aria-current", "page"); else control.removeAttribute("aria-current");
    });
    paintBody();
  };
  search.onkeydown = event => {
    if (event.key === "Escape") { searchQuery = ""; search.value = ""; search.dispatchEvent(new Event("input")); }
    if (event.key === "ArrowDown") { event.preventDefault(); body.querySelector<HTMLButtonElement>(".settings-result")?.focus(); }
  };
  paintBody();
  const wrap = h("div", "setwrap"); wrap.append(nav, body); view.replaceChildren(wrap);
  if (samePage) {
    for (const node of body.querySelectorAll<HTMLDetailsElement>("details[data-settings-disclosure]")) node.open = details.get(node.dataset.settingsDisclosure) ?? false;
    body.scrollTop = scroll;
  }
  if (targetSection) {
    const target = body.querySelector<HTMLElement>(`#settings-${targetSection}`);
    target?.querySelectorAll<HTMLDetailsElement>("details:not(.account-usage)").forEach(node => node.open = true);
    target?.scrollIntoView({ block: "start" });
    const heading = target?.querySelector<HTMLElement>("h2");
    if (heading && !heading.hidden) { heading.tabIndex = -1; heading.focus({ preventScroll: true }); }
    targetSection = undefined;
  }
  if (focus) {
    const next = view.querySelector<HTMLElement>(`[data-focus="${CSS.escape(focus)}"]`);
    next?.focus({ preventScroll: true });
    if (next instanceof HTMLInputElement && selection && selection[0] !== null) next.setSelectionRange(selection[0], selection[1]);
  }
}

function projectRows(): HTMLElement[] {
  const rows = actions.projects().map(project => {
    const row = template("div", "setrow", `<span class="glyph">${icon("folder", 18)}</span><div class="txt"><b></b><span></span></div>`);
    row.querySelector("b")!.textContent = project.name; row.querySelector(".txt span")!.textContent = project.path;
    return row;
  });
  return [...rows, ...projects.settingsRows(ctx.say)];
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

  const choice = select(picked ?? "", options.map(([id, label]) => [id ?? "", label]));
  choice.control.setAttribute("aria-label", t("settings.lang"));
  choice.onchange = () => choose(choice.value === "" ? null : choice.value as Lang);
  row.querySelector(".act")!.append(choice.control);
  return row;
}

/// Dictation language stays independent from the interface: people often read one language and speak another.
function voiceRow(): HTMLElement {
  const row = template(
    "div",
    "setrow",
    `<span class="glyph">${icon("mic", 18)}</span><div class="txt"><b></b><span></span></div><div class="act"></div>`,
  );
  row.querySelector(".txt b")!.textContent = t("settings.voice");
  row.querySelector(".txt span")!.textContent = t("settings.voice.body");
  const options: [string | null, string][] = [
    [null, t("settings.voice.interface")],
    ...voice.TAGS.map((tag) => [tag, voice.nameOf(tag)] as [string, string]),
  ];
  const choice = select(voice.chosen() ?? "", options.map(([id, label]) => [id ?? "", label]));
  choice.control.setAttribute("aria-label", t("settings.voice"));
  choice.onchange = () => voice.choose(choice.value || null);
  row.querySelector(".act")!.append(choice.control);
  return row;
}

// Defaults.

/// Launcher defaults are explicit preferences. Experimenting within one workspace
/// does not change the starting configuration of future workspaces.
function defaultsRows(): HTMLElement[] {
  const resources = disclosure(t("settings.initialResources"), mcpRow(), pluginRow());
  resources.dataset.settingsDisclosure = "defaults-resources";
  return [modelRow(), effortRow(), resources, h("p", "ui-hint settings-section-note", t("settings.defaults.hint"))];
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
  const btn = button("", () => {});
  btn.classList.add("pick"); btn.append(h("span", ""));
  btn.insertAdjacentHTML("beforeend", icon("chevron-down", 12));
  btn.setAttribute("aria-label", t(title));
  row.querySelector(".act")!.append(btn);
  return { row, btn };
}

let defaultEffortAdjusted = false;
let paintDefaultModel = () => {};
let paintDefaultEffort = () => {};
function modelRow(): HTMLElement {
  const { row, btn } = pickRow("sparkles", "settings.defaults.model", "settings.defaults.model.body");
  paintDefaultModel = () => {
    const choice = defaultChoice();
    btn.children[0].textContent = choice ? choiceLabel(choice) : t("models.choose");
    btn.title = defaultEffortAdjusted ? t("models.effortAdjusted") : "";
  };
  paintDefaultModel();
  btn.addEventListener("click", () => {
    const choice = defaultChoice();
    openModelPicker(btn, {
      current: choice ?? { agent: "claude", model: "" },
      select: selected => {
        const previous = choice ? defaultEffort(choice) : "";
        const effort = fitsEffort(selected.model, previous, selected.agent);
        setDefaultChoice(selected);
        setDefaultEffort(effort);
        defaultEffortAdjusted = effort !== previous;
        paintDefaultModel(); paintDefaultEffort();
      },
    });
  });
  return row;
}

function effortRow(): HTMLElement {
  const { row, btn } = pickRow("signal", "settings.defaults.effort", "settings.defaults.effort.body");
  paintDefaultEffort = () => {
    const choice = defaultChoice();
    const now = choice ? defaultEffort(choice) : "";
    const step = choice ? effortStep(choice.model, now, choice.agent) : null;
    row.hidden = !step;
    btn.children[0].textContent = step?.label ?? "";
  };
  paintDefaultEffort();
  btn.addEventListener("click", () => {
    const choice = defaultChoice();
    if (!choice) return;
    openEffortPicker(btn, choice, defaultEffort(choice), effort => {
      setDefaultEffort(effort);
      defaultEffortAdjusted = false;
      paintDefaultModel(); paintDefaultEffort();
    });
  });
  return row;
}

function mcpRow(): HTMLElement {
  const { row, btn } = pickRow("plug", "settings.defaults.mcp", "settings.defaults.mcp.body");
  const unset = button(t("settings.defaults.unset"), () => {}, "ghost");
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
  const unset = button(t("settings.defaults.unset"), () => {}, "ghost");
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
  const connect = async (button: HTMLButtonElement) => {
    button.disabled = true;
    status = { ...status, busy: true };
    draw();
    try {
      status = await invoke("linear_connect");
    } catch (e) {
      status = { ...status, busy: false };
      ctx.say(fromBack(e), true);
    }
    draw();
  };

  if (status.connected && status.who) {
    const { name, org } = status.who;
    text.innerHTML = `<span class="ok"></span> <span class="as"></span> <b></b> · <b></b>`;
    text.querySelector(".ok")!.textContent = t("linear.connected");
    text.querySelector(".as")!.textContent = t("linear.asWord");
    text.querySelectorAll("b")[0].textContent = name || status.who.email;
    text.querySelectorAll("b")[1].textContent = org || status.who.org_key;

    if (!status.can_assign) {
      text.append(` · ${t("linear.readOnly")}`);
      const upgrade = h("button", "outline md", t("linear.allowAssignment")) as HTMLButtonElement;
      upgrade.disabled = status.busy;
      upgrade.addEventListener("click", () => void connect(upgrade));
      act.append(upgrade);
    }

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
  on.addEventListener("click", () => void connect(on));
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
