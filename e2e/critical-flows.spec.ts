import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Board, Status } from "../src/types";

async function boot(page: Page) {
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await expect(page.locator("#railbody .navitem.sub").first()).toBeVisible();
}

async function openWorkspace(page: Page, title: string) {
  await page.locator("#railbody .navitem.sub .lbl").getByText(title, { exact: true }).click();
  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#crumb")).toContainText(title);
}

async function bootTeam(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem("mock:team", JSON.stringify({
      relay: null,
      team: "fakeTeam",
      secret: "fakeSecret123456789",
      member: "eu_mock",
      credential: "c".repeat(43),
      name: "You",
    }));
  });
  await boot(page);
  await expect(page.locator("#railbody .navitem.mentions")).toBeVisible();
}

test("sharing adopts a new device key without requesting review", { tag: "@webkit" }, async ({ page }) => {
  await bootTeam(page);
  const settings = async () => {
    await page.locator("#settings").click();
    await page.locator(".setnavitem", { hasText: "Work and team" }).click();
  };
  await settings();
  await expect(page.locator("#settingsView")).toContainText("end-to-end encrypted");
  // The list names people, never their devices.
  await expect(page.locator("#settingsView .members .mem .nm")).toHaveText(["You (you)", "Marcus Hale", "John Okafor"]);

  // Replacing only the simulated peer's private storage changes that device's identity: a reinstall.
  await page.evaluate(() => {
    for (const key of Object.keys(localStorage)) if (key.startsWith("mock:peer-security:")) localStorage.removeItem(key);
  });
  await page.reload();
  await settings();
  await expect(page.locator("#settingsView .members .mem .nm")).toHaveText(["You (you)", "Marcus Hale", "John Okafor"]);
  await expect(page.locator("#settingsView")).not.toContainText("blocked");
  // Encrypted collaboration keeps flowing under the new key, with nothing to confirm.
  await expect(page.locator("#railbody .navitem.mentions")).toBeVisible();
});

test("comments stay beside the session until resolved", { tag: "@webkit" }, async ({ page }) => {
  await bootTeam(page);

  // Opening the context does not resolve the pending comment.
  await page.locator("#railbody .navitem.mentions").click();
  await page.locator(".inboxrow").click();
  await expect(page.locator("#crumb")).toContainText("Archive completed todos");
  await expect(page.locator("#comments .commentthread")).toBeVisible();
  await expect(page.locator("#chatwrap .commentpin")).toHaveCount(1);
  await expect(page.locator("#railbody .navitem.mentions .n")).toHaveText("1");
  await expect(page.locator("#chatwrap .composer textarea")).toHaveAttribute("placeholder", "Write in Marcus Hale's conversation");

  await page.locator('#tabbar .tab[data-tab="mt2"]').click();
  await expect(page.locator("#comments .commentempty")).toContainText("No open comments");
  await page.locator('#tabbar .tab[data-tab="mt1"]').click();
  await page.locator("#comments .commentcard").first().click();

  await page.locator(".replybox textarea").fill("I agree with the column.");
  await page.locator(".replybox .submit").click();
  await expect(page.locator(".commentmessage")).toHaveCount(2);
  await page.locator(".replybox .resolve").click();
  await expect(page.locator(".threadstate")).toHaveText("Resolved");
  await expect(page.locator("#railbody .navitem.mentions")).toHaveCount(0);
  await expect(page.locator("#chatwrap .commentpin")).toHaveCount(0);

  // Comments use a separate field; the main composer still sends to the agent.
  await page.locator(".threadhead .back").click();
  await page.locator('.commentfilters [data-filter="resolved"]').click();
  await expect(page.locator(".commentcard", { hasText: "Completing a todo now" })).toBeVisible();
  await page.locator('.commentfilters [data-filter="open"]').click();
  await page.locator("#chatwrap .meta .cm").last().click();
  await expect(page.locator(".commentdraft")).toBeVisible();
  await expect(page.locator(".commentdraft .draftquote-text")).not.toBeEmpty();
  await expect(page.locator("#chatwrap .composer textarea")).toHaveAttribute("placeholder", "Write in Marcus Hale's conversation");
  await page.locator(".commentdraft textarea").fill("New question for the team.");
  await page.locator(".commentdraft").evaluate(card => {
    const submit = card.querySelector<HTMLButtonElement>(".submit")!;
    submit.click(); submit.click();
    const area = card.querySelector<HTMLTextAreaElement>("textarea")!;
    area.value = "Draft typed during encryption.";
    area.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect(page.locator(".commentcard", { hasText: "New question for the team." })).toBeVisible();
  await expect(page.locator(".commentcard", { hasText: "New question for the team." })).toHaveCount(1);
  await expect(page.locator(".commentdraft textarea")).toHaveValue("Draft typed during encryption.");
  await expect(page.locator("#chatwrap .commentpin")).toHaveCount(1);
  await expect(page.locator("#chatwrap .note")).toHaveCount(0);
});

test("stream tokens preserve controls, keep painting unfocused and catch up after hiding", { tag: "@webkit" }, async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");
  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", { type: "stream_event", event: { type: "message_start", message: { id: "paint-check" } } });
    mock.line("t1", { type: "stream_event", event: { type: "content_block_start", index: 0, content_block: { type: "text", text: "Initial text" } } });
  });
  const output = page.locator('#chatwrap .turn.bot .md[data-kind="text"]').last();
  await expect(output).toContainText("Initial text");
  const sendIcon = await page.locator("#chatwrap .composer .send svg").elementHandle();
  await page.evaluate(() => {
    (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock.line("t1", {
      type: "stream_event", event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: " and more" } },
    });
  });
  await expect(output).toContainText("Initial text and more");
  expect(await sendIcon!.evaluate(node => node.isConnected)).toBe(true);

  // A visible window without focus, such as one on a second monitor, keeps following the response.
  await page.evaluate(() => {
    Object.defineProperty(document, "hasFocus", { configurable: true, value: () => false });
    window.dispatchEvent(new Event("blur"));
  });
  await page.evaluate(() => {
    (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock.line("t1", {
      type: "stream_event", event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: " while unfocused" } },
    });
  });
  await expect(output).toContainText("while unfocused");
  await page.evaluate(() => {
    delete (document as unknown as { hasFocus?: unknown }).hasFocus;
    window.dispatchEvent(new Event("focus"));
  });

  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, value: true });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(30);
  await page.evaluate(() => {
    (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock.line("t1", {
      type: "stream_event", event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: " while hidden" } },
    });
  });
  await page.waitForTimeout(50);
  await expect(output).not.toContainText("while hidden");
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, value: false });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(output).toContainText("while hidden");
});

