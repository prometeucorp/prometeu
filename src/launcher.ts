import * as accounts from "./accounts";
import { button, checkbox, dropdown, field, input } from "./ui";
export type { Group } from "./ui";
import { open } from "@tauri-apps/plugin-dialog";
import { capabilitiesOf, catalogOf, descriptor, effortsOf, isKnownModel, nativeEffort, onCatalogChange } from "./agents";
import { choiceLabel, defaultChoice, defaultEffort, effortStep, fitsEffort } from "./model-choice";
import { openModelPicker, openEffortPicker } from "./model-picker";
import { freshBranch } from "./branch";
import { avatar, icon } from "./icons";
import { paint, t } from "./i18n";
import * as issues from "./issues";
import * as kickoff from "./kickoff";
import * as skills from "./skills";
import * as mcp from "./mcp";
import * as plugins from "./plugins";
import * as menu from "./menu";
import { invoke } from "./ipc";
import { pasteFiles } from "./paste";
import { reviewControls } from "./context-review-view";
import type { ReviewContext } from "./context-review";
import { h, template } from "./util";
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
  /// With a worktree, choose whether to create a branch or use an existing one. Without a worktree, false keeps the clone's checkout.
  newBranch: boolean;
  /// Selected local or remote ref when using an existing branch.
  source?: string;
  title: string;
  stage: string;
  prompt: string;
  inject: string[];
  /// An originating Linear issue supplies the workspace title, branch, and initial prompt.
  issue: IssueRef | null;
  /// Keep provider identity explicit because model IDs can overlap.
  agent: ProviderId;
  /// Empty model inherits the selected provider’s native default.
  model: string;
  /// Empty effort inherits the provider default.
  effort: string;
  /// Start only the first conversation in plan mode until its plan is approved.
  plan: boolean;
  /// Null MCP selection preserves inherited CLI behavior; do not silently override existing user configuration.
  mcp: string[] | null;
  /// Null plugin selection preserves inherited CLI behavior, following the MCP rule.
  plugins: string[] | null;
  /// The "Start with" skill as `<package>/<skill>`; empty starts from the prompt alone (ADR 0057).
  kickoff: string;
};


/// Workspace agents use unattended permissions; no launcher toggle changes that behavior.

/// Persist worktree preference across launches. Plan mode belongs to one task and is not remembered.
const WORKTREE_KEY = "prometeu:worktree";
const BRANCH_KEY = "prometeu:branch-nova";

