import { dropdown, type Group } from "./ui";
export type { Group } from "./ui";
import { open } from "@tauri-apps/plugin-dialog";
import {
  capabilitiesOf,
  descriptors,
  effortsOf,
  installed,
  isKnownModel,
  modelLabelOf,
  modelsOf,
  providerOfModel,
  usesNativeUltraLabel,
} from "./agents";
import { freshBranch } from "./branch";
import { avatar, icon } from "./icons";
import { paint, t } from "./i18n";
import * as issues from "./issues";
import * as mcp from "./mcp";
import * as plugins from "./plugins";
import * as menu from "./menu";
import { invoke } from "./ipc";
import { pasteFiles } from "./paste";
import { template } from "./util";
import { branchTaken, type Board, type Issue, type IssueRef, type ProviderId, type Workspace } from "./types";

export type Draft = {
  project: string;
  /// Additional repositories share the workspace branch in sibling worktrees under one agent working directory.
  extras: string[];
  branch: string;
  /// Base for the new branch; empty only before references load.
  base: string;
  /// Enable a dedicated worktree, or switch branches in the existing clone when disabled.
  worktree: boolean;
  /// Without branch creation, open the repository as it is. Dedicated worktrees always require their own branch.
  newBranch: boolean;
  title: string;
  stage: string;
  prompt: string;
  inject: string[];
  /// An originating Linear issue supplies the workspace title, branch, and initial prompt.
  issue: IssueRef | null;
  /// Derive the provider from the selected model's catalog association.
  agent: ProviderId;
  /// Choose a discovered provider model explicitly for the whole workspace rather than offering a CLI-default choice.
  model: string;
  /// Always choose an effort for new workspaces; empty legacy values omit the override.
  effort: string;
  /// Start only the first conversation in plan mode until its plan is approved.
  plan: boolean;
  /// Null MCP selection preserves inherited CLI behavior; do not silently override existing user configuration.
  mcp: string[] | null;
  /// Null plugin selection preserves inherited CLI behavior, following the MCP rule.
  plugins: string[] | null;
};


/// Workspace agents use unattended permissions; no launcher toggle changes that behavior.

/// Model selection also chooses its provider; no separate provider control is needed.
export const agentOf = providerOfModel;

/// Share model groups between launcher and tab controls. Restrict existing conversations to their provider because resume identities are not interchangeable across CLIs.
export function modelGroups(only?: ProviderId): Group[] {
  return installed()
    .filter((provider) => only === undefined || provider.id === only)
    .map((provider) => ({
      head: provider.label,
      items: modelsOf(provider.id).map((model) => [model.id, model.label] as [string, string]),
    }))
    .filter((group) => group.items.length > 0);
}

/// Clamp effort to a supported level when switching models so the CLI never receives an unsupported value.
export function fitsEffort(model: string, effort: string, provider = providerOfModel(model)): string {
  const stairs = ladderOf(model, provider);
  if (stairs.some(([id]) => id === effort)) return effort;
  return stairs[stairs.length - 1]?.[0] ?? "high";
}

/// Default to the first installed provider's model instead of selecting an unavailable CLI.
const fallbackModel = () =>
  installed().flatMap((provider) => modelsOf(provider.id))[0]?.id ?? "";

/// Cycle supported effort levels with one click, wrapping after the last. New launchers choose an explicit effort; ultracode adds application orchestration where supported.
const EFFORTS: [string, string][] = [
  ["low", t("effort.low")],
  ["medium", t("effort.medium")],
  ["high", t("effort.high")],
  ["xhigh", t("effort.xhigh")],
  ["max", t("effort.max")],
  ["ultracode", t("effort.ultracode")],
];

/// Normalize provider effort catalogs for presentation, including application orchestration above Claude xhigh. Preserve the fallback ladder when no catalog levels are published.
function ladderOf(model: string, provider = providerOfModel(model)): [string, string][] {
  const accepted = effortsOf(provider, model);
  if (!accepted.length) return EFFORTS;
  return EFFORTS.filter(([id]) => accepted.includes(id)).map(([id, name]) => [
    id,
    id === "ultracode" ? t(usesNativeUltraLabel(provider) ? "effort.ultra" : "effort.ultracode") : name,
  ]);
}

/// Use the same readable model label in launcher and conversation footers.
export function modelLabel(model: string, provider?: ProviderId): string {
  return modelLabelOf(model, provider);
}