test("clicking a project opens clone files without a workspace", async ({ page }) => {
  await boot(page);

  const project = page.locator("#railbody .group", { hasText: "prometeu", hasNotText: "+ njord" });
  await project.locator("span").nth(1).click();

  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#crumb")).toContainText("prometeu");
  // Without a branch, worktree or conversation, no dock or Changes view is available.
  await expect(page.locator("#dock")).toBeHidden();
  await expect(page.locator("#tab-diff")).toBeHidden();
  // The center stays empty until a file is selected.
  await expect(page.locator("#offline")).toBeVisible();
  await expect(page.locator("#offtitle")).toBeEmpty();
  await expect(page.locator("#offbody")).toBeHidden();

  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vpre")).toContainText("Personal finance management");
  await page.locator("#vpreview").click();
  await expect(page.locator("#vread")).toBeVisible();
  await expect(page.locator("#vread h1")).toHaveText("Njord");
  await expect(page.locator("#vread strong")).toHaveText("left");
  await expect(page.locator("#vpreview")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("#vtext")).toBeHidden();
  await page.locator("#vsource").click();
  await expect(page.locator("#vtext")).toBeVisible();

  // Opened files become tabs and share the same add control as workspace tabs.
  await expect(page.locator("#tabbar .tab")).toHaveText(["CLAUDE.md"]);
  await expect(page.locator("#tabbar .tabadd .caret")).toBeVisible();
  await page.locator("#tree .treerow", { hasText: "README.md" }).click();
  await expect(page.locator("#tabbar .tab")).toHaveText(["CLAUDE.md", "README.md"]);
  await expect(page.locator("#tabbar .tab.on")).toHaveText("README.md");
  await page.locator("#tabbar .tab", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vpre")).toContainText("Personal finance management");

  // Editing and saving use the workspace viewer path.
  await page.locator("#vtext").fill("# Directly from the project\n");
  await expect(page.locator("#vsave")).toBeVisible();
  await page.locator("#vsave").click();
  await expect(page.locator("#vsave")).toBeHidden();

  // The dropdown lists actions available from the project.
  await page.locator("#tabbar .tabadd .caret").click();
  await expect(page.locator(".menu .mrow")).toHaveText(["New terminal", "New workspace"]);
  await page.keyboard.press("Escape");

  // The add button opens a terminal in the clone. Setup and Run require a workspace.
  await page.locator("#tabbar .tabadd .ico").first().click();
  const term = page.locator("#tabbar .tab", { hasText: "Terminal" });
  await expect(term).toHaveClass(/on/);
  await expect(page.locator("#termview")).toBeVisible();
  await expect(page.locator("#dock")).toBeHidden();

  // Returning to the file keeps its terminal alive in the background.
  await page.locator("#tabbar .tab", { hasText: "README.md" }).click();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#termview")).toBeHidden();
  await term.hover();
  await term.locator(".tabx").click();
  await expect(page.locator("#tabbar .tab", { hasText: "Terminal" })).toHaveCount(0);

  // Closing every tab restores the empty center.
  await page.locator("#tabbar .tab", { hasText: "CLAUDE.md" }).locator(".tabx").click();
  await page.locator("#tabbar .tab", { hasText: "README.md" }).locator(".tabx").click();
  await expect(page.locator("#viewer")).toBeHidden();
  await expect(page.locator("#offline")).toBeVisible();

  // The chevron still collapses the project list.
  await project.locator(".gc").click();
  await expect(page.locator("#railbody .navitem.sub", { hasText: "Match the Conductor screen" })).toHaveCount(0);
});

test("the sidebar preserves conversation and terminal when leaving a project file", { tag: "@webkit" }, async ({ page }) => {
  await boot(page);
  const project = page.locator("#railbody .group", { hasText: "njord", hasNotText: "+" });
  await project.locator("span").nth(1).click();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toBeVisible();
  await page.locator("#vtext").fill("Clone draft");

  // Entering a workspace leaves project-only mode before any tab or dock redraw.
  await openWorkspace(page, "Hello");
  const conversation = page.locator('#tabbar .tab[data-tab="t1"]');
  const composer = page.locator("#chatwrap .composer textarea");
  await expect(composer).toBeVisible();
  await expect(conversation).toHaveClass(/\bon\b/);
  await expect(page.locator("#viewer")).toBeHidden();

  // The same path belongs to a different root; opening it preserves conversation tabs.
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toBeVisible();
  await expect(page.locator("#vtext")).not.toHaveValue("Clone draft");
  await expect(page.locator("#tabbar .tab[data-tab]")).toHaveCount(2);
  await page.locator("#vtext").fill("Workspace draft");
  await page.locator("#tabbar .tab", { hasText: "CLAUDE.md" }).locator(".tabx").click();
  await expect(composer).toBeVisible();
  await expect(conversation).toHaveClass(/\bon\b/);

  await page.locator("#tabbar .tabadd .caret").click();
  await page.locator(".ui-search-picker-choice", { hasText: "New terminal" }).click();
  await expect(page.locator("#termview")).toBeVisible();
  await expect(conversation).toBeVisible();
  await conversation.click();
  await expect(composer).toBeVisible();
  const terminal = page.locator("#tabbar .tab", { hasText: "Terminal" });
  await terminal.click();
  await terminal.locator(".tabx").click();
  await expect(terminal).toHaveCount(0);
  await expect(composer).toBeVisible();

  // History restores each root's own files and drafts.
  await page.locator("#back").click();
  await expect(page.locator("#vtext")).toHaveValue("Clone draft");
  await expect(page.locator("#tabbar .tab")).toHaveText(["CLAUDE.md"]);
  await page.locator("#fwd").click();
  await expect(composer).toBeVisible();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toHaveValue("Workspace draft");
  await expect(conversation).toBeVisible();

  // Selecting an agent directly also exits project-only mode.
  await project.locator("span").nth(1).click();
  await expect(page.locator("#vtext")).toHaveValue("Clone draft");
  await page.locator('.railagent[data-tab="t2"]').click();
  await expect(composer).toBeVisible();
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "t2");
});

test("removing a project preserves its workspaces", async ({ page }) => {
  await boot(page);

  // The combined project name also matches a search for its second repository.
  const project = page.locator("#railbody .group", { hasText: "njord", hasNotText: "+" });
  await project.hover();
  await project.locator('button[title="Actions for njord"]').click();
  await page.locator(".menu .mrow", { hasText: "Remove project" }).click();

  await expect(project).toHaveCount(0);
  await expect(page.locator("#railbody .group", { hasText: "No project" })).toBeVisible();
  await expect(page.locator("#railbody .navitem.sub", { hasText: "Hello" })).toBeVisible();
});