/// Model, effort, MCP, and plugin defaults are explicit Settings choices. Launcher changes affect only the new workspace. Retain existing storage keys for compatibility with older remembered choices.
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

  const initialChoice = defaultChoice();
  let needsChoice = initialChoice === null;
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
    agent: initialChoice?.agent ?? "claude",
    model: initialChoice?.model ?? "",
    effort: "",
    plan: false,
    mcp: defaultMcp(),
    plugins: defaultPlugins(),
    kickoff: "",
  };
  let generatedBranch = draft.branch;
  let existingRef = "";
  const selectedBranch = () => existingRef && (localBranches.includes(existingRef) ? existingRef : existingRef.slice(existingRef.indexOf("/") + 1));
  draft.effort = defaultEffort(draft);
  const conformCapabilities = () => {
    const capabilities = capabilitiesOf(draft.agent);
    if (!capabilities.initialPlanMode) draft.plan = false;
    if (!capabilities.workspaceMcpSelection) draft.mcp = null;
    if (!capabilities.workspacePluginSelection) { draft.plugins = null; draft.kickoff = ""; }
    if (!capabilities.attachments) draft.inject = [];
  };
  conformCapabilities();

  const sheet = document.createElement("div");
  sheet.className = "sheet launcher";
  sheet.setAttribute("role", "dialog");
  sheet.setAttribute("aria-labelledby", "d-title");
  sheet.innerHTML = `
    <header class="launcher-heading">
      <div><h1 id="d-title" data-t="project.newWorkspace"></h1><p data-t="launcher.subtitle"></p></div>
      <span id="d-avatar"></span>
    </header>
    <div class="picker" id="d-picker" hidden></div>
    <div class="picker" id="d-ipicker" hidden></div>
    <div class="picker" id="d-kpicker" hidden></div>
    <div class="launcher-scroll"><div class="launcher-body">
      <section class="launcher-brief">
        <h2><label for="d-prompt" data-t="launcher.prompt"></label></h2>
        <div class="launcher-prompt" id="d-prompt-box">
          <div class="attach" id="d-inj" hidden></div>
          <div class="launcher-attachments" id="d-attachments"></div>
        </div>
        <div class="launcher-review" id="d-review" hidden></div>
        <section class="launcher-repository" aria-labelledby="d-repository-title">
          <h2 id="d-repository-title" data-t="git.repository"></h2>
          <div class="launcher-fields" id="d-repository-fields"></div>
          <div class="launcher-switches" id="d-switches"></div>
          <p class="ui-hint" id="d-repo-hint"></p>
          <div id="d-issue-field"></div>
          <div class="attach" id="d-issue" hidden></div>
          <div id="d-extra-repositories"></div>
          <div class="attach" id="d-repos" hidden></div>
        </section>
      </section>
      <aside class="launcher-settings" aria-labelledby="d-settings-title">
        <h2 id="d-settings-title" data-t="launcher.agentTools"></h2>
        <div id="d-model-field"></div><div id="d-effort-field"></div><div id="d-plan-field"></div>
        <div id="d-account-field"></div>
        <section class="launcher-tools" id="d-tools" aria-labelledby="d-tools-title">
          <h3 id="d-tools-title" data-t="launcher.tools"></h3>
          <div id="d-mcp-field"></div><div id="d-plugins-field"></div>
        </section>
      </aside>
    </div></div>
    <footer class="sheetbar launcher-footer">
      <span class="hint" id="d-hint" role="status"></span>
      <div class="launcher-actions" id="d-actions"></div>
    </footer>`;
  paint(sheet);

  const $ = <T extends HTMLElement>(id: string) => sheet.querySelector(`#${id}`) as T;
  const pick = (id: string) => {
    const control = button("");
    control.id = id;
    control.className = "ui-select pick";
    control.append(h("span", ""));
    control.insertAdjacentHTML("beforeend", icon("chevron-down", 12));
    return control;
  };
  const projectButton = pick("d-project");
  const baseButton = pick("d-base");
  baseButton.querySelector("span")!.id = "d-basename";
  const projectField = field(t("launcher.project"), projectButton);
  const branchField = field(t("launcher.base.label"), baseButton);
  $("d-repository-fields").append(projectField, branchField);
  $("d-issue-field").append(field(t("launcher.issue.label"), pick("d-issuebtn")));
  $("d-model-field").append(field(t("actions.model"), pick("d-model")));
  const effortButton = pick("d-effort");
  effortButton.classList.add("effort");
  effortButton.querySelector("span")!.className = "el";
  effortButton.insertAdjacentHTML("afterbegin", '<span class="bars" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i></span>');
  $("d-effort-field").append(field(t("models.effort"), effortButton));
  $("d-mcp-field").append(field(t("launcher.mcp"), pick("d-mcp")));
  $("d-plugins-field").append(field(t("launcher.plugins"), pick("d-plugins")));
  const worktree = checkbox(t("launcher.worktree"), draft.worktree);
  worktree.control.id = "d-wt";
  const newBranch = checkbox(t("launcher.newBranch"), draft.newBranch);
  newBranch.control.id = "d-nb";
  $("d-switches").append(worktree.label, newBranch.label);
  const planMode = checkbox(t("launcher.plan.start"), draft.plan);
  planMode.control.id = "d-plan";
  $("d-plan-field").append(planMode.label);
  const moreButton = button(t("launcher.addRepo.label"), undefined, "ghost");
  moreButton.id = "d-more";
  moreButton.insertAdjacentHTML("afterbegin", icon("plus", 14));
  $("d-extra-repositories").append(moreButton);
  const prompt = input("", true);
  prompt.id = "d-prompt";
  $("d-prompt-box").prepend(prompt);
  const attach = button(t("launcher.attach.label"), undefined, "ghost");
  attach.id = "d-add";
  attach.title = t("launcher.attach");
  attach.insertAdjacentHTML("afterbegin", icon("paperclip", 14));
  // The starting skill sits next to the prompt it opens, sharing the attachment row.
  const kickoffBtn = button("", undefined, "ghost");
  kickoffBtn.id = "d-kickoff";
  kickoffBtn.classList.add("launcher-kickoff");
  kickoffBtn.insertAdjacentHTML("afterbegin", icon("sparkles", 14));
  kickoffBtn.append(h("span", ""));
  $("d-attachments").append(attach, kickoffBtn);
  const cancel = button(t("account.cancel"), () => hide(), "ghost");
  // Optional missing-context review; created after the draft helpers below exist.
  let review: ReturnType<typeof reviewControls> | null = null;
  const create = button(t("launcher.go"), undefined, "pri");
  create.id = "d-go";
  create.append(h("kbd", "", "↵"));
  $("d-actions").append(cancel, create);
  const hint = $("d-hint");

  const nameOf = (id: string) => board.projects.find((p) => p.id === id)?.name ?? "";
  const projectName = () => nameOf(draft.project);
  const choiceProblem = () => {
    if (needsChoice) return t("models.choose");
    if (!descriptor(draft.agent).installed) return t("models.unavailable");
    const pending = ["idle", "loading"].includes(catalogOf(draft.agent).status);
    if (draft.model && !isKnownModel(draft.agent, draft.model)) return t(pending ? "models.loading" : "models.unavailable");
    if (draft.effort && !effortsOf(draft.agent, draft.model).includes(nativeEffort(draft.agent, draft.effort))) {
      return pending ? t("models.loading") : t("models.unavailableEffort", { effort: draft.effort });
    }
    return "";
  };
  // Disable creation while another workspace owns the branch at a conflicting path, preserving the typed request.
  let taken: Workspace | null = null;
  let receiving = 0;
  const drawHint = () => {
    $("d-avatar").innerHTML = avatar(projectName());
    const from = draft.newBranch && draft.base ? ` ← ${draft.base}` : "";
    const onde = !draft.newBranch && !draft.worktree
      ? t("launcher.hint.here")
      : t(
          draft.extras.length ? "launcher.hint.multi" : draft.worktree ? "launcher.hint.worktree" : "launcher.hint.switch",
          { branch: draft.branch, from },
        );

    const names = [draft.project, ...draft.extras].map(nameOf).join(" + ");
    // Git cannot check out one branch at two paths. Linear can suggest an existing branch whose destination changes with the selected repository set.
    taken =
      draft.worktree && !!draft.branch
        ? branchTaken(board, [draft.project, ...draft.extras], draft.branch)
        : null;
    const aviso = taken ? t("launcher.hint.taken", { ws: taken.title }) :
      draft.worktree && !draft.newBranch && !draft.branch ? t("launcher.branch.required") : choiceProblem();
    hint.classList.toggle("bad", !!aviso);
    hint.title = aviso || `${names} · ${onde}`;
    hint.textContent = aviso || onde;
    $<HTMLButtonElement>("d-go").disabled = !!aviso || receiving > 0 || needsChoice;
    review?.update();
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
          draft.branch = generatedBranch;
          drawExtras();
          drawSwitches();
          prompt.focus();
        },
      })),
    );
  });

  /* Worktree and branch controls. */

  // Support a new or existing branch in a worktree, a new branch in the clone, or the current clone unchanged.
  const wt = worktree.control;
  const nb = newBranch.control;

  // Disable branch/worktree controls for non-Git directories after reference discovery confirms their status.
  let isGit = true;

  const drawSwitches = () => {
    for (const [el, on] of [
      [wt, draft.worktree],
      [nb, draft.newBranch],
    ] as const) {
      el.checked = on;
    }
    nb.disabled = !!draft.extras.length || !isGit;
    nb.title = t(!isGit ? "launcher.noGit" : draft.extras.length ? "launcher.nb.locked" : "launcher.nb.off");
    wt.disabled = !!git || draft.extras.length > 0 || !isGit;
    wt.title = t(!isGit ? "launcher.noGit" : draft.extras.length ? "launcher.wt.locked" : draft.worktree ? "launcher.wt.on" : "launcher.wt.off");
    $("d-repo-hint").textContent = t(!isGit ? "launcher.noGit" : draft.worktree ? "launcher.wt.on" : draft.newBranch ? "launcher.wt.off" : "launcher.nb.off");
    const branchLabel = t(draft.worktree && !draft.newBranch ? "launcher.branch.label" : "launcher.base.label");
    branchField.querySelector("label")!.textContent = branchLabel;
    baseBtn.setAttribute("aria-label", branchLabel);
    $("d-picker").querySelector("input")!.placeholder = t(draft.worktree && !draft.newBranch ? "launcher.branch.pick" : "launcher.base.pick");
    baseBtn.disabled = (!draft.newBranch && !draft.worktree) || !branches.length;
    baseName.textContent = draft.worktree && !draft.newBranch ? existingRef || t("launcher.branch.pick") : draft.base || t("launcher.base.none");
    baseBtn.classList.toggle("empty", draft.worktree && !draft.newBranch ? !existingRef : !draft.base);
    basePick.close();
    drawHint();
  };

  wt.addEventListener("change", () => {
    draft.worktree = wt.checked;
    draft.branch = draft.worktree && !draft.newBranch ? selectedBranch() : generatedBranch;
    localStorage.setItem(WORKTREE_KEY, draft.worktree ? "1" : "0");
    localStorage.setItem(BRANCH_KEY, draft.newBranch ? "1" : "0");
    drawSwitches();
  });
  nb.addEventListener("change", () => {
    draft.newBranch = nb.checked;
    draft.branch = draft.newBranch ? generatedBranch : selectedBranch();
    localStorage.setItem(BRANCH_KEY, draft.newBranch ? "1" : "0");
    drawSwitches();
  });

  /* Model, effort, and plan controls. */

  const accountButton = button("", () => accounts.openPicker(draft.agent), "ghost");
  accountButton.id = "d-account";
  accountButton.classList.add("launcher-account");
  const accountName = h("span", "launcher-account-name");
  accountButton.append(accountName, h("span", "launcher-account-change", t("launcher.account.change")));
  const accountField = field(t("launcher.account.label"), accountButton);
  const accountMeta = h("p", "ui-hint");
  accountField.append(accountMeta);
  $("d-account-field").append(accountField);
  const drawAccount = () => {
    const account = accounts.selected(draft.agent);
    accountName.textContent = account ? accounts.accountName(account) : t("launcher.account.select");
    accountMeta.textContent = account ? [account.plan, t("launcher.account.active")].filter(Boolean).join(" · ") : "";
    accountMeta.hidden = !account;
    accountButton.title = t(account ? "account.use" : "launcher.account.required");
  };
  const forgetAccounts = accounts.onChange(drawAccount);
  drawAccount();

  // Return focus to the prompt after changing secondary launch choices.
  const drawAttach = () => {
    const supported = capabilitiesOf(draft.agent).attachments;
    $<HTMLButtonElement>("d-add").hidden = !supported;
    if (!supported) {
      $("d-inj").hidden = true;
      $("d-inj").replaceChildren();
    }
  };
  const modelButton = $<HTMLButtonElement>("d-model");
  let effortAdjusted = false;
  const drawModel = () => {
    modelButton.querySelector("span")!.textContent = needsChoice ? t("models.choose") : choiceLabel(draft);
  };
  modelButton.addEventListener("click", () => openModelPicker(modelButton, {
    current: draft,
    select: choice => {
      const previousEffort = draft.effort;
      Object.assign(draft, choice);
      needsChoice = false;
      conformCapabilities();
      draft.effort = fitsEffort(draft.model, draft.effort, draft.agent);
      effortAdjusted = previousEffort !== draft.effort;
      drawModel(); drawEffort(); drawPlan(); drawMcp(); drawPlugins(); drawKickoff(); drawAttach(); drawAccount(); drawHint();
      prompt.focus();
    },
  }));
  const effort = $<HTMLButtonElement>("d-effort");
  const drawEffort = () => {
    const step = effortStep(draft.model, draft.effort, draft.agent);
    effort.hidden = !step || needsChoice;
    $("d-effort-field").hidden = effort.hidden;
    if (step) {
      effort.querySelector(".el")!.textContent = step.label;
      effort.querySelectorAll(".bars i").forEach((bar, n) => bar.classList.toggle("lit", n <= step.step));
    }
    effort.classList.toggle("ultra", draft.effort === "ultracode");
    effort.title = effortAdjusted ? t("models.effortAdjusted") : t("launcher.effort.title");
    modelButton.title = effortAdjusted ? t("models.effortAdjusted") : t("launcher.model.title");
  };
  effort.addEventListener("click", () => openEffortPicker(effort, draft, draft.effort, value => {
    draft.effort = value;
    effortAdjusted = false;
    drawEffort();
    drawHint();
    prompt.focus();
  }));
  const forgetCatalog = onCatalogChange(() => {
    if (needsChoice) {
      const resolved = defaultChoice();
      if (resolved) {
        Object.assign(draft, resolved);
        draft.effort = defaultEffort(resolved);
        needsChoice = false;
        conformCapabilities();
        drawPlan(); drawMcp(); drawPlugins(); drawKickoff(); drawAttach(); drawAccount(); drawHint();
      }
    }
    drawModel(); drawEffort(); drawHint();
  });
  drawEffort();
  drawModel();

  const plan = planMode.control;
  const drawPlan = () => {
    plan.hidden = !capabilitiesOf(draft.agent).initialPlanMode;
    $("d-plan-field").hidden = plan.hidden;
    plan.checked = draft.plan;
    plan.title = t(draft.plan ? "launcher.plan.on" : "launcher.plan.off");
  };
  plan.addEventListener("change", () => {
    draft.plan = plan.checked;
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
    $("d-mcp-field").hidden = mcpBtn.hidden;
    $("d-tools").hidden = $("d-mcp-field").hidden && $("d-plugins-field").hidden;
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
    $("d-plugins-field").hidden = plugBtn.hidden;
    $("d-tools").hidden = $("d-mcp-field").hidden && $("d-plugins-field").hidden;
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

  /* Starting skill. */

  // Offer the skills of the standalone hub and of installed plugins. The last explicit choice is a
  // per-installation preference; a skill that is no longer installed falls back to none.
  // The explicit choice lives apart from the draft so a provider without skills only suppresses it.
  let chosenKickoff: string | null = null;
  const kickoffEntries = () => kickoff.catalog(skills.list(), kickoff.pluginSkills());
  const drawKickoff = () => {
    const entries = kickoffEntries();
    const supported = capabilitiesOf(draft.agent).workspacePluginSelection;
    draft.kickoff = kickoff.effectiveKickoff(chosenKickoff, entries, supported);
    const chosen = entries.find((entry) => entry.id === draft.kickoff);
    kickoffBtn.querySelector("span")!.textContent = chosen ? t("launcher.kickoff.chosen", { skill: chosen.name }) : t("launcher.kickoff.label");
    kickoffBtn.classList.toggle("on", !!chosen);
    kickoffBtn.title = chosen
      ? [chosen.plugin ?? t("launcher.kickoff.standalone"), chosen.description].filter(Boolean).join(" · ")
      : t("launcher.kickoff.hint");
    kickoffBtn.hidden = !supported || !entries.length;
    if (kickoffBtn.hidden) kickoffPick?.close();
  };
  const kickoffPick = picker({
    btn: kickoffBtn,
    el: $("d-kpicker"),
    placeholder: t("launcher.kickoff.pick"),
    rows: () => [
      { id: "", label: t("launcher.kickoff.none"), run: () => chooseKickoff("") },
      ...kickoffEntries().map((entry) => ({
        id: entry.id,
        label: entry.description ? `${entry.name} — ${entry.description}` : entry.name,
        sub: entry.plugin ?? t("launcher.kickoff.standalone"),
        run: () => chooseKickoff(entry.id),
      })),
    ],
    none: () => t("launcher.kickoff.noMatch"),
    current: () => draft.kickoff,
    after: () => prompt.focus(),
  });
  const chooseKickoff = (id: string) => {
    chosenKickoff = id;
    kickoff.rememberKickoff(id);
    drawKickoff();
  };
  const forgetSkills = skills.onChange(drawKickoff);
  drawKickoff();
  void kickoff.refresh().then(drawKickoff).catch((error) => console.warn("plugin_skills", error));

  /* Branch base. */

  // Changing projects refreshes available bases because repository references differ.
  const baseBtn = $<HTMLButtonElement>("d-base");
  const baseName = $("d-basename");
  let branches: string[] = [];
  let localBranches: string[] = [];
  const selectable = () => branches.filter(name => localBranches.includes(name) || !localBranches.includes(name.slice(name.indexOf("/") + 1)));

  const setBase = (name: string) => {
    draft.base = name;
    baseName.textContent = draft.worktree && !draft.newBranch ? existingRef || t("launcher.branch.pick") : name || t("launcher.base.none");
    baseBtn.classList.toggle("empty", !name);
    drawHint();
  };

  const basePick = picker({
    btn: baseBtn,
    el: $("d-picker"),
    placeholder: t("launcher.base.pick"),
    rows: () => (draft.worktree && !draft.newBranch ? selectable() : branches).map((name) => ({ id: name, label: name, run: () => {
      if (draft.worktree && !draft.newBranch) {
        existingRef = name;
        draft.branch = selectedBranch();
        draft.source = name;
        baseName.textContent = name;
        baseBtn.classList.remove("empty");
        drawHint();
      } else setBase(name);
    } })),
    none: () => t(branches.length ? "launcher.base.noMatch" : "launcher.base.empty"),
    current: () => draft.worktree && !draft.newBranch ? existingRef : draft.base,
    after: () => prompt.focus(),
  });

  const loadBranches = async () => {
    const loadingProject = draft.project;
    const fromGit = loadingProject === project ? git : undefined;
    branches = [];
    localBranches = [];
    existingRef = "";
    draft.source = undefined;
    if (draft.worktree && !draft.newBranch) draft.branch = "";
    isGit = true;
    baseBtn.disabled = true;
    baseName.textContent = t("launcher.loading");
    drawHint();
    try {
      const got = await invoke("list_branches", { project: draft.project });
      if (draft.project !== loadingProject) return;
      branches = got.all;
      localBranches = got.local ?? [];
      isGit = got.git;
      // A non-Git directory opens directly without branch operations.
      if (!isGit) {
        draft.worktree = false;
        draft.newBranch = false;
      }
      drawSwitches();
      // Refresh generated names after loading repository references while preserving explicit Git or issue selections.
      if (!seed) generatedBranch = fromGit?.branch || freshBranch(branches);
      draft.branch = draft.worktree && !draft.newBranch ? selectedBranch() : generatedBranch;
      setBase(fromGit?.base ?? got.default);
    } catch {
      if (draft.project !== loadingProject) return;
      // A newly initialized repository without references branches from its current HEAD.
      setBase(fromGit?.base ?? "");
      drawSwitches();
    }
    baseBtn.disabled = (!draft.newBranch && !draft.worktree) || !branches.length;
  };

  /* Originating issue. */

  // Show the selected issue as part of the first prompt; its chip supplies the title/branch and can be removed.
  const issueBox = $("d-issue");
  const issueBtn = $<HTMLButtonElement>("d-issuebtn");
  const setSeed = (issue: Issue | undefined) => {
    seed = issue;
    draft.issue = issue ? { id: issue.id, identifier: issue.identifier, title: issue.title, url: issue.url } : null;
    generatedBranch = (draft.project === project ? git?.branch : undefined) || issue?.branch_name || freshBranch(branches);
    draft.branch = draft.worktree && !draft.newBranch ? selectedBranch() : generatedBranch;
    draft.title = issue ? `${issue.identifier} · ${issue.title}` : "";
    prompt.placeholder = t(issue ? "launcher.prompt.issue" : "launcher.prompt");
    issueBtn.querySelector("span")!.textContent = issue?.identifier ?? t("launcher.issue.none");
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
    for (const p of [basePick, issuePick, kickoffPick]) {
      if (p.isOpen() && !p.contains(e.target as Node)) p.close();
    }
  });
  // Dismiss anchored menus on a scroll gesture, not on the browser scrolling a focused trigger into view.
  for (const event of ["wheel", "touchmove"]) {
    sheet.querySelector(".launcher-scroll")!.addEventListener(event, () => {
      basePick.close(); issuePick.close(); kickoffPick.close(); menu.close();
    }, { passive: true });
  }

  // Keep attachments visible between the prompt and footer because they belong to the first message.
  const injList = $("d-inj");
  const drawInject = () => {
    review?.update();
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

  // Review input: the draft, the complete originating issue and the project metadata the launcher
  // actually has. Attachments are counted as uninspected; their paths and contents are not sent.
  const reviewContext = (): ReviewContext => ({
    draft: prompt.value,
    issue: seed
      ? { identifier: seed.identifier, title: seed.title, description: seed.description, state: seed.state.name,
          labels: seed.labels.map(label => label.name), team: seed.team, project: seed.project }
      : null,
    project: { name: projectName(), repositories: draft.extras.map(nameOf), base: draft.newBranch ? draft.base : "" },
    attachments: draft.inject.length,
  });
  review = reviewControls({ panel: $("d-review"), prompt, context: reviewContext, edited: () => review?.update() });
  $("d-actions").prepend(review.trigger);
  prompt.addEventListener("input", () => review?.update());

  const hide = () => {
    review?.close();
    menu.close();
    forgetCatalog();
    forgetAccounts();
    forgetMcp();
    forgetPlugins();
    forgetSkills();
    takeFiles = null;
    veil.replaceChildren();
    veil.hidden = true;
  };
  sheet.addEventListener("keydown", event => {
    if (event.key === "Escape" && !document.querySelector("dialog[open]")) {
      event.stopPropagation();
      hide();
    }
  });
  const submit = () => {
    if (taken || receiving || choiceProblem()) return;
    if (!accounts.selected(draft.agent)) { accounts.openPicker(draft.agent); return; }
    // An empty branch tells the backend to use the repository's current checkout.
    if (!draft.newBranch && !draft.worktree) draft.branch = "";
    if (!draft.newBranch && draft.worktree && !draft.branch) return;
    draft.prompt = seed ? issueBlock(seed, prompt.value) : prompt.value;
    // Derive the title from the issue, then prompt, then branch.
    draft.title = draft.title || summarize(prompt.value) || draft.branch || projectName();
    hide();
    go(draft);
  };

  $("d-go").addEventListener("click", submit);
  // Enter creates; Shift-Enter inserts a newline.
  prompt.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
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
    const anchor = btn.getBoundingClientRect();
    const bounds = sheet.getBoundingClientRect();
    el.style.left = `${Math.max(edge, Math.min(anchor.left - bounds.left, sheet.clientWidth - width - edge))}px`;
    el.style.width = `${width}px`;
    const foot = sheet.querySelector<HTMLElement>(".sheetbar")!;
    const top = sheet.querySelector<HTMLElement>(".launcher-heading")!.offsetHeight + edge;
    const height = Math.min(320, foot.offsetTop - top - edge);
    el.style.top = `${Math.max(top, Math.min(anchor.bottom - bounds.top + 4, foot.offsetTop - height - edge))}px`;
    el.style.maxHeight = `${height}px`;
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