/// Cycle within the model's effort ladder; unknown values start at its first level.
export function nextEffort(model: string, effort: string, provider = providerOfModel(model)): string {
  const stairs = ladderOf(model, provider);
  const step = stairs.findIndex(([id]) => id === effort);
  return stairs[(step + 1) % stairs.length]?.[0] ?? effort;
}

/// Expose the current effort position and ladder length for the indicator; unknown values light no bars.
export function effortStep(
  model: string,
  effort: string,
  provider = providerOfModel(model),
): { label: string; step: number; total: number } | null {
  const stairs = ladderOf(model, provider);
  const step = stairs.findIndex(([id]) => id === effort);
  if (step === -1) return null;
  return { label: stairs[step][1], step, total: stairs.length };
}

/// Persist worktree preference across launches. Plan mode belongs to one task and is not remembered.
const WORKTREE_KEY = "prometeu:worktree";
const BRANCH_KEY = "prometeu:branch-nova";

/// Model, effort, MCP, and plugin defaults are explicit Settings choices. Launcher changes affect only the new workspace. Retain existing storage keys for compatibility with older remembered choices.
const MODEL_KEY = "prometeu:model";
const EFFORT_KEY = "prometeu:effort";
const MCP_KEY = "prometeu:mcp";
const PLUGIN_KEY = "prometeu:plugins";

/// main.ts receives native drops and forwards attachments to the open launcher.
let takeFiles: { put: (paths: string[]) => void; wait: () => () => void } | null = null;
export const fileDropTarget = () => takeFiles;

/// Create from the prompt and project selection, deriving title, a readable branch name, and initial stage. Enter submits. A Linear seed supplies its branch and title and prepends the full issue to any extra instructions.
export type Open = {
  /// Preselect the active workspace's project or the sidebar creation target.
  preset?: string;
  seed?: Issue;
  /// The Git panel can seed a chosen base or branch in an isolated worktree.
  git?: { base: string; branch?: string };
  go: (d: Draft) => void;
  /// The Linear setup action closes the launcher and opens Settings.
  toSettings: () => void;
};