test("the sidebar lists agents by workspace, tracks status and opens the selected tab", async ({ page }) => {
  await boot(page);
  await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const board = await invoke("load_board") as Board;
    board.workspaces[0].tabs[1].choice = { agent: "codex", model: "gpt-5.6-sol", effort: "high" };
    await invoke("set_stage", { id: "sessao-0929", stage: "Fazendo" });
  });
  const card = page.locator('.railworkspace[data-workspace="sessao-0929"]');
  const first = card.locator('.railagent[data-tab="t1"]');
  const second = card.locator('.railagent[data-tab="t2"]');
  await expect(card.locator(".railagents-toggle")).toHaveText("2 agents");
  await expect(first.locator(".provider image")).toBeVisible();
  await expect(first).toHaveAttribute("title", /Claude Code/);
  await expect(second.locator(".provider path")).toHaveCount(1);
  await expect(second).toHaveAttribute("title", /Codex/);
  // An unnamed tab displays its model on one line.
  await expect(first.locator(".lbl")).toHaveText("Opus (1M context)");
  await expect(second.locator(".lbl")).toHaveText("GPT-5.6-Sol");
  await expect(first.locator(".note")).toHaveCount(0);

  // Tab status follows the board even when its workspace is closed.
  for (const status of ["rodando", "querendo", "desligada", "pronta"] as Status[]) {
    await page.evaluate(async (status) => {
      type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
      const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
      const board = await invoke("load_board") as Board;
      const workspace = board.workspaces[0];
      workspace.tabs[0].status = status;
      await invoke("set_stage", { id: workspace.id, stage: workspace.stage });
    }, status);
    await expect(first.locator(".rail-status")).toHaveAttribute("data-status", status);
    await expect(first.locator(".rail-status")).toHaveCSS("animation-name", status === "rodando" ? "spin" : "none");
    await expect(card.locator(".navitem .rail-status")).toHaveAttribute("data-status", status === "desligada" ? "pronta" : status);
  }
  await expect(first.locator(".rail-status")).toHaveCSS("color", "rgb(95, 191, 115)");
  await expect(card.locator(".navitem .rail-status")).toHaveCSS("color", "rgb(95, 191, 115)");
  // A single agent appears directly in the card without a redundant collapse row.
  await expect(page.locator('.railworkspace[data-workspace="ui-2231"] .railagents-toggle')).toHaveCount(0);
  await expect(page.locator('.railworkspace[data-workspace="ui-2231"] .railagent')).toHaveCount(1);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(page.locator('.railagent[data-tab="t3"] .rail-status')).toHaveCSS("animation-name", "none");

  await second.click();
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "t2");
  await expect(second).toHaveAttribute("aria-current", "true");
  await expect(page.locator("#chatwrap .composer .mdl")).toContainText("GPT-5.6-Sol");
  await page.locator('#tabbar .tab[data-tab="t1"]').click();
  await expect(first).toHaveAttribute("aria-current", "true");
  await expect(second).not.toHaveAttribute("aria-current");

  // The agent row returns from an open file to its selected conversation.
  await page.locator("#tab-files").click();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#viewer")).toBeVisible();
  await first.click();
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(page.locator("#viewer")).toBeHidden();
  await second.click();
  await openWorkspace(page, "Match the Conductor screen");
  await openWorkspace(page, "Hello");
  await expect(second).toHaveAttribute("aria-current", "true");
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "t2");

  // Collapsing does not navigate, and the choice survives redraws and reloads.
  await card.locator(".railagents-toggle").focus();
  await page.keyboard.press("Enter");
  await expect(card.locator(".railagents-toggle")).toHaveAttribute("aria-expanded", "false");
  await expect(first).toBeHidden();
  await expect(page.locator("#wsView")).toBeVisible();
  await page.locator("#railbody .navitem", { hasText: "Desk" }).click();
  await expect(first).toBeHidden();
  await page.reload();
  await expect(card.locator(".railagents-toggle")).toHaveAttribute("aria-expanded", "false");
  await card.locator(".railagents-toggle").click();
  await expect(first).toBeVisible();

  // Before a tab exists, the card opens workspace preparation status.
  await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const board = await invoke("load_board") as Board;
    const workspace = board.workspaces[0];
    workspace.tabs = [];
    workspace.active = null;
    workspace.preparing = true;
    await invoke("set_stage", { id: workspace.id, stage: workspace.stage });
  });
  await expect(card.locator(".railagents-toggle")).toHaveCount(0);
  await expect(card.locator(".rail-status")).toHaveAttribute("aria-label", "preparing");
  await card.locator(".navitem").click();
  await expect(card).toHaveClass(/\bon\b/);
  await expect(page.locator("#wsView")).toBeVisible();
});

test("the sidebar opens remote agents without inventing a provider and shows the offline owner", async ({ page }) => {
  await bootTeam(page);
  const second = page.locator('.railagent[data-tab="mt2"]');
  await expect(second.locator(".provider .avatar")).toBeVisible();
  await expect(second).not.toHaveAttribute("title", /Claude|Codex/);
  await second.click();
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "mt2");
  await expect(second).toHaveAttribute("aria-current", "true");
  await page.locator('#tabbar .tab[data-tab="mt1"]').click();
  await expect(page.locator('.railagent[data-tab="mt1"]')).toHaveAttribute("aria-current", "true");
  await expect(second).not.toHaveAttribute("aria-current");
  await second.click();
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "mt2");
  await expect(second).toHaveAttribute("aria-current", "true");
  await page.evaluate(() => {
    (window as unknown as { mock: { presence: (online: boolean) => void } }).mock.presence(false);
  });
  await expect(page.locator('.railagent[data-tab="mt1"] .rail-status')).toHaveAttribute("data-status", "desligada");
  await expect(second.locator(".rail-status")).toHaveAttribute("aria-label", "stopped");
});

test("switching tabs quickly ignores the previous tab’s delayed snapshot", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  const first = page.locator('#tabbar .tab[data-tab="t1"]');
  const second = page.locator('#tabbar .tab[data-tab="t2"]');
  await expect(first).toHaveClass(/\bon\b/);

  await page.evaluate(() => {
    const mock = (window as unknown as {
      mock: { line: (tab: string, line: unknown) => void };
    }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "E2E_MARKER_T1" } });
    mock.line("t2", { type: "user", message: { role: "user", content: "E2E_MARKER_T2" } });
  });
  await expect(page.locator("#chatwrap .bubble", { hasText: "E2E_MARKER_T1" })).toBeVisible();

  // Deliver t2's snapshot after t1's to expose stale responses during rapid tab switching.
  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = async function (command, args, options) {
      if (command === "chat_snapshot" && args?.session === "t2") {
        await new Promise((resolve) => setTimeout(resolve, 350));
      }
      return original.call(this, command, args, options);
    };
  });

  await second.click();
  await first.click();

  await expect(first).toHaveClass(/\bon\b/);
  await expect(page.locator("#chatwrap .bubble", { hasText: "E2E_MARKER_T1" })).toBeVisible();
  await expect(page.locator("#chatwrap .bubble", { hasText: "E2E_MARKER_T2" })).toHaveCount(0);
  await page.waitForTimeout(450);
  await expect(first).toHaveClass(/\bon\b/);
  await expect(page.locator("#chatwrap .bubble", { hasText: "E2E_MARKER_T2" })).toHaveCount(0);
});

/// Closing the selected conversation must hand the center to the surviving tab without a click.
test("closing the selected conversation selects the remaining tab", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  const first = page.locator('#tabbar .tab[data-tab="t1"]');
  const second = page.locator('#tabbar .tab[data-tab="t2"]');
  await page.evaluate(() => {
    const mock = (window as unknown as {
      mock: { line: (tab: string, line: unknown) => void };
    }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "E2E_CLOSE_T1" } });
  });

  await second.click();
  await expect(second).toHaveClass(/\bon\b/);
  await second.hover();
  await second.locator(".tabx").click();

  await expect(second).toHaveCount(0);
  await expect(first).toHaveClass(/\bon\b/);
  await expect(page.locator("#chatwrap .bubble", { hasText: "E2E_CLOSE_T1" })).toBeVisible();
});

test("failed tool output stays collapsed until expanded", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  await page.evaluate(() => {
    const mock = (window as unknown as {
      mock: { line: (tab: string, line: unknown) => void };
    }).mock;
    mock.line("t1", {
      type: "assistant",
      message: {
        id: "m-error-e2e",
        role: "assistant",
        content: [{ type: "tool_use", id: "tu-error-e2e", name: "Bash", input: { command: "apply_patch" } }],
      },
    });
    mock.line("t1", {
      type: "user",
      message: {
        role: "user",
        content: [{
          type: "tool_result",
          tool_use_id: "tu-error-e2e",
          is_error: true,
          content: "Script failed\nWall time: 0.1 seconds\nOutput:\napply_patch verification failed: snippet not found",
        }],
      },
    });
  });

  const tool = page.locator('#chatwrap .tool[data-tool="tu-error-e2e"]');
  await expect(tool).toHaveClass(/\bbad\b/);
  await expect(tool.locator(".tout")).toBeHidden();

  await tool.locator(".thead").click();
  await expect(tool.locator(".tfail")).toContainText("A step failed");
  await expect(tool.locator(".tout")).toBeHidden();

  await tool.locator(".ttechnical summary").click();
  await expect(tool.locator(".tout")).toContainText("apply_patch verification failed");
});

test("the launcher creates a workspace and follows preparation into the conversation", { tag: "@webkit" }, async ({ page }) => {
  await boot(page);

  await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
  await expect(page.locator("#veil .sheet")).toBeVisible();

  const title = "Workspace created by E2E";
  await page.locator("#d-prompt").fill(title);
  await page.locator("#d-prompt").press("Enter");

  await expect(page.locator("#veil")).toBeHidden();
  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#crumb")).toContainText(title);
  await expect(page.locator("#offline")).toBeVisible();

  await expect(page.locator('#tabbar .tab[data-tab^="t-nova-"]')).toBeVisible({ timeout: 4_000 });
  await expect(page.locator("#offline")).toBeHidden();
  await expect(page.locator("#chatwrap .composer textarea")).toBeVisible();
});

test("the issue list fits the launcher and keeps titles readable", async ({ page }) => {
  await boot(page);

  // Expand the fixture from its current length until the issue popup exceeds both its side and footer
  // boundaries.
  const total = await page.evaluate(async () => {
    const BATCHES = 4;
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = async function (command, args, options) {
      const result = await original.call(this, command, args, options);
      if (command !== "linear_issues") return result;
      const found = result as { issues: Record<string, unknown>[]; fetched_at: number };
      return {
        ...found,
        issues: Array.from({ length: BATCHES }, (_, batch) =>
          found.issues.map((issue) => ({ ...issue, id: `${issue.id}-${batch}` })),
        ).flat(),
      };
    };
    // Connect Linear first; the mock rejects searches while disconnected.
    await internals.invoke("linear_connect");
    const found = (await internals.invoke("linear_issues", { force: false })) as { issues: unknown[] };
    return found.issues.length;
  });
  // The fixture must exceed the available viewport height.
  expect(total).toBeGreaterThanOrEqual(20);

  await expect(page.locator("#railbody .navitem", { hasText: "Issues" }).locator(".n")).toHaveText(String(total));

  await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
  await page.locator("#d-issuebtn").click();
  await expect(page.locator("#d-ipicker .prow")).toHaveCount(total);

  const geometry = await page.evaluate(() => {
    const rect = (selector: string) => document.querySelector<HTMLElement>(selector)!.getBoundingClientRect();
    const picker = rect("#d-ipicker");
    const sheet = rect("#veil .sheet");
    const foot = rect("#veil .sheetbar");
    const id = rect("#d-ipicker .prow .iid");
    const title = rect("#d-ipicker .prow > span:last-child");
    return {
      picker: { left: picker.left, right: picker.right, bottom: picker.bottom },
      sheet: { left: sheet.left, right: sheet.right },
      foot: { top: foot.top },
      title: { width: title.width, gap: title.left - id.right },
    };
  });
  expect(geometry.picker.left).toBeGreaterThanOrEqual(geometry.sheet.left);
  expect(geometry.picker.right).toBeLessThanOrEqual(geometry.sheet.right);
  expect(geometry.picker.bottom).toBeLessThanOrEqual(geometry.foot.top);
  expect(geometry.title.width).toBeGreaterThan(200);
  expect(geometry.title.gap).toBeGreaterThanOrEqual(8);
});

test("launcher controls fit their container with a long base branch", async ({ page }) => {
  await page.setViewportSize({ width: 650, height: 800 });
  await boot(page);

  const branch = "origin/feature/a-branch-name-long-enough-to-require-ellipsis-in-the-control";
  await page.evaluate((longBranch) => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = function (command, args, options) {
      if (command === "list_branches") return Promise.resolve({ all: [longBranch], default: longBranch });
      return original.call(this, command, args, options);
    };
  }, branch);

  await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
  await expect(page.locator("#d-basename")).toHaveText(branch);

  const bounds = await page.locator(".launcher-repository").evaluate((top) => {
    const branchName = top.querySelector<HTMLElement>("#d-basename")!;
    const worktree = top.querySelector<HTMLElement>("#d-wt")!;
    const topRect = top.getBoundingClientRect();
    return {
      topRight: topRect.right,
      worktreeRight: worktree.getBoundingClientRect().right,
      branchWidth: branchName.clientWidth,
      branchContentWidth: branchName.scrollWidth,
    };
  });

  expect(bounds.worktreeRight).toBeLessThanOrEqual(bounds.topRight);
  expect(bounds.branchContentWidth).toBeLessThanOrEqual(bounds.branchWidth);
});