export function openLauncher(board: Board, opts: Open) {
  const veil = document.getElementById("veil")!;
  if (!board.projects.length) return;
  const { preset, go, git } = opts;
  const project = preset ?? board.projects[0].id;
  let seed = opts.seed;

  const draft: Draft = {
    project,
    extras: [],
    // Regenerate the branch name once repository references arrive and collisions can be checked.
    branch: git?.branch || seed?.branch_name || freshBranch([]),
    base: git?.base ?? "",
    worktree: !!git || localStorage.getItem(WORKTREE_KEY) !== "0",
    newBranch: !!git || localStorage.getItem(BRANCH_KEY) !== "0",
    title: seed ? `${seed.identifier} · ${seed.title}` : "",
    stage: board.stages[1] ?? board.stages[0],
    prompt: "",
    inject: [],
    issue: seed ? { id: seed.id, identifier: seed.identifier, title: seed.title, url: seed.url } : null,
    agent: "claude",
    model: defaultModel(),
    effort: "",
    plan: false,
    mcp: defaultMcp(),
    plugins: defaultPlugins(),
  };
  draft.effort = defaultEffort(draft.model);
  // The catalog associates the default model with its provider.
  draft.agent = agentOf(draft.model);
  const conformCapabilities = () => {
    const capabilities = capabilitiesOf(draft.agent);
    if (!capabilities.initialPlanMode) draft.plan = false;
    if (!capabilities.workspaceMcpSelection) draft.mcp = null;
    if (!capabilities.workspacePluginSelection) draft.plugins = null;
    if (!capabilities.attachments) draft.inject = [];
  };
  conformCapabilities();

  const sheet = document.createElement("div");
  sheet.className = "sheet";
  sheet.innerHTML = `
    <div class="sheettop">
      <span class="who"><span id="d-avatar"></span><button id="d-project" class="ghost pick"><span></span>${icon("chevron-down", 12)}</button><button id="d-more" class="ico sm" data-t-title="launcher.addRepo">${icon("plus", 14)}</button></span>
      <button id="d-base" class="ghost base" data-t-title="launcher.base.title">
        ${icon("git-branch", 12)}<span id="d-basename"></span>${icon("chevron-down", 12)}
      </button>
      <button id="d-issuebtn" class="ghost base empty" data-t-title="launcher.issue.title">
        ${icon("linear", 12)}<span></span>${icon("chevron-down", 12)}
      </button>
      <span class="spacer"></span>
      <button id="d-nb" class="ghost sw" role="switch">
        <span data-t="launcher.newBranch"></span><i class="knob"></i>
      </button>
      <button id="d-wt" class="ghost sw" role="switch">
        <span data-t="launcher.worktree"></span><i class="knob"></i>
      </button>
    </div>
    <div class="picker" id="d-picker" hidden></div>
    <div class="picker" id="d-ipicker" hidden></div>
    <textarea id="d-prompt" rows="6"></textarea>
    <div class="attach" id="d-repos" hidden></div>
    <div class="attach" id="d-issue" hidden></div>
    <div class="attach" id="d-inj" hidden></div>
    <div class="sheetbar">
      <button id="d-model" class="ghost pick" data-t-title="launcher.model.title">${icon("sparkles", 14)}<span></span>${icon("chevron-down", 12)}</button>
      <button id="d-effort" class="ghost effort"><span class="bars"><i></i><i></i><i></i><i></i><i></i></span><span class="el"></span></button>
      <button id="d-plan" class="ghost">${icon("map", 14)}<span data-t="launcher.plan"></span></button>
      <button id="d-mcp" class="ghost pick" data-t-title="mcp.title">${icon("plug", 14)}<span></span></button>
      <button id="d-plugins" class="ghost pick" data-t-title="plugin.title">${icon("puzzle", 14)}<span></span></button>
      <span class="hint" id="d-hint"></span>
      <button id="d-add" class="ico" data-t-title="launcher.attach">${icon("paperclip", 16)}</button>
      <button id="d-go" class="pri"><span data-t="launcher.go"></span> <kbd>↵</kbd></button>
    </div>`;
  paint(sheet);

  const $ = <T extends HTMLElement>(id: string) => sheet.querySelector(`#${id}`) as T;
  const prompt = $<HTMLTextAreaElement>("d-prompt");
  const hint = $("d-hint");

  const nameOf = (id: string) => board.projects.find((p) => p.id === id)?.name ?? "";
  const projectName = () => nameOf(draft.project);
  // Disable creation while another workspace owns the branch at a conflicting path, preserving the typed request.
  let taken: Workspace | null = null;
  let receiving = 0;
  const drawHint = () => {
    $("d-avatar").innerHTML = avatar(projectName());
    const from = draft.base ? ` ← ${draft.base}` : "";
    const onde = !draft.newBranch
      ? t("launcher.hint.here")
      : t(
          draft.extras.length ? "launcher.hint.multi" : draft.worktree ? "launcher.hint.worktree" : "launcher.hint.switch",
          { branch: draft.branch, from },
        );

    const names = [draft.project, ...draft.extras].map(nameOf).join(" + ");
    // Git cannot check out one branch at two paths. Linear can suggest an existing branch whose destination changes with the selected repository set.
    taken =
      draft.newBranch && draft.worktree
        ? branchTaken(board, [draft.project, ...draft.extras], draft.branch)
        : null;
    const aviso = taken ? t("launcher.hint.taken", { ws: taken.title }) : "";
    hint.classList.toggle("bad", !!aviso);
    hint.title = aviso || `${names} · ${onde}`;
    hint.textContent = aviso || onde;
    $<HTMLButtonElement>("d-go").disabled = !!aviso || receiving > 0;
  };
  drawHint();

  /* Multiple repositories. */

  // Represent cross-repository work as one workspace with sibling worktrees on the same branch. Show extra repositories as chips and offer only missing projects.
  const more = $<HTMLButtonElement>("d-more");
  const reposBox = $("d-repos");
  const others = () => board.projects.filter((p) => p.id !== draft.project && !draft.extras.includes(p.id));
  const drawExtras = () => {
    more.hidden = board.projects.length < 2;
    // Multiple repositories require Git-backed worktrees.
    more.disabled = !others().length || !isGit;
    more.title = t(!isGit ? "launcher.noGit" : others().length ? "launcher.addRepo" : "launcher.addRepo.none");
    reposBox.hidden = !draft.extras.length;
    reposBox.replaceChildren(
      ...draft.extras.map((id) => {
        const chip = template("span", "injchip repo", `${icon("git-branch", 12)}<span></span><button class="ico sm">${icon("x", 12)}</button>`);
        chip.children[1].textContent = nameOf(id);
        (chip.children[2] as HTMLElement).title = t("launcher.removeRepo", { name: nameOf(id) });
        chip.children[2].addEventListener("click", () => {
          draft.extras = draft.extras.filter((x) => x !== id);
          drawExtras();
          drawSwitches();
          prompt.focus();
        });
        return chip;
      }),
    );
  };
  more.addEventListener("click", () => {
    const at = more.getBoundingClientRect();
    menu.openAt(
      { x: at.left, y: at.bottom + 4 },
      others().map((p) => ({
        label: p.name,
        run: () => {
          draft.extras.push(p.id);
          // A shared parent directory makes multiple repositories available to one agent session.
          draft.worktree = true;
          draft.newBranch = true;
          drawExtras();
          drawSwitches();
          prompt.focus();
        },
      })),
    );
  });

  /* Worktree and branch controls. */

  // Support a dedicated worktree with a branch, a new branch in the clone, or the current clone unchanged. A worktree without its own branch is invalid.
  const wt = $<HTMLButtonElement>("d-wt");
  const nb = $<HTMLButtonElement>("d-nb");

  // Disable branch/worktree controls for non-Git directories after reference discovery confirms their status.
  let isGit = true;

  const drawSwitches = () => {
    for (const [el, on] of [
      [wt, draft.worktree],
      [nb, draft.newBranch],
    ] as const) {
      el.classList.toggle("on", on);
      el.setAttribute("aria-checked", String(on));
    }
    nb.disabled = draft.worktree || !isGit;
    nb.title = t(!isGit ? "launcher.noGit" : draft.worktree ? "launcher.nb.locked" : "launcher.nb.off");
    wt.disabled = !!git || draft.extras.length > 0 || !isGit;
    wt.title = t(!isGit ? "launcher.noGit" : draft.extras.length ? "launcher.wt.locked" : draft.worktree ? "launcher.wt.on" : "launcher.wt.off");
    // No new branch means no base selection.
    baseBtn.disabled = !draft.newBranch || !branches.length;
    if (!draft.newBranch) basePick.close();
    drawHint();
  };

  wt.addEventListener("click", () => {
    draft.worktree = !draft.worktree;
    if (draft.worktree) draft.newBranch = true;
    localStorage.setItem(WORKTREE_KEY, draft.worktree ? "1" : "0");
    localStorage.setItem(BRANCH_KEY, draft.newBranch ? "1" : "0");
    drawSwitches();
  });
  nb.addEventListener("click", () => {
    draft.newBranch = !draft.newBranch;
    localStorage.setItem(BRANCH_KEY, draft.newBranch ? "1" : "0");
    drawSwitches();
  });

  /* Model, effort, and plan controls. */

  // Return focus to the prompt after changing secondary launch choices.
  const drawAttach = () => {
    const supported = capabilitiesOf(draft.agent).attachments;
    $<HTMLButtonElement>("d-add").hidden = !supported;
    if (!supported) {
      $("d-inj").hidden = true;
      $("d-inj").replaceChildren();
    }
  };
  const drawModel = dropdown(
    $("d-model"),
    modelGroups,
    () => draft.model,
    (id) => {
      draft.model = id;
      draft.agent = agentOf(id);
      // Changing provider/model may invalidate current capabilities and effort.
      conformCapabilities();
      draft.effort = fits(draft.effort);
      drawEffort();
      drawPlan();
      drawMcp();
      drawPlugins();
      drawAttach();
      prompt.focus();
    },
  );

  // Cycle only efforts supported by the selected model; unsupported choices would fail at the CLI.
  const effort = $<HTMLButtonElement>("d-effort");
  const drawEffort = () => {
    const stairs = ladder();
    const step = Math.max(0, stairs.findIndex(([id]) => id === draft.effort));
    const ultra = stairs[step][0] === "ultracode";
    effort.querySelector(".el")!.textContent = stairs[step][1];
    effort.querySelectorAll(".bars i").forEach((bar, n) => bar.classList.toggle("lit", n <= step));
    effort.classList.toggle("ultra", ultra);
    effort.title = t(ultra ? "launcher.effort.ultra" : "launcher.effort.title");
  };
  effort.addEventListener("click", () => {
    const stairs = ladder();
    const step = stairs.findIndex(([id]) => id === draft.effort);
    draft.effort = stairs[(step + 1) % stairs.length][0];
    drawEffort();
    prompt.focus();
  });

  /// The current model's supported effort ladder.
  const ladder = () => ladderOf(draft.model, draft.agent);

  const fits = (level: string) => fitsEffort(draft.model, level, draft.agent);

  draft.effort = fits(draft.effort);
  drawEffort();
  drawModel();

  const plan = $<HTMLButtonElement>("d-plan");
  const drawPlan = () => {
    plan.hidden = !capabilitiesOf(draft.agent).initialPlanMode;
    plan.classList.toggle("on", draft.plan);
    plan.setAttribute("aria-pressed", String(draft.plan));
    plan.title = t(draft.plan ? "launcher.plan.on" : "launcher.plan.off");
  };
  plan.addEventListener("click", () => {
    draft.plan = !draft.plan;
    drawPlan();
    prompt.focus();
  });
  drawPlan();
  drawAttach();

  // Offer MCP selection only when the hub has entries; Settings owns registration.
  const mcpBtn = $<HTMLButtonElement>("d-mcp");
  const drawMcp = () => {
    mcpBtn.hidden =
      !capabilitiesOf(draft.agent).workspaceMcpSelection ||
      (!mcp.list().length && draft.mcp === null);
    mcpBtn.querySelector("span")!.textContent = mcp.flatLabel(draft.mcp);
    mcpBtn.classList.toggle("on", !!draft.mcp?.length);
  };
  mcpBtn.addEventListener("click", () => {
    const at = mcpBtn.getBoundingClientRect();
    mcp.openDefaultPicker({
      chosen: () => draft.mcp,
      set: (ids) => {
        draft.mcp = ids;
        drawMcp();
      },
      at: () => ({ x: at.left, y: at.bottom + 4 }),
    });
  });
  const forgetMcp = mcp.onChange(drawMcp);
  drawMcp();

  // Plugin selection also requires hub entries and runtime support for workspace selection.
  const plugBtn = $<HTMLButtonElement>("d-plugins");
  const drawPlugins = () => {
    plugBtn.hidden =
      !capabilitiesOf(draft.agent).workspacePluginSelection ||
      (!plugins.list().length && draft.plugins === null);
    plugBtn.querySelector("span")!.textContent = plugins.flatLabel(draft.plugins);
    plugBtn.classList.toggle("on", !!draft.plugins?.length);
  };
  plugBtn.addEventListener("click", () => {
    const at = plugBtn.getBoundingClientRect();
    plugins.openDefaultPicker({
      chosen: () => draft.plugins,
      set: (ids) => {
        draft.plugins = ids;
        drawPlugins();
      },
      at: () => ({ x: at.left, y: at.bottom + 4 }),
    });
  });
  const forgetPlugins = plugins.onChange(drawPlugins);
  drawPlugins();

  /* Branch base. */

  // Changing projects refreshes available bases because repository references differ.
  const baseBtn = $<HTMLButtonElement>("d-base");
  const baseName = $("d-basename");
  let branches: string[] = [];

  const setBase = (name: string) => {
    draft.base = name;
    baseName.textContent = name || t("launcher.base.none");
    baseBtn.classList.toggle("empty", !name);
    drawHint();
  };

  const basePick = picker({
    btn: baseBtn,
    el: $("d-picker"),
    placeholder: t("launcher.base.pick"),
    rows: () => branches.map((name) => ({ id: name, label: name, run: () => setBase(name) })),
    none: () => t(branches.length ? "launcher.base.noMatch" : "launcher.base.empty"),
    current: () => draft.base,
    after: () => prompt.focus(),
  });

  const loadBranches = async () => {
    const loadingProject = draft.project;
    const fromGit = loadingProject === project ? git : undefined;
    branches = [];
    isGit = true;
    baseBtn.disabled = true;
    baseName.textContent = t("launcher.loading");
    try {
      const got = await invoke("list_branches", { project: draft.project });
      if (draft.project !== loadingProject) return;
      branches = got.all;
      isGit = got.git;
      // A non-Git directory opens directly without branch operations.
      if (!isGit) {
        draft.worktree = false;
        draft.newBranch = false;
      }
      drawSwitches();
      // Refresh generated names after loading repository references while preserving explicit Git or issue selections.
      if (!seed) draft.branch = fromGit?.branch || freshBranch(branches);
      setBase(fromGit?.base ?? got.default);
    } catch {
      if (draft.project !== loadingProject) return;
      // A newly initialized repository without references branches from its current HEAD.
      setBase(fromGit?.base ?? "");
      drawSwitches();
    }
    baseBtn.disabled = !draft.newBranch || !branches.length;
  };

  /* Originating issue. */

  // Show the selected issue as part of the first prompt; its chip supplies the title/branch and can be removed.
  const issueBox = $("d-issue");
  const issueBtn = $<HTMLButtonElement>("d-issuebtn");
  const setSeed = (issue: Issue | undefined) => {
    seed = issue;
    draft.issue = issue ? { id: issue.id, identifier: issue.identifier, title: issue.title, url: issue.url } : null;
    draft.branch = (draft.project === project ? git?.branch : undefined) || issue?.branch_name || freshBranch(branches);
    draft.title = issue ? `${issue.identifier} · ${issue.title}` : "";
    prompt.placeholder = t(issue ? "launcher.prompt.issue" : "launcher.prompt");
    issueBtn.querySelector("span")!.textContent = issue?.identifier ?? t("launcher.issue");
    issueBtn.classList.toggle("empty", !issue);
    issueBox.hidden = !issue;
    issueBox.replaceChildren();
    if (issue) {
      const chip = template(
        "span",
        "injchip issue",
        `${icon("linear", 12)}<b class="iid"></b><span class="it"></span><button class="ico sm">${icon("x", 12)}</button>`,
      );
      chip.children[1].textContent = issue.identifier;
      chip.children[2].textContent = issue.title;
      chip.title = `${issue.title}\n${issue.url}`;
      chip.children[3].addEventListener("click", () => {
        setSeed(undefined);
        prompt.focus();
      });
      issueBox.append(chip);
    }
    drawHint();
  };

  // List assigned Linear issues, or offer connection setup when unavailable.
  const issuePick = picker({
    btn: issueBtn,
    el: $("d-ipicker"),
    placeholder: t("launcher.issue.pick"),
    rows: () => {
      const list = issues.list();
      if (list === null) {
        return [
          {
            id: "@configurar",
            label: t("launcher.issue.setup"),
            glyph: icon("arrow-right", 14),
            run: () => {
              hide();
              opts.toSettings();
            },
          },
        ];
      }
      return list.map((i) => ({ id: i.id, label: i.title, sub: i.identifier, run: () => setSeed(i) }));
    },
    none: () =>
      issues.busy()
        ? t("launcher.issue.busy")
        : t(issues.list()?.length ? "launcher.issue.noMatch" : "launcher.issue.empty"),
    current: () => draft.issue?.id ?? "",
    after: () => prompt.focus(),
  });
  // Refresh missing or stale issue data when opening the picker.
  issueBtn.addEventListener("click", () => {
    if (issuePick.isOpen()) void issues.load().then(() => issuePick.isOpen() && issuePick.draw());
  });

  // Changing projects refreshes the branch base and conflict labels.
  dropdown(
    $("d-project"),
    () => [{ items: board.projects.map((p) => [p.id, p.name] as [string, string]) }],
    () => draft.project,
    (id) => {
      draft.project = id;
      // The primary project cannot also appear among additional repositories.
      draft.extras = draft.extras.filter((x) => x !== id);
      basePick.close();
      loadBranches();
      drawExtras();
      drawHint();
      prompt.focus();
    },
  );
  drawExtras();
  drawSwitches();
  loadBranches();
  setSeed(seed);

  sheet.addEventListener("mousedown", (e) => {
    for (const p of [basePick, issuePick]) {
      if (p.isOpen() && !p.contains(e.target as Node)) p.close();
    }
  });

  // Keep attachments visible between the prompt and footer because they belong to the first message.
  const injList = $("d-inj");
  const drawInject = () => {
    injList.hidden = !draft.inject.length;
    injList.replaceChildren(
      ...draft.inject.map((path, i) => {
        const row = document.createElement("span");
        row.className = "injchip";
        row.innerHTML = `<span></span><button class="ico sm">${icon("x", 12)}</button>`;
        row.children[0].textContent = path.split("/").pop() ?? path;
        row.children[0].setAttribute("title", path);
        row.children[1].addEventListener("click", () => {
          draft.inject.splice(i, 1);
          drawInject();
        });
        return row;
      }),
    );
  };
  const addFiles = (paths: string[]) => {
    if (!capabilitiesOf(draft.agent).attachments) return;
    for (const path of paths) if (path && !draft.inject.includes(path)) draft.inject.push(path);
    drawInject();
  };
  // Pasted screenshots and copied files become attachments, like a drop on the launcher.
  prompt.addEventListener("paste", (e) => {
    if (capabilitiesOf(draft.agent).attachments) pasteFiles(e, addFiles, (error) => console.warn("paste", error));
  });
  $("d-add").addEventListener("click", async () => {
    const picked = await open({ multiple: true, title: t("launcher.attach.dialog") });
    addFiles(Array.isArray(picked) ? picked : picked ? [picked] : []);
  });
  takeFiles = {
    put: addFiles,
    wait: () => {
      receiving++;
      drawHint();
      return () => { receiving--; drawHint(); };
    },
  };

  const hide = () => {
    forgetMcp();
    forgetPlugins();
    takeFiles = null;
    veil.replaceChildren();
    veil.hidden = true;
  };
  const submit = () => {
    if (taken || receiving) return;
    // An empty branch tells the backend to use the repository's current checkout.
    if (!draft.newBranch) draft.branch = "";
    draft.prompt = seed ? issueBlock(seed, prompt.value) : prompt.value;
    // Derive the title from the issue, then prompt, then branch.
    draft.title = draft.title || summarize(prompt.value) || draft.branch || projectName();
    hide();
    go(draft);
  };

  $("d-go").addEventListener("click", submit);
  // Enter creates; Shift-Enter inserts a newline.
  prompt.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  });
  // Close only on backdrop mousedown, not when a text-selection drag ends there. Avoid returning false from the handler because that prevents normal field focus.
  veil.onmousedown = (e) => {
    if (e.target === veil) hide();
  };

  veil.replaceChildren(sheet);
  veil.hidden = false;
  prompt.focus();
}