test("sending a question and answering its card returns control to the chat", { tag: "@webkit" }, async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("I have a question for the E2E flow");
  await composer.press("Enter");

  const card = page.locator("#chatwrap .question");
  await expect(card).toBeVisible();
  await expect(card.locator(".qbody")).toContainText("Where should completed items be stored?");
  await card.locator(".qbody .opt").first().click();
  await expect(card.locator(".qbody")).toContainText("Run the migration now?");
  await card.locator(".qbody .opt").first().click();

  const answer = card.locator("button.pri");
  await expect(answer).toBeEnabled();
  await answer.click();

  await expect(card).toHaveCount(0);
  await expect(page.locator("#chatwrap .feed")).toContainText("Agreed. Continuing.");
  await expect(composer).toBeEnabled();
});

/// File mentions must insert a real workspace path into the composer for the agent to read.
test("the composer completes a workspace path after @", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("see @app/adapters/tra");

  const first = page.locator(".menu .mrow").first();
  await expect(first).toContainText("app/adapters/transcriber.rb");

  // Tab completes the full path without sending the prompt.
  await composer.press("Tab");
  await expect(composer).toHaveValue("see @app/adapters/transcriber.rb ");
  await expect(page.locator("#chatwrap .feed")).not.toContainText("see @app");
});

/// A macOS drop may report an offset final position. Retain the previously highlighted target and use
/// the attachment draft.
test("dropping a file attaches it despite an imprecise final position", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("Compare with this screenshot");
  const at = await composer.boundingBox();
  expect(at).not.toBeNull();

  const path = "/Users/me/Desktop/Screenshot 1.png";
  await page.evaluate(({ path, x, y }) => {
    const mock = (window as unknown as {
      mock: { drop: (paths: string[], x: number, y: number, dropX: number, dropY: number) => void };
    }).mock;
    // Hover inside the composer, then drop beyond the viewport to simulate shifted native coordinates.
    mock.drop([path], x, y, x, innerHeight + 100);
  }, { path, x: at!.x + at!.width / 2, y: at!.y + at!.height / 2 });

  await expect(page.locator("#chatwrap .cfiles .injchip")).toHaveCount(1);
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText("Screenshot 1.png");
  await expect(composer).toHaveValue("Compare with this screenshot");

  await composer.press("Enter");
  const bubble = page.locator("#chatwrap .turn.user .bubble").last();
  await expect(bubble).toContainText("Compare with this screenshot");
  // The agent receives the @path; the bubble shows it as a numbered image chip.
  await expect(bubble.locator(".attachment-tag")).toHaveText("Image #1");
  await expect(bubble.locator(".attachment-tag")).toHaveAttribute("title", path);
  expect(await bubble.textContent()).toBe("Image #1\n\nCompare with this screenshot");
});

/// A large multi-repository diff mounts only rows near the viewport, avoiding thousands of offscreen DOM
/// nodes.
test("the Changes view does not mount offscreen diffs", async ({ page }) => {
  await boot(page);

  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    const patch = (n: number) =>
      ["@@ -1,30 +1,30 @@ function example() {"]
        .concat(Array.from({ length: n }, (_, i) => (i % 2 ? `+  const x${i} = updated(${i});` : `-  const y${i} = previous(${i});`)))
        .join("\n");
    const files = (repo: string, n: number) =>
      Array.from({ length: n }, (_, i) => ({
        path: `${repo}/src/folder${i % 7}/file${i}.ts`,
        added: 30,
        removed: 30,
        new_file: false,
        deleted: false,
        dirty: false,
        patch: patch(60),
      }));
    internals.invoke = async function (command, args, options) {
      if (command === "workspace_git_diff" && args?.scope === "compare") {
        return { base: "base", head: "head", files: [...files("one", 60), ...files("two", 50)] };
      }
      return original.call(this, command, args, options);
    };
  });

  await openWorkspace(page, "Hiring through the portal");
  // Refresh the diff while its view is open without requiring another agent mutation.
  await page.locator("#tab-diff").click();
  await expect(page.locator(".git-repository")).toBeVisible();

  await page.locator("#review").click();
  const dlist = page.locator("#dlist .git-review-list");
  await expect(dlist.locator(".dfile")).toHaveCount(110);
  // All 110 headers exist, while only visible portions of 6,600 diff lines mount.
  const lines = await dlist.locator(".drow").count();
  expect(lines).toBeGreaterThan(0);
  expect(lines).toBeLessThan(2_000);

  // Reserve each file's height before mounting its rows so scrolling does not jump.
  const height = await dlist.evaluate((el) => el.scrollHeight);
  expect(height).toBeGreaterThan(100_000);

  // Selecting a file near the list's end scrolls to its mounted diff.
  await dlist.locator(".dfile").last().scrollIntoViewIfNeeded();
  await expect(dlist.locator(".dfile").last().locator(".drow").first()).toBeVisible();
});

/// Files open directly in an editable viewer. Frequent board updates must preserve partially typed
/// drafts.
test("file edits survive board redraws and save", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  await page.locator("#tab-files").click();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vpre")).toContainText("Personal finance management");

  // An unchanged file has nothing to save or undo.
  await expect(page.locator("#vtext")).toBeVisible();
  await expect(page.locator("#vsave")).toBeHidden();
  await expect(page.locator("#vcancel")).toBeHidden();

  const text = "# Njord\n\nEdited manually by E2E.\n";
  await page.locator("#vtext").fill(text);
  // The highlighted pre element reflects newly typed text.
  await expect(page.locator("#vpre")).toContainText("Edited manually by E2E.");
  await page.locator("#vpreview").click();
  await expect(page.locator("#vread")).toContainText("Edited manually by E2E.");
  await page.locator("#vsource").click();
  await expect(page.locator("#vtext")).toHaveValue(text);
  await expect(page.locator("#vsave")).toBeVisible();
  await expect(page.locator("#vcrumb")).toHaveClass(/\bdirty\b/);

  const board = () =>
    page.evaluate(async () => {
      type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
      const invoke = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__.invoke;
      await invoke("set_stage", { id: "sessao-0929", stage: "Fazendo" });
    });

  await board();
  await expect(page.locator("#vtext")).toHaveValue(text);

  // Navigating to another file and back preserves the draft.
  await page.locator("#tree .treerow", { hasText: ".gitignore" }).click();
  await expect(page.locator("#vpre")).toContainText("Ignore bundler config");
  await expect(page.locator("#vview")).toBeHidden();
  await expect(page.locator("#vsave")).toBeHidden();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toHaveValue(text);
  await expect(page.locator("#vsave")).toBeVisible();

  await page.locator("#vsave").click();
  await expect(page.locator("#vsave")).toBeHidden();
  await expect(page.locator("#vcrumb")).not.toHaveClass(/\bdirty\b/);

  // After saving, a later redraw reads the edited content from disk.
  await board();
  await expect(page.locator("#vpre")).toContainText("Edited manually by E2E.");
  await expect(page.locator("#vpre")).not.toContainText("Personal finance management");

  await page.locator("#vtext").fill("# Temporary draft\n");
  await page.locator("#vpreview").click();
  await expect(page.locator("#vread")).toContainText("Temporary draft");
  await page.locator("#vcancel").click();
  await expect(page.locator("#vread")).toContainText("Edited manually by E2E.");
  await expect(page.locator("#vread")).not.toContainText("Temporary draft");

  // Saving the edited file must not overwrite another file visited during editing.
  await page.locator("#tree .treerow", { hasText: ".gitignore" }).click();
  await expect(page.locator("#vpre")).toContainText("Ignore bundler config");
  await expect(page.locator("#vpre")).not.toContainText("Edited manually by E2E.");
});