type Row = { id: string; label: string; sub?: string; glyph?: string; run: () => void };

/// Share searchable branch/issue lists with keyboard navigation and outside-click dismissal. Checked rows identify the current choice; optional secondary labels show identifiers.
function picker(o: {
  btn: HTMLButtonElement;
  el: HTMLElement;
  placeholder: string;
  rows: () => Row[];
  none: () => string;
  current: () => string;
  /// Return focus to the prompt after selection or dismissal.
  after: () => void;
}) {
  const { btn, el } = o;
  el.innerHTML = `<label class="pfind">${icon("search", 14)}<input spellcheck="false" /></label><div class="plist"></div>`;
  const find = el.querySelector("input")!;
  find.placeholder = o.placeholder;
  const list = el.querySelector<HTMLElement>(".plist")!;
  let marked = 0;

  const close = () => {
    el.hidden = true;
    btn.classList.remove("open");
  };
  const choose = (row: Row) => {
    row.run();
    close();
    o.after();
  };
  const draw = () => {
    const q = find.value.trim().toLowerCase();
    const hits = o.rows().filter((r) => `${r.sub ?? ""} ${r.label}`.toLowerCase().includes(q));
    marked = Math.min(marked, Math.max(hits.length - 1, 0));
    list.replaceChildren(
      ...hits.slice(0, 300).map((row, i) => {
        const b = document.createElement("button");
        b.className = "prow" + (i === marked ? " on" : "");
        b.innerHTML =
          `<span class="pc">${row.id === o.current() ? icon("check", 14) : (row.glyph ?? "")}</span>` +
          (row.sub === undefined ? "" : `<span class="iid"></span>`) +
          `<span></span>`;
        if (row.sub !== undefined) b.children[1].textContent = row.sub;
        b.lastElementChild!.textContent = row.label;
        b.addEventListener("mousemove", () => {
          if (marked === i) return;
          marked = i;
          [...list.children].forEach((c, ci) => c.classList.toggle("on", ci === i));
        });
        b.addEventListener("click", () => choose(row));
        return b;
      }),
    );
    if (!hits.length) {
      const none = document.createElement("div");
      none.className = "none";
      none.textContent = o.none();
      list.append(none);
    }
    list.querySelector(".prow.on")?.scrollIntoView({ block: "nearest" });
  };
  const open = () => {
    marked = Math.max(o.rows().findIndex((r) => r.id === o.current()), 0);
    find.value = "";
    // Keep the anchored picker within the sheet bounds and above its footer.
    el.hidden = false;
    const sheet = el.offsetParent as HTMLElement;
    const edge = 12;
    const width = Math.min(420, sheet.clientWidth - edge * 2);
    el.style.left = `${Math.max(edge, Math.min(btn.offsetLeft, sheet.clientWidth - width - edge))}px`;
    el.style.width = `${width}px`;
    const foot = sheet.querySelector<HTMLElement>(".sheetbar")!;
    el.style.maxHeight = `${Math.min(320, foot.offsetTop - el.offsetTop)}px`;
    btn.classList.add("open");
    draw();
    find.focus();
  };

  btn.addEventListener("click", () => (el.hidden ? open() : close()));
  find.addEventListener("input", () => {
    marked = 0;
    draw();
  });
  find.addEventListener("keydown", (e) => {
    const rows = [...list.querySelectorAll<HTMLElement>(".prow")];
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      marked = Math.min(Math.max(marked + (e.key === "ArrowDown" ? 1 : -1), 0), rows.length - 1);
      rows.forEach((r, i) => r.classList.toggle("on", i === marked));
      rows[marked]?.scrollIntoView({ block: "nearest" });
    }
    if (e.key === "Enter") {
      e.preventDefault();
      rows[marked]?.click();
    }
    // Escape closes the picker first; a second Escape closes the launcher.
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
      o.after();
    }
  });

  return { open, close, draw, isOpen: () => !el.hidden, contains: (n: Node) => el.contains(n) || btn.contains(n) };
}

/* Defaults managed by Settings. */

/// Filter default MCP selection against the current registry; null preserves CLI inheritance.
export function defaultMcp(): string[] | null {
  return storedList(MCP_KEY, mcp.known);
}

export function setDefaultMcp(ids: string[] | null) {
  store(MCP_KEY, ids);
}

/// Filter default plugins using the same inheritance rules as MCP.
export function defaultPlugins(): string[] | null {
  return storedList(PLUGIN_KEY, plugins.known);
}

export function setDefaultPlugins(ids: string[] | null) {
  store(PLUGIN_KEY, ids);
}

function storedList(key: string, known: (id: string) => boolean): string[] | null {
  const saved = localStorage.getItem(key);
  if (saved === null) return null;
  try {
    const ids = JSON.parse(saved) as string[];
    return Array.isArray(ids) ? ids.filter(known) : null;
  } catch {
    return null;
  }
}

/// Null removes the explicit selection and restores CLI defaults.
function store(key: string, ids: string[] | null) {
  if (ids === null) localStorage.removeItem(key);
  else localStorage.setItem(key, JSON.stringify(ids));
}

/// Validate the saved model against installed catalogs; fall back to the first model when its alias or provider is unavailable.
export function defaultModel(): string {
  const saved = localStorage.getItem(MODEL_KEY) ?? "";
  const provider = providerOfModel(saved);
  const known = descriptors().some(
    (candidate) => candidate.id === provider && candidate.installed && isKnownModel(candidate.id, saved),
  );
  return known ? saved : fallbackModel();
}

export function setDefaultModel(id: string) {
  localStorage.setItem(MODEL_KEY, id);
}

/// Clamp the shared default effort to the selected model's supported ladder.
export function defaultEffort(model: string): string {
  const saved = localStorage.getItem(EFFORT_KEY) ?? "";
  return fitsEffort(model, EFFORTS.some(([id]) => id === saved) ? saved : "high");
}

export function setDefaultEffort(id: string) {
  localStorage.setItem(EFFORT_KEY, id);
}

/// Expose the model effort ladder to Settings and other controls outside the launcher.
export const effortLadder = ladderOf;

/// Prepend the complete originating issue to user instructions in the initial prompt.
export function issueBlock(issue: Issue, extra: string): string {
  const head = [t("launcher.issueBlock", { id: issue.identifier, title: issue.title }), issue.url];
  const body = issue.description?.trim();
  const parts = [head.join("\n"), body, extra.trim() ? `---\n\n${extra.trim()}` : ""];
  return parts.filter(Boolean).join("\n\n");
}

function summarize(prompt: string) {
  const line = prompt.trim().split("\n")[0]?.trim() ?? "";
  return line.length > 46 ? line.slice(0, 45) + "…" : line;
}