/// The Files panel is how a file inside the worktree reaches the agent. Its menu crosses the tree,
/// the workspace and the conversation draft, and the route it replaces is a Finder drag no unit test
/// can exercise. The tree's mark follows the viewer across panels, which only the rebuilt DOM shows.
test("the file tree marks the viewer's file and hands a file to the conversation", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  // A file opened from Changes is the one the tree marks once the person returns to Files.
  await page.locator("#tab-diff").click();
  await page.locator("#difflist .git-file-name", { hasText: "style.css" }).first().dblclick();
  await expect(page.locator("#vcrumb")).toContainText("style.css");
  await page.locator("#tab-files").click();
  await page.locator("#tree .treerow", { hasText: "src" }).click();
  const style = page.locator('#tree .treerow[data-path="src/style.css"]');
  await expect(style).toHaveClass(/\bselected\b/);

  // The tab strip moves the mark too, and the conversation clears it.
  await page.locator("#tree .treerow", { hasText: "README.md" }).click();
  const readme = page.locator('#tree .treerow[data-path="README.md"]');
  await expect(readme).toHaveClass(/\bselected\b/);
  await expect(style).not.toHaveClass(/\bselected\b/);
  await page.locator("#tabbar .tab.file", { hasText: "style.css" }).click();
  await expect(style).toHaveClass(/\bselected\b/);
  await expect(readme).not.toHaveClass(/\bselected\b/);
  // Closing the shown file hands the viewer, and the mark, to its neighbor.
  await page.locator("#tabbar .tab.file", { hasText: "style.css" }).locator(".tabx").click();
  await expect(readme).toHaveClass(/\bselected\b/);
  await page.locator("#tabbar .tab.file", { hasText: "README.md" }).locator(".tabx").click();
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(page.locator("#tree .treerow.selected")).toHaveCount(0);

  // The row a menu acts on, a folder included, is marked apart from the open file while the menu
  // stays open.
  const folder = page.locator('#tree .treerow[data-path="src"]');
  await folder.click({ button: "right" });
  await expect(folder).toHaveClass(/\btargeted\b/);
  await page.keyboard.press("Escape");
  await expect(folder).not.toHaveClass(/\btargeted\b/);
  const row = page.locator("#tree .treerow", { hasText: "CLAUDE.md" });
  await row.click({ button: "right" });
  await expect(row).toHaveClass(/\btargeted\b/);
  await page.locator(".menu .mrow", { hasText: "Attach to the conversation" }).click();
  await expect(row).not.toHaveClass(/\btargeted\b/);
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText("CLAUDE.md");

  // The attachment travels as the mention the agent already understands.
  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("Review this");
  await composer.press("Enter");
  const bubble = page.locator("#chatwrap .turn.user .bubble").last();
  await expect(bubble).toContainText("Review this");
  await expect(bubble.locator(".attachment-tag")).toHaveAttribute("title", "CLAUDE.md");
  expect(await bubble.textContent()).toBe("CLAUDE.md\n\nReview this");
});

/// Double-clicking a file or activating its explicit diff button opens the full file in the viewer.
test("Git opens files through double-click and the diff button", { tag: "@webkit" }, async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  await page.locator("#tab-diff").click();
  const row = page.locator("#difflist .git-file-name", { hasText: "style.css" }).first();
  await expect(row).toBeVisible();

  // A single click scrolls within the central diff without opening the file.
  await row.click();
  await expect(page.locator('#dlist .dfile[data-key$="src/style.css"] .dbody')).toBeVisible();
  await expect(page.locator("#viewer")).toBeHidden();

  await row.dblclick();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vcrumb")).toContainText("style.css");
  await expect(page.locator("#vpre")).toContainText("padding: 12px");

  // The explicit button supports keyboard access without discovering the double-click gesture.
  await page.locator("#tab-diff").click();
  const open = page.locator('#dlist .dfile[data-key$="src/style.css"] .dopen');
  await open.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vcrumb")).toContainText("style.css");
});

/// A clean repository remains selectable. Staging or committing another repository must not change its
/// index.
test("Git keeps clean repositories visible and isolates each repository’s stage", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hiring through the portal");
  await page.locator("#tab-diff").click();
  const picker = page.getByRole("button", { name: "Repository", exact: true });
  await expect(picker).toContainText("prometeu");
  await page.locator('.git-nav [data-mode="staged"]').click();
  await page.locator('[data-scope="staged"]').getByRole("button", { name: "Unstage all", exact: true }).click();
  await expect(page.locator('[data-scope="staged"] .git-file')).toHaveCount(0);
  await picker.click();
  await page.locator(".menu .mrow", { hasText: "njord" }).click();
  await expect(picker).toContainText("njord");
  await page.locator('.git-nav [data-mode="staged"]').click();
  await expect(page.locator('[data-scope="staged"] .git-file')).toHaveCount(1);
  await picker.click();
  await page.locator(".menu .mrow", { hasText: "prometeu" }).click();
  await page.locator('.git-nav [data-mode="staged"]').click();
  await expect(page.locator('[data-scope="staged"] .git-file')).toHaveCount(0);
  await expect(page.locator('.git-repository .avatar')).toHaveText("P");
});

/// Retuning preserves the tab, restarts its process and uses the selected effort. Model choices remain
/// within the current provider because resume identities are incompatible.
test("changing a running conversation’s model stops its process and preserves the tab", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Hello");

  // Target the workspace composer; hidden desk panels also remain in the DOM.
  const model = page.locator("#chatwrap .composer .mdl");
  const effort = page.locator("#chatwrap .composer .effort");
  await expect(model).toContainText("Opus (1M context)");
  await expect(effort).toContainText("High");

  await model.click();
  await expect(page.locator(".ui-search-picker-choice", { hasText: "GPT-5.6-Sol" })).toHaveCount(0);
  await page.locator(".ui-search-picker-choice", { hasText: "Sonnet" }).first().click();

  await expect(model).toContainText("Sonnet");
  await expect(page.locator("#chatwrap .composer textarea")).toHaveAttribute(
    "placeholder",
    /writing picks it up/,
  );
  // The tab survives process shutdown and its unnamed label follows the newly selected model.
  await expect(page.locator('#tabbar .tab[data-tab="t1"]')).toContainText("Sonnet 1");
  await expect(page.locator('#tabbar .tab[data-tab="t2"]')).toContainText("Sonnet 2");

  await effort.click();
  await page.getByRole("menuitemcheckbox", { name: "Very high", exact: true }).click();
  await expect(effort).toContainText("Very high");
});

test("tools: saving choices preserves a sibling tab with a queued message", async ({ page }) => {
  await boot(page);
  // Create through the normal settings flow so all three pickers share the refreshed registry.
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Resources" }).click();
  await page.locator('[data-filter="skills"]').click();
  await page.getByRole("button", { name: "Add resource", exact: true }).click();
  await page.getByRole("menuitem", { name: "Create skill", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Create skill" });
  await dialog.getByLabel("Skill name").fill("review");
  await dialog.getByLabel("When to use this skill").fill("Before shipping code");
  await dialog.getByLabel("Instructions", { exact: true }).fill("Read the changes and run tests.");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await openWorkspace(page, "Hello");
  const before = await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const board = await invoke("load_board") as Board;
    const workspace = board.workspaces.find(ws => ws.id === "sessao-0929")!;
    workspace.mcp = { base: "none", add: ["capim-ds"], remove: [] };
    workspace.plugins = { base: "none", add: ["caveman"], remove: [] };
    workspace.skills = { base: "none", add: [], remove: [] };
    for (const tab of workspace.tabs) { tab.pending_prompt = null; tab.status = "pronta"; }
    const sibling = workspace.tabs.find(tab => tab.id === "t2")!;
    sibling.status = "pronta";
    sibling.pending_prompt = "queued prompt";
    await invoke("set_stage", { id: workspace.id, stage: workspace.stage });
    return workspace;
  });
  const current = () => page.evaluate(async () => {
    type Invoke = (command: string) => Promise<Board>;
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    return (await invoke("load_board")).workspaces.find(ws => ws.id === "sessao-0929")!;
  });
  const choices = [
    ["mcp", ".mcpbtn", "notion", "notion"],
    ["plugins", ".plugbtn", "ponytail", "ponytail"],
    ["skills", ".skillbtn", "review", "skill-review"],
  ] as const;
  const choose = async (button: string, entry: string) => {
    await page.locator(`#chatwrap ${button}`).click();
    await page.locator(".menu .mrow").filter({ hasText: entry }).first().click();
    await expect(page.locator(".menu")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator(".menu")).toHaveCount(0);
  };

  const expected = structuredClone(before);
  for (const [field, button, entry, id] of choices) {
    await choose(button, entry);
    expected[field]!.add.push(id);
    // The UI preserves the queued draft; workspace_tools.rs covers the full state/selection matrix.
    await expect.poll(current).toEqual(expected);
  }
  expect(await page.evaluate(() => (window as unknown as { mock: { writes(): number } }).mock.writes())).toBe(3);
});

/// The desk supports direct replies and preserves panel order, size and collapse state across reloads.
test("the desk shows each conversation in a tile, accepts replies and preserves ordering", { tag: "@webkit" }, async ({ page }) => {
  await boot(page);
  const tiles = page.locator("#tiles .tile");
  await expect(tiles).toHaveCount(6);
  await expect(page.locator("#railbody .navitem", { hasText: "Desk" })).toHaveClass(/\bon\b/);
  const tabsContainLabels = await page.locator("#deskbar .tab").evaluateAll((tabs) => {
    tabs[0].querySelector(".n")!.textContent = "Opus with a 1M context window";
    return tabs.every((tab) =>
      [...tab.querySelectorAll("span")].every(
        (label) => label.getBoundingClientRect().right <= tab.getBoundingClientRect().right,
      ),
    );
  });
  expect(tabsContainLabels).toBe(true);

  // Include live local conversations, excluding archived, cleaned and remote workspaces.
  await expect(page.locator('#tiles .tile[data-tab="t7"]')).toHaveCount(0);
  const first = page.locator('#tiles .tile[data-tab="t1"]');
  await expect(first.locator(".tile-head")).toContainText("Hello");
  await expect(first.locator(".feed .turn")).not.toHaveCount(0);

  // A desk reply reaches its conversation.
  const composer = first.locator(".composer textarea");
  await composer.fill("Hello from the desk");
  await composer.press("Enter");
  await expect(first.locator(".feed")).toContainText("Understood: Hello from the desk");
  await expect(page.locator('#tiles .tile[data-tab="t3"] .feed')).not.toContainText("Hello from the desk");

  // Dragging reorders panels while a ghost follows the pointer and the original reserves its
  // destination.
  const head = first.locator(".tile-head");
  const target = page.locator('#tiles .tile[data-tab="t2"]');
  const from = (await head.boundingBox())!;
  const to = (await target.boundingBox())!;
  await page.mouse.move(from.x + 40, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + to.width * 0.75, to.y + 20, { steps: 8 });
  await expect(page.locator(".tile.ghost")).toHaveCount(1);
  await expect(first).toHaveClass(/\bdragging\b/);
  await page.mouse.up();
  await expect(page.locator(".tile.ghost")).toHaveCount(0);
  await expect(first).not.toHaveClass(/\bdragging\b/);
  await expect(tiles.nth(0)).toHaveAttribute("data-tab", "t2");
  await expect(tiles.nth(1)).toHaveAttribute("data-tab", "t1");

  await page.reload();
  await expect(page.locator("#tiles .tile").nth(0)).toHaveAttribute("data-tab", "t2");

  // The corner handle resizes the panel and persists its dimensions.
  const tile = page.locator('#tiles .tile[data-tab="t1"]');
  const box = (await tile.boundingBox())!;
  const grip = (await tile.locator(".tile-grip").boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2 - 120, grip.y + grip.height / 2 - 60, { steps: 6 });
  await page.mouse.up();
  const width = () => page.locator('#tiles .tile[data-tab="t1"]').evaluate((el) => el.offsetWidth);
  expect(await width()).toBeLessThan(box.width - 100);
  await page.reload();
  await expect(page.locator("#tiles .tile")).toHaveCount(6);
  expect(await width()).toBeLessThan(box.width - 100);

  // The top strip toggles collapse state and preserves it.
  const chip = page.locator('#deskbar .tab[data-tab="t3"]');
  await expect(chip).toHaveClass(/\bon\b/);
  await chip.click();
  await expect(page.locator('#tiles .tile[data-tab="t3"]')).toBeHidden();
  await expect(chip).not.toHaveClass(/\bon\b/);
  await page.reload();
  await expect(page.locator('#tiles .tile[data-tab="t3"]')).toBeHidden();
  await page.locator('#deskbar .tab[data-tab="t3"]').click();
  await expect(page.locator('#tiles .tile[data-tab="t3"]')).toBeVisible();
  // The header button also collapses the panel.
  await page.locator('#tiles .tile[data-tab="t3"] .tmin').click();
  await expect(page.locator('#tiles .tile[data-tab="t3"]')).toBeHidden();

  // The panel arrow opens its workspace at the same conversation.
  await page.locator('#tiles .tile[data-tab="t2"] .topen').click();
  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "t2");
  await expect(page.locator("#deskView")).toBeHidden();
});

/// The first prompt can arrive live and in the initial snapshot. Sequence filtering must prevent
/// duplication.
test("a message arriving during the snapshot is not duplicated", async ({ page }) => {
  await boot(page);

  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    type WindowWithHold = Window & { __TAURI_INTERNALS__: { invoke: Invoke }; snapshotHeld?: boolean };
    const w = window as WindowWithHold;
    const original = w.__TAURI_INTERNALS__.invoke;
    let first = true;
    w.__TAURI_INTERNALS__.invoke = async function (command, args, options) {
      if (first && command === "chat_snapshot" && args?.session === "t1") {
        first = false;
        w.snapshotHeld = true;
        await new Promise((resolve) => setTimeout(resolve, 300));
      }
      return original.call(this, command, args, options);
    };
  });

  await page.locator("#railbody .navitem.sub .lbl").getByText("Hello", { exact: true }).click();
  await expect.poll(() => page.evaluate(() => !!(window as Window & { snapshotHeld?: boolean }).snapshotHeld)).toBe(true);
  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "LAUNCH_MESSAGE" } });
  });

  await expect(page.locator("#chatwrap .bubble", { hasText: "LAUNCH_MESSAGE" })).toHaveCount(1);
  await page.waitForTimeout(400);
  await expect(page.locator("#chatwrap .bubble", { hasText: "LAUNCH_MESSAGE" })).toHaveCount(1);
});

/// A tab can reattach while an older snapshot is pending. Attachment generation, not tab ID alone,
/// determines which response can render.
test("the desk ignores a delayed snapshot of the same conversation", async ({ page }) => {
  await boot(page);
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  await expect(page.locator("#wsView")).toBeVisible();

  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "OLD_SNAPSHOT" } });
  });
  await expect(page.locator('#chatwrap .bubble', { hasText: "OLD_SNAPSHOT" })).toBeVisible();

  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    type WindowWithDelay = Window & {
      __TAURI_INTERNALS__: { invoke: Invoke };
      deskSnapshotCaptured?: boolean;
      deskSnapshotReturned?: boolean;
    };
    const w = window as WindowWithDelay;
    const original = w.__TAURI_INTERNALS__.invoke;
    let first = true;
    w.__TAURI_INTERNALS__.invoke = async function (command, args, options) {
      if (first && command === "chat_snapshot" && args?.session === "t1") {
        first = false;
        const snapshot = await original.call(this, command, args, options);
        w.deskSnapshotCaptured = true;
        await new Promise((resolve) => setTimeout(resolve, 800));
        w.deskSnapshotReturned = true;
        return snapshot;
      }
      return original.call(this, command, args, options);
    };
  });

  await page.locator("#railbody .navitem", { hasText: "Desk" }).click();
  await expect.poll(() => page.evaluate(() => !!(window as Window & { deskSnapshotCaptured?: boolean }).deskSnapshotCaptured)).toBe(true);
  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "NEW_SNAPSHOT" } });
  });
  await page.locator("#railbody .navitem", { hasText: "Desk" }).click();

  const tile = page.locator('#tiles .tile[data-tab="t1"]');
  await expect(tile.locator(".bubble", { hasText: "NEW_SNAPSHOT" })).toHaveCount(1);
  await expect(tile.locator(".bubble", { hasText: "OLD_SNAPSHOT" })).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => !!(window as Window & { deskSnapshotReturned?: boolean }).deskSnapshotReturned)).toBe(true);
  await expect(tile.locator(".bubble", { hasText: "OLD_SNAPSHOT" })).toHaveCount(1);
});

/// The desk persists each workspace's MCP pick independently; the provenance picker opens asynchronously
/// and re-opens after each write (ADR 0045, phases 6 and 7).
test("the desk saves MCP choices in separate workspaces", async ({ page }) => {
  await boot(page);
  const first = page.locator('#tiles .tile[data-tab="t1"] .mcpbtn');
  const second = page.locator('#tiles .tile[data-tab="t5"] .mcpbtn');
  await expect(first).toBeVisible();
  await expect(second).toBeVisible();

  const pick = async (button: Locator, name: string) => {
    await button.click();
    await page.locator(".menu .mrow").filter({ hasText: name }).first().click();
    // The picker re-opens after the write resolves; dismiss it before the next interaction.
    await expect(page.locator(".menu")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator(".menu")).toHaveCount(0);
    await expect(button).toContainText(name);
  };

  await pick(first, "capim-ds");
  await pick(second, "notion");

  await expect(first).toContainText("capim-ds");
  await expect(second).toContainText("notion");
  await expect.poll(() => page.evaluate(() => (window as unknown as { mock: { writes: () => number } }).mock.writes())).toBe(2);
});

/// Desk and workspace share the conversation draft, including unsent text and attachments.
test("the desk preserves drafts and attachments when opening a workspace", async ({ page }) => {
  await boot(page);
  const tile = page.locator('#tiles .tile[data-tab="t1"]');
  const draft = "Continue this message in the workspace";
  await tile.locator(".composer textarea").fill(draft);
  const at = (await tile.locator(".feed").boundingBox())!;
  await page.evaluate(({ x, y }) => {
    (window as unknown as { mock: { drop: (paths: string[], x: number, y: number) => void } }).mock.drop(
      ["/Users/me/Desktop/context.txt"],
      x,
      y,
    );
  }, { x: at.x + at.width / 2, y: at.y + at.height / 2 });
  await expect(tile.locator(".cfiles .injchip")).toContainText("context.txt");

  await tile.locator(".topen").click();
  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#chatwrap .composer textarea")).toHaveValue(draft);
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText("context.txt");
});
