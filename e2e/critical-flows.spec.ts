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
      team: "timeDeMentira",
      secret: "segredoDeMentira123456789",
      member: "eu_mock",
      credential: "c".repeat(43),
      name: "Você",
    }));
  });
  await boot(page);
  await expect(page.locator("#railbody .navitem.mentions")).toBeVisible();
}

test("compartilhamento adota a chave nova do dispositivo sem pedir revisão", async ({ page }) => {
  await bootTeam(page);
  const settings = async () => {
    await page.locator("#settings").click();
    await page.locator(".setnavitem", { hasText: "Organizações" }).click();
  };
  await settings();
  await expect(page.locator("#settingsView")).toContainText("criptografia ponta a ponta");
  // The list names people, never their devices.
  await expect(page.locator("#settingsView .members .mem .nm")).toHaveText(["Você (você)", "Marcus Hale", "John Okafor"]);

  // Replacing only the simulated peer's private storage changes that device's identity: a reinstall.
  await page.evaluate(() => {
    for (const key of Object.keys(localStorage)) if (key.startsWith("mock:peer-security:")) localStorage.removeItem(key);
  });
  await page.reload();
  await settings();
  await expect(page.locator("#settingsView .members .mem .nm")).toHaveText(["Você (você)", "Marcus Hale", "John Okafor"]);
  await expect(page.locator("#settingsView")).not.toContainText("bloquead");
  // Encrypted collaboration keeps flowing under the new key, with nothing to confirm.
  await expect(page.locator("#railbody .navitem.mentions")).toBeVisible();
});

test("comentário fica ao lado da sessão até alguém resolver", async ({ page }) => {
  await bootTeam(page);

  // Opening the context does not resolve the pending comment.
  await page.locator("#railbody .navitem.mentions").click();
  await page.locator(".inboxrow").click();
  await expect(page.locator("#crumb")).toContainText("Arquivar todos os concluídos");
  await expect(page.locator("#comments .commentthread")).toBeVisible();
  await expect(page.locator("#chatwrap .commentpin")).toHaveCount(1);
  await expect(page.locator("#railbody .navitem.mentions .n")).toHaveText("1");
  await expect(page.locator("#chatwrap .composer textarea")).toHaveAttribute("placeholder", "Escreva na conversa de Marcus Hale");

  await page.locator('#tabbar .tab[data-tab="mt2"]').click();
  await expect(page.locator("#comments .commentempty")).toContainText("Nenhum comentário aberto");
  await page.locator('#tabbar .tab[data-tab="mt1"]').click();
  await page.locator("#comments .commentcard").first().click();

  await page.locator(".replybox textarea").fill("Concordo com a coluna.");
  await page.locator(".replybox .submit").click();
  await expect(page.locator(".commentmessage")).toHaveCount(2);
  await page.locator(".replybox .resolve").click();
  await expect(page.locator(".threadstate")).toHaveText("Resolvido");
  await expect(page.locator("#railbody .navitem.mentions")).toHaveCount(0);
  await expect(page.locator("#chatwrap .commentpin")).toHaveCount(0);

  // Comments use a separate field; the main composer still sends to the agent.
  await page.locator(".threadhead .back").click();
  await page.locator('.commentfilters [data-filter="resolved"]').click();
  await expect(page.locator(".commentcard", { hasText: "Completar um todo agora" })).toBeVisible();
  await page.locator('.commentfilters [data-filter="open"]').click();
  await page.locator("#chatwrap .meta .cm").last().click();
  await expect(page.locator(".commentdraft")).toBeVisible();
  await expect(page.locator(".commentdraft .draftquote-text")).not.toBeEmpty();
  await expect(page.locator("#chatwrap .composer textarea")).toHaveAttribute("placeholder", "Escreva na conversa de Marcus Hale");
  await page.locator(".commentdraft textarea").fill("Nova dúvida para o time.");
  await page.locator(".commentdraft").evaluate(card => {
    const submit = card.querySelector<HTMLButtonElement>(".submit")!;
    submit.click(); submit.click();
    const area = card.querySelector<HTMLTextAreaElement>("textarea")!;
    area.value = "Rascunho digitado enquanto cifra.";
    area.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect(page.locator(".commentcard", { hasText: "Nova dúvida para o time." })).toBeVisible();
  await expect(page.locator(".commentcard", { hasText: "Nova dúvida para o time." })).toHaveCount(1);
  await expect(page.locator(".commentdraft textarea")).toHaveValue("Rascunho digitado enquanto cifra.");
  await expect(page.locator("#chatwrap .commentpin")).toHaveCount(1);
  await expect(page.locator("#chatwrap .note")).toHaveCount(0);
});

test("clicar no projeto abre os arquivos do clone, sem workspace", async ({ page }) => {
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
  await expect(page.locator("#vpre")).toContainText("Controle financeiro pessoal");

  // Opened files become tabs and share the same add control as workspace tabs.
  await expect(page.locator("#tabbar .tab")).toHaveText(["CLAUDE.md"]);
  await expect(page.locator("#tabbar .tabadd .caret")).toBeVisible();
  await page.locator("#tree .treerow", { hasText: "README.md" }).click();
  await expect(page.locator("#tabbar .tab")).toHaveText(["CLAUDE.md", "README.md"]);
  await expect(page.locator("#tabbar .tab.on")).toHaveText("README.md");
  await page.locator("#tabbar .tab", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vpre")).toContainText("Controle financeiro pessoal");

  // Editing and saving use the workspace viewer path.
  await page.locator("#vtext").fill("# Direto do projeto\n");
  await expect(page.locator("#vsave")).toBeVisible();
  await page.locator("#vsave").click();
  await expect(page.locator("#vsave")).toBeHidden();

  // The dropdown lists actions available from the project.
  await page.locator("#tabbar .tabadd .caret").click();
  await expect(page.locator(".menu .mrow")).toHaveText(["Terminal novo", "Workspace novo"]);
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
  await expect(page.locator("#railbody .navitem.sub", { hasText: "Tela igual ao Conductor" })).toHaveCount(0);
});

test("a barra lateral preserva conversa e terminal ao sair de um arquivo do projeto", async ({ page }) => {
  await boot(page);
  const project = page.locator("#railbody .group", { hasText: "njord", hasNotText: "+" });
  await project.locator("span").nth(1).click();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toBeVisible();
  await page.locator("#vtext").fill("Rascunho do clone");

  // Entering a workspace leaves project-only mode before any tab or dock redraw.
  await openWorkspace(page, "Ola");
  const conversation = page.locator('#tabbar .tab[data-tab="t1"]');
  const composer = page.locator("#chatwrap .composer textarea");
  await expect(composer).toBeVisible();
  await expect(conversation).toHaveClass(/\bon\b/);
  await expect(page.locator("#viewer")).toBeHidden();

  // The same path belongs to a different root; opening it preserves conversation tabs.
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toBeVisible();
  await expect(page.locator("#vtext")).not.toHaveValue("Rascunho do clone");
  await expect(page.locator("#tabbar .tab[data-tab]")).toHaveCount(2);
  await page.locator("#vtext").fill("Rascunho do workspace");
  await page.locator("#tabbar .tab", { hasText: "CLAUDE.md" }).locator(".tabx").click();
  await expect(composer).toBeVisible();
  await expect(conversation).toHaveClass(/\bon\b/);

  await page.locator("#tabbar .tabadd .caret").click();
  await page.locator(".menu .mrow", { hasText: "Terminal novo" }).click();
  await expect(page.locator("#termview")).toBeVisible();
  await expect(conversation).toBeVisible();
  await conversation.click();
  await expect(composer).toBeVisible();

  // History restores each root's own files and drafts.
  await page.locator("#back").click();
  await expect(page.locator("#vtext")).toHaveValue("Rascunho do clone");
  await expect(page.locator("#tabbar .tab")).toHaveText(["CLAUDE.md"]);
  await page.locator("#fwd").click();
  await expect(composer).toBeVisible();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toHaveValue("Rascunho do workspace");
  await expect(conversation).toBeVisible();

  // Selecting an agent directly also exits project-only mode.
  await project.locator("span").nth(1).click();
  await expect(page.locator("#vtext")).toHaveValue("Rascunho do clone");
  await page.locator('.railagent[data-tab="t2"]').click();
  await expect(composer).toBeVisible();
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "t2");
});

test("remove projeto sem apagar seus workspaces", async ({ page }) => {
  await boot(page);

  // The combined project name also matches a search for its second repository.
  const project = page.locator("#railbody .group", { hasText: "njord", hasNotText: "+" });
  await project.hover();
  await project.locator('button[title="Ações de njord"]').click();
  await page.locator(".menu .mrow", { hasText: "Remover projeto" }).click();

  await expect(project).toHaveCount(0);
  await expect(page.locator("#railbody .group", { hasText: "Sem projeto" })).toBeVisible();
  await expect(page.locator("#railbody .navitem.sub", { hasText: "Ola" })).toBeVisible();
});

test("workspace com mais de um repo mora em Conjuntos, não no projeto", async ({ page }) => {
  await boot(page);

  const set = page.locator("#railbody .group", { hasText: "prometeu + njord" });
  await expect(set).toBeVisible();
  await expect(set.locator(".avatar.multi.n2 i")).toHaveText(["P", "N"]);
  await expect(set.locator(".ico")).toHaveCount(0);
  await expect(page.locator("#railbody .sect", { hasText: "Conjuntos" })).toBeVisible();

  await expect(page.locator("#railbody .navitem.sub", { hasText: "Contratação pelo portal" })).toHaveCount(1);
  await expect(set.locator("xpath=following-sibling::*[1]").locator(".navitem.sub .lbl")).toHaveText("Contratação pelo portal");
});

test("separa a cota geral das janelas próprias de um modelo Codex", async ({ page }) => {
  await boot(page);
  const usage = page.locator('#status .uchip[title="Cotas"]');
  await expect(usage).toHaveCount(2);
  await usage.nth(1).click();

  const panel = page.locator(".upop.usage");
  const general = panel.locator(".ugroup", { hasText: "Geral" });
  const spark = panel.locator(".ugroup", { hasText: "GPT-5.3-Codex-Spark" });
  await expect(general).toBeVisible();
  await expect(general.locator("xpath=following-sibling::*[1]")).toContainText("7 dias");
  await expect(spark).toBeVisible();
  await expect(spark.locator("xpath=following-sibling::*[1]")).toContainText("5 horas");
  await expect(spark.locator("xpath=following-sibling::*[2]")).toContainText("7 dias");
});

test("o topo local fica estável e não trata workspace comum como compartilhado", async ({ page }) => {
  await boot(page);
  const railWorkspace = page.locator("#railbody .navitem.sub", { hasText: "Ola" });
  await expect(railWorkspace.locator(".rail-status")).toHaveAttribute("aria-label", "pronta");
  await expect(railWorkspace.locator(".wsbranch")).toHaveText("prometeu/sessao-0929");
  await expect(railWorkspace.locator(".st")).toHaveCount(0);
  await expect(railWorkspace).toHaveAttribute("title", /njord/);
  await expect(railWorkspace).not.toHaveAttribute("title", /Fazendo/);
  await expect(railWorkspace).not.toHaveAttribute("title", /Pronta|Rodando|Desligada/);
  await openWorkspace(page, "Ola");

  // An unshared workspace has no permanent collaboration toolbar state. Sharing appears in its menu only
  // after team setup.
  await expect(page.locator("#msg")).toBeHidden();
  await expect(page.locator("#share")).toBeHidden();
  await expect(page.locator("#wsmore")).toBeVisible();
  await page.locator("#wsmore").click();
  await expect(page.locator(".menu .mrow", { hasText: "Definir etapa" })).toBeVisible();
  await page.keyboard.press("Escape");

  // Board redraws preserve the identity node instead of recreating it for every agent tool event.
  await page.locator("#crumb .nm").evaluate((el) => (el.dataset.stable = "yes"));
  await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const invoke = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__.invoke;
    await invoke("set_stage", { id: "sessao-0929", stage: "Fazendo" });
  });
  await expect(page.locator("#crumb .nm")).toHaveAttribute("data-stable", "yes");
  await expect(page.locator("#msg")).toBeHidden();
});

test("a barra lateral lista agentes por workspace, acompanha status e abre a aba escolhida", async ({ page }) => {
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
  await expect(card.locator(".railagents-toggle")).toHaveText("2 agentes");
  await expect(first.locator(".provider image")).toBeVisible();
  await expect(first).toHaveAttribute("title", /Claude Code/);
  await expect(second.locator(".provider path")).toHaveCount(1);
  await expect(second).toHaveAttribute("title", /Codex/);
  // An unnamed tab displays its model on one line.
  await expect(first.locator(".lbl")).toHaveText("Opus · 1M");
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
  await openWorkspace(page, "Tela igual ao Conductor");
  await openWorkspace(page, "Ola");
  await expect(second).toHaveAttribute("aria-current", "true");
  await expect(page.locator("#tabbar .tab.on")).toHaveAttribute("data-tab", "t2");

  // Collapsing does not navigate, and the choice survives redraws and reloads.
  await card.locator(".railagents-toggle").focus();
  await page.keyboard.press("Enter");
  await expect(card.locator(".railagents-toggle")).toHaveAttribute("aria-expanded", "false");
  await expect(first).toBeHidden();
  await expect(page.locator("#wsView")).toBeVisible();
  await page.locator("#railbody .navitem", { hasText: "Mesa" }).click();
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
  await expect(card.locator(".rail-status")).toHaveAttribute("aria-label", "preparando");
  await card.locator(".navitem").click();
  await expect(card).toHaveClass(/\bon\b/);
  await expect(page.locator("#wsView")).toBeVisible();
});

test("a barra lateral abre agentes remotos sem inventar provider e mostra o dono offline", async ({ page }) => {
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
  await expect(second.locator(".rail-status")).toHaveAttribute("aria-label", "desligada");
});

test("a troca rápida de aba ignora o snapshot atrasado da aba anterior", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

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
test("fechar a conversa selecionada seleciona a aba que sobrou", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const first = page.locator('#tabbar .tab[data-tab="t1"]');
  const second = page.locator('#tabbar .tab[data-tab="t2"]');
  await page.evaluate(() => {
    const mock = (window as unknown as {
      mock: { line: (tab: string, line: unknown) => void };
    }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "E2E_FECHAR_T1" } });
  });

  await second.click();
  await expect(second).toHaveClass(/\bon\b/);
  await second.hover();
  await second.locator(".tabx").click();

  await expect(second).toHaveCount(0);
  await expect(first).toHaveClass(/\bon\b/);
  await expect(page.locator("#chatwrap .bubble", { hasText: "E2E_FECHAR_T1" })).toBeVisible();
});

test("recolhe a saída técnica de uma ferramenta que falhou", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  await page.evaluate(() => {
    const mock = (window as unknown as {
      mock: { line: (tab: string, line: unknown) => void };
    }).mock;
    mock.line("t1", {
      type: "assistant",
      message: {
        id: "m-erro-e2e",
        role: "assistant",
        content: [{ type: "tool_use", id: "tu-erro-e2e", name: "Bash", input: { command: "apply_patch" } }],
      },
    });
    mock.line("t1", {
      type: "user",
      message: {
        role: "user",
        content: [{
          type: "tool_result",
          tool_use_id: "tu-erro-e2e",
          is_error: true,
          content: "Script failed\nWall time: 0.1 seconds\nOutput:\napply_patch verification failed: trecho não encontrado",
        }],
      },
    });
  });

  const tool = page.locator('#chatwrap .tool[data-tool="tu-erro-e2e"]');
  await expect(tool).toHaveClass(/\bbad\b/);
  await expect(tool.locator(".tout")).toBeHidden();

  await tool.locator(".thead").click();
  await expect(tool.locator(".tfail")).toContainText("Uma etapa falhou");
  await expect(tool.locator(".tout")).toBeHidden();

  await tool.locator(".ttechnical summary").click();
  await expect(tool.locator(".tout")).toContainText("apply_patch verification failed");
});

test("cria um workspace pelo launcher e acompanha o preparo até a conversa", async ({ page }) => {
  await boot(page);

  await page.locator("#railbody").getByRole("button", { name: "Criar", exact: true }).click();
  await expect(page.locator("#veil .sheet")).toBeVisible();

  const title = "Workspace criado pelo E2E";
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

test("a lista de issues cabe no lançador e deixa os títulos legíveis", async ({ page }) => {
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

  await page.locator("#railbody").getByRole("button", { name: "Criar", exact: true }).click();
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

test("mantém os controles do lançador dentro da caixa com branch base longa", async ({ page }) => {
  await page.setViewportSize({ width: 650, height: 800 });
  await boot(page);

  const branch = "origin/feature/nome-de-branch-comprido-o-bastante-para-precisar-de-reticencias";
  await page.evaluate((longBranch) => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = function (command, args, options) {
      if (command === "list_branches") return Promise.resolve({ all: [longBranch], default: longBranch });
      return original.call(this, command, args, options);
    };
  }, branch);

  await page.locator("#railbody").getByRole("button", { name: "Criar", exact: true }).click();
  await expect(page.locator("#d-basename")).toHaveText(branch);

  const bounds = await page.locator(".sheettop").evaluate((top) => {
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
  expect(bounds.branchContentWidth).toBeGreaterThan(bounds.branchWidth);
});

test("envia uma pergunta, responde o card e devolve o controle ao chat", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("Tenho uma pergunta para o fluxo E2E");
  await composer.press("Enter");

  const card = page.locator("#chatwrap .question");
  await expect(card).toBeVisible();
  await expect(card.locator(".qbody")).toContainText("Onde guardar os concluídos?");
  await card.locator(".qbody .opt").first().click();
  await expect(card.locator(".qbody")).toContainText("Rodar a migração agora?");
  await card.locator(".qbody .opt").first().click();

  const answer = card.locator("button.pri");
  await expect(answer).toBeEnabled();
  await answer.click();

  await expect(card).toHaveCount(0);
  await expect(page.locator("#chatwrap .feed")).toContainText("Combinado. Seguindo.");
  await expect(composer).toBeEnabled();
});

/// File mentions must insert a real workspace path into the composer for the agent to read.
test("o @ na caixa completa um caminho do workspace", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("veja @app/adapters/tra");

  const first = page.locator(".menu .mrow").first();
  await expect(first).toContainText("app/adapters/transcriber.rb");

  // Tab completes the full path without sending the prompt.
  await composer.press("Tab");
  await expect(composer).toHaveValue("veja @app/adapters/transcriber.rb ");
  await expect(page.locator("#chatwrap .feed")).not.toContainText("veja @app");
});

/// A macOS drop may report an offset final position. Retain the previously highlighted target and use
/// the attachment draft.
test("arquivo solto na conversa vira anexo mesmo com a posição final imprecisa", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("Compare com esta captura");
  const at = await composer.boundingBox();
  expect(at).not.toBeNull();

  const path = "/Users/eu/Desktop/Captura de Tela.png";
  await page.evaluate(({ path, x, y }) => {
    const mock = (window as unknown as {
      mock: { drop: (paths: string[], x: number, y: number, dropX: number, dropY: number) => void };
    }).mock;
    // Hover inside the composer, then drop beyond the viewport to simulate shifted native coordinates.
    mock.drop([path], x, y, x, innerHeight + 100);
  }, { path, x: at!.x + at!.width / 2, y: at!.y + at!.height / 2 });

  await expect(page.locator("#chatwrap .cfiles .injchip")).toHaveCount(1);
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText("Captura de Tela.png");
  await expect(composer).toHaveValue("Compare com esta captura");

  await composer.press("Enter");
  const bubble = page.locator("#chatwrap .turn.user .bubble").last();
  await expect(bubble).toContainText("Compare com esta captura");
  expect(await bubble.textContent()).toBe('@"/Users/eu/Desktop/Captura de Tela.png"\n\nCompare com esta captura');
});

/// Recently accessed files rank first when multiple paths match equally.
test("o arquivo que o agente acabou de ler sobe na lista do @", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("veja @waha");
  await expect(page.locator(".menu .mrow").first()).toContainText("app/adapters/waha.rb");

  // An agent read updates mention ranking ahead of another equally matching path.
  await composer.fill("");
  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", {
      type: "assistant",
      message: {
        id: "m-recency",
        role: "assistant",
        content: [{ type: "tool_use", id: "tu-recency", name: "Read", input: { file_path: "app/adapters/transcriber.rb" } }],
      },
    });
  });
  await composer.fill("veja @adapters");
  await expect(page.locator(".menu .mrow").first()).toContainText("app/adapters/transcriber.rb");
});

/// A large multi-repository diff mounts only rows near the viewport, avoiding thousands of offscreen DOM
/// nodes.
test("a tela de Mudanças não monta o diff que ninguém está vendo", async ({ page }) => {
  await boot(page);

  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    const patch = (n: number) =>
      ["@@ -1,30 +1,30 @@ function algo() {"]
        .concat(Array.from({ length: n }, (_, i) => (i % 2 ? `+  const x${i} = novo(${i});` : `-  const y${i} = velho(${i});`)))
        .join("\n");
    const files = (repo: string, n: number) =>
      Array.from({ length: n }, (_, i) => ({
        path: `${repo}/src/pasta${i % 7}/arquivo${i}.ts`,
        added: 30,
        removed: 30,
        new_file: false,
        deleted: false,
        dirty: false,
        patch: patch(60),
      }));
    internals.invoke = async function (command, args, options) {
      if (command === "workspace_git_diff" && args?.scope === "compare") {
        return { base: "base", head: "head", files: [...files("um", 60), ...files("dois", 50)] };
      }
      return original.call(this, command, args, options);
    };
  });

  await openWorkspace(page, "Contratação pelo portal");
  // Refresh the diff while its view is open without requiring another agent mutation.
  await page.locator("#tab-diff").click();
  await expect(page.locator(".git-repository")).toBeVisible();

  await page.locator("#review").click();
  const dlist = page.locator("#dlist .git-review-list");
  await expect(dlist.locator(".dfile")).toHaveCount(110);
  // All 110 headers exist, while only visible portions of 6,600 diff lines mount.
  const linhas = await dlist.locator(".drow").count();
  expect(linhas).toBeGreaterThan(0);
  expect(linhas).toBeLessThan(2_000);

  // Reserve each file's height before mounting its rows so scrolling does not jump.
  const altura = await dlist.evaluate((el) => el.scrollHeight);
  expect(altura).toBeGreaterThan(100_000);

  // Selecting a file near the list's end scrolls to its mounted diff.
  await dlist.locator(".dfile").last().scrollIntoViewIfNeeded();
  await expect(dlist.locator(".dfile").last().locator(".drow").first()).toBeVisible();
});

/// Files open directly in an editable viewer. Frequent board updates must preserve partially typed
/// drafts.
test("escrever no arquivo aberto sobrevive ao redesenho do quadro e salva", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  await page.locator("#tab-files").click();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vpre")).toContainText("Controle financeiro pessoal");

  // An unchanged file has nothing to save or undo.
  await expect(page.locator("#vtext")).toBeVisible();
  await expect(page.locator("#vsave")).toBeHidden();
  await expect(page.locator("#vcancel")).toBeHidden();

  const texto = "# Njord\n\nCorrigido à mão pelo E2E.\n";
  await page.locator("#vtext").fill(texto);
  // The highlighted pre element reflects newly typed text.
  await expect(page.locator("#vpre")).toContainText("Corrigido à mão pelo E2E.");
  await expect(page.locator("#vsave")).toBeVisible();
  await expect(page.locator("#vcrumb")).toHaveClass(/\bdirty\b/);

  const board = () =>
    page.evaluate(async () => {
      type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
      const invoke = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__.invoke;
      await invoke("set_stage", { id: "sessao-0929", stage: "Fazendo" });
    });

  await board();
  await expect(page.locator("#vtext")).toHaveValue(texto);

  // Navigating to another file and back preserves the draft.
  await page.locator("#tree .treerow", { hasText: ".gitignore" }).click();
  await expect(page.locator("#vpre")).toContainText("Ignore bundler config");
  await expect(page.locator("#vsave")).toBeHidden();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toHaveValue(texto);
  await expect(page.locator("#vsave")).toBeVisible();

  await page.locator("#vsave").click();
  await expect(page.locator("#vsave")).toBeHidden();
  await expect(page.locator("#vcrumb")).not.toHaveClass(/\bdirty\b/);

  // After saving, a later redraw reads the edited content from disk.
  await board();
  await expect(page.locator("#vpre")).toContainText("Corrigido à mão pelo E2E.");
  await expect(page.locator("#vpre")).not.toContainText("Controle financeiro pessoal");

  // Saving the edited file must not overwrite another file visited during editing.
  await page.locator("#tree .treerow", { hasText: ".gitignore" }).click();
  await expect(page.locator("#vpre")).toContainText("Ignore bundler config");
  await expect(page.locator("#vpre")).not.toContainText("Corrigido à mão pelo E2E.");
});

/// Double-clicking a file or activating its explicit diff button opens the full file in the viewer.
test("Git abre o arquivo pelo duplo clique e pelo botão do diff", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

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
test("Git mantém repositórios limpos e isola o stage de cada repositório", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Contratação pelo portal");
  await page.locator("#tab-diff").click();
  const picker = page.getByRole("button", { name: "Repositório", exact: true });
  await expect(picker).toContainText("prometeu");
  await page.locator('.git-nav [data-mode="staged"]').click();
  await page.locator('[data-scope="staged"]').getByRole("button", { name: "Remover tudo do stage", exact: true }).click();
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

/// MCP and plugin selection belongs to the workspace and remains visible across provider changes.
test("escolher um GPT mantém o seletor de plugins do lançador", async ({ page }) => {
  await boot(page);
  await page.locator("#railbody").getByRole("button", { name: "Criar", exact: true }).click();
  await expect(page.locator("#d-plugins")).toBeVisible();

  await page.locator("#d-model").click();
  await page.locator(".menu .mrow", { hasText: "GPT-5.6-Sol" }).first().click();
  await expect(page.locator("#d-plugins")).toBeVisible();
  await expect(page.locator("#d-mcp")).toBeVisible();
});

/// Retuning preserves the tab, restarts its process and uses the selected effort. Model choices remain
/// within the current provider because resume identities are incompatible.
test("trocar o modelo de uma conversa de pé desliga o processo e mantém a aba", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  // Target the workspace composer; hidden desk panels also remain in the DOM.
  const model = page.locator("#chatwrap .composer .mdl");
  const effort = page.locator("#chatwrap .composer .effort");
  await expect(model).toContainText("Opus · 1M");
  await expect(effort).toContainText("Alto");

  await model.click();
  await expect(page.locator(".menu .mrow", { hasText: "GPT-5.6-Sol" })).toHaveCount(0);
  await page.locator(".menu .mrow", { hasText: "Sonnet" }).first().click();

  await expect(model).toContainText("Sonnet");
  await expect(page.locator("#chatwrap .composer textarea")).toHaveAttribute(
    "placeholder",
    /escrever retoma/,
  );
  // The tab survives process shutdown and its unnamed label follows the newly selected model.
  await expect(page.locator('#tabbar .tab[data-tab="t1"]')).toContainText("Sonnet 1");
  await expect(page.locator('#tabbar .tab[data-tab="t2"]')).toContainText("Sonnet 2");

  await effort.click();
  await expect(effort).toContainText("Muito alto");
});

test("o filtro por time corta a lista de issues e as contagens seguem a busca", async ({ page }) => {
  await boot(page);
  await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const invoke = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__.invoke;
    await invoke("linear_connect");
  });
  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  await expect(page.locator("#issuesView")).toBeVisible();

  const pills = page.locator("#iteams .tpill");
  await expect(pills).toHaveCount(3, { timeout: 10_000 });
  await expect(pills.first()).toHaveClass(/\bon\b/);
  await expect(page.locator("#ilist .irow")).toHaveCount(7);

  // Selecting a team filters its issues.
  await pills.filter({ hasText: "INF" }).click();
  await expect(page.locator("#ilist .irow")).toHaveCount(2);
  await expect(page.locator("#ilist .irow .iid").first()).toContainText("INF-");

  // Search updates team counts without replacing the team filters or current selection.
  await page.locator("#ibar input").fill("runner");
  await expect(pills).toHaveCount(3);
  await expect(pills.filter({ hasText: "INF" })).toHaveClass(/\bon\b/);
  await expect(pills.filter({ hasText: "MOA" })).toContainText("0");
  await expect(page.locator("#ilist .irow")).toHaveCount(1);

  // Selecting all teams restores matching issues across teams.
  await page.locator("#ibar input").fill("linear");
  await expect(page.locator("#ilist .iempty")).toBeVisible();
  await pills.first().click();
  await expect(page.locator("#ilist .irow")).toHaveCount(1);
});

/// Test plugin creation from request through streamed agent output to its final catalog entry.
test("cria um plugin pelo Prometeu e ele entra na lista", async ({ page }) => {
  await boot(page);
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Plugins" }).click();
  await page.locator(".setrow.head button", { hasText: "Criar plugin" }).click();

  await page.locator(".sheet.hubedit input").fill("Diário do dia");
  await page.locator(".sheet.hubedit textarea").fill("Um comando que resume o dia num arquivo datado.");
  await page.locator(".sheetbar button", { hasText: "Criar" }).click();

  // Show live generation output and hide the editable request while the agent works.
  await expect(page.locator(".sheet.hubedit .mstep").first()).toBeVisible();
  await expect(page.locator(".sheet.hubedit textarea")).toHaveCount(0);
  await expect(page.locator(".sheet.hubedit .mstep", { hasText: "plugin.json" })).toBeVisible();

  // Completion closes the sheet and registers the plugin from its managed Prometeu directory.
  await expect(page.locator(".sheet.hubedit")).toHaveCount(0);
  const row = page.locator(".setrow", { hasText: "diario-do-dia" });
  await expect(row).toBeVisible();
  await expect(row).toContainText("~/.prometeu/plugins/diario-do-dia");
});

/// Installing a repository with multiple plugins asks which entries to register.
test("instala um plugin pelo endereço do repositório", async ({ page }) => {
  await boot(page);
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Plugins" }).click();

  await page.locator(".setrow.head button", { hasText: "Instalar plugin" }).click();
  await page.locator(".sheet.hubedit input").fill("gbrancaglione/exemplo");
  await page.locator(".sheetbar button", { hasText: "Instalar" }).click();

  // A single-plugin repository installs directly and closes the sheet.
  await expect(page.locator(".sheet.hubedit")).toHaveCount(0);
  const row = page.locator(".setrow", { hasText: "exemplo" });
  await expect(row).toContainText("github.com/gbrancaglione/exemplo");
  await expect(row.locator("button", { hasText: "Atualizar" })).toBeVisible();

  // For multi-plugin repositories, install only checked entries.
  await page.locator(".setrow.head button", { hasText: "Instalar plugin" }).click();
  await page.locator(".sheet.hubedit input").fill("acme/muitos-plugins");
  await page.locator(".sheetbar button", { hasText: "Instalar" }).click();
  await expect(page.locator(".mpickrow")).toHaveCount(2);
  await page.locator(".mpickrow", { hasText: "muitos-plugins-dois" }).locator("input").uncheck();
  await page.locator(".sheetbar button", { hasText: "Adicionar" }).click();

  await expect(page.locator(".sheet.hubedit")).toHaveCount(0);
  await expect(page.locator(".setrow", { hasText: "muitos-plugins-um" })).toBeVisible();
  await expect(page.locator(".setrow", { hasText: "muitos-plugins-dois" })).toHaveCount(0);
});

/// Each tool toggle persists on its own and applies at the next spawn; the picker re-opens showing the
/// resolved effective set instead of restarting the running session (ADR 0043, phases 6 and 7).
test("marcar plugins na conversa grava cada mudança e vale para a próxima fala", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  // Target the workspace composer; hidden desk panels also remain in the DOM.
  const plugbtn = page.locator("#chatwrap .plugbtn");
  const mcpbtn = page.locator("#chatwrap .mcpbtn");
  const menuRow = (name: string) => page.locator(".menu .mrow").filter({ hasText: name }).first();
  const reopen = async () => {
    // The picker closes on click and re-opens after the write resolves; wait for it, then dismiss it.
    await expect(page.locator(".menu")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator(".menu")).toHaveCount(0);
  };

  await plugbtn.click();
  await menuRow("caveman").click();
  await reopen();

  await plugbtn.click();
  await menuRow("ponytail").click();
  // Both picks survive the re-open: the resolved provenance marks each row as on.
  await expect(page.locator(".menu")).toBeVisible();
  await expect(menuRow("caveman").locator(".mc svg")).toBeVisible();
  await expect(menuRow("ponytail").locator(".mc svg")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".menu")).toHaveCount(0);

  // Two deltas over the inherited base summarize as "+2", and each toggle saved on its own.
  await expect(plugbtn).toContainText("+2");
  expect(await page.evaluate(() => (window as unknown as { mock: { writes: () => number } }).mock.writes())).toBe(2);

  // The MCP axis is independent: a single pick shows its own id and leaves the plugin label intact.
  await mcpbtn.click();
  await menuRow("capim-ds").click();
  await reopen();
  await expect(mcpbtn).toContainText("capim-ds");
  await expect(plugbtn).toContainText("+2");
});

/// Servers Claude Code loads from the person's CLI configuration appear in the picker as the visible
/// inherited base (ADR 0044), badged as such and removable without importing them first.
test("o picker mostra a base herdada do CLI e remove sem importar", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const mcpbtn = page.locator("#chatwrap .mcpbtn");
  const menuRow = (name: string) => page.locator(".menu .mrow").filter({ hasText: name }).first();
  await mcpbtn.click();
  await expect(page.locator(".menu")).toBeVisible();
  // Discovered CLI servers join the hub rows, checked and badged as inherited from the CLI.
  await expect(menuRow("metabase")).toContainText("herdado do CLI");
  await expect(menuRow("metabase").locator(".mc svg")).toBeVisible();
  await expect(menuRow("n8n")).toContainText("herdado do CLI");
  // The mixed universe reads as two labeled sections rather than one flat list.
  const headers = page.locator(".menu .mrow.off");
  await expect(headers.filter({ hasText: "Cadastrados" })).toHaveCount(1);
  await expect(headers.filter({ hasText: "Herdados do CLI" })).toHaveCount(1);

  // Unchecking a CLI server writes a removal into the workspace layer, keeping inherit as the base.
  await menuRow("metabase").click();
  await expect(page.locator(".menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".menu")).toHaveCount(0);
  await expect(mcpbtn).toContainText("−1");

  const layer = await page.evaluate(async () => {
    type Invoke = (command: string) => Promise<Board>;
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const board = await invoke("load_board");
    return board.workspaces.find((w) => w.title === "Ola")!.mcp;
  });
  expect(layer).toEqual({ base: "inherit", add: [], remove: ["metabase"] });
});

/// The desk supports direct replies and preserves panel order, size and collapse state across reloads.
test("a mesa mostra cada conversa num quadro, responde dali e guarda a ordem", async ({ page }) => {
  await boot(page);
  const tiles = page.locator("#tiles .tile");
  await expect(tiles).toHaveCount(6);
  await expect(page.locator("#railbody .navitem", { hasText: "Mesa" })).toHaveClass(/\bon\b/);

  // Include live local conversations, excluding archived, cleaned and remote workspaces.
  await expect(page.locator('#tiles .tile[data-tab="t7"]')).toHaveCount(0);
  const first = page.locator('#tiles .tile[data-tab="t1"]');
  await expect(first.locator(".tile-head")).toContainText("Ola");
  await expect(first.locator(".feed .turn")).not.toHaveCount(0);

  // A desk reply reaches its conversation.
  const composer = first.locator(".composer textarea");
  await composer.fill("Oi da mesa");
  await composer.press("Enter");
  await expect(first.locator(".feed")).toContainText("Entendi: Oi da mesa");
  await expect(page.locator('#tiles .tile[data-tab="t3"] .feed')).not.toContainText("Oi da mesa");

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

/// Dropping a file on the desk attaches it only to the panel beneath the pointer.
test("arquivo solto num quadro da mesa vira anexo daquela conversa", async ({ page }) => {
  await boot(page);
  const tile = page.locator('#tiles .tile[data-tab="t3"]');
  const composer = tile.locator(".composer textarea");
  await composer.fill("Olha esta captura");
  const at = (await tile.locator(".feed").boundingBox())!;

  const path = "/Users/eu/Desktop/Captura de Tela.png";
  await page.evaluate(({ path, x, y }) => {
    const mock = (window as unknown as {
      mock: { drop: (paths: string[], x: number, y: number, dropX: number, dropY: number) => void };
    }).mock;
    mock.drop([path], x, y, x, y);
  }, { path, x: at.x + at.width / 2, y: at.y + at.height / 2 });

  await expect(tile.locator(".cfiles .injchip")).toHaveCount(1);
  await expect(tile.locator(".cfiles .injchip")).toContainText("Captura de Tela.png");
  await expect(page.locator('#tiles .tile[data-tab="t1"] .cfiles .injchip')).toHaveCount(0);
  await expect(composer).toHaveValue("Olha esta captura");

  await composer.press("Enter");
  const bubble = tile.locator(".turn.user .bubble").last();
  await expect(bubble).toContainText("Olha esta captura");
  expect(await bubble.textContent()).toBe('@"/Users/eu/Desktop/Captura de Tela.png"\n\nOlha esta captura');
});

/// The first prompt can arrive live and in the initial snapshot. Sequence filtering must prevent
/// duplication.
test("a fala que chega durante o snapshot não entra duas vezes", async ({ page }) => {
  await boot(page);

  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    type WindowWithHold = Window & { __TAURI_INTERNALS__: { invoke: Invoke }; snapshotSegurado?: boolean };
    const w = window as WindowWithHold;
    const original = w.__TAURI_INTERNALS__.invoke;
    let first = true;
    w.__TAURI_INTERNALS__.invoke = async function (command, args, options) {
      if (first && command === "chat_snapshot" && args?.session === "t1") {
        first = false;
        w.snapshotSegurado = true;
        await new Promise((resolve) => setTimeout(resolve, 300));
      }
      return original.call(this, command, args, options);
    };
  });

  await page.locator("#railbody .navitem.sub .lbl").getByText("Ola", { exact: true }).click();
  await expect.poll(() => page.evaluate(() => !!(window as Window & { snapshotSegurado?: boolean }).snapshotSegurado)).toBe(true);
  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "FALA_DO_LANCAMENTO" } });
  });

  await expect(page.locator("#chatwrap .bubble", { hasText: "FALA_DO_LANCAMENTO" })).toHaveCount(1);
  await page.waitForTimeout(400);
  await expect(page.locator("#chatwrap .bubble", { hasText: "FALA_DO_LANCAMENTO" })).toHaveCount(1);
});

/// A tab can reattach while an older snapshot is pending. Attachment generation, not tab ID alone,
/// determines which response can render.
test("a mesa ignora um snapshot atrasado da mesma conversa", async ({ page }) => {
  await boot(page);
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  await expect(page.locator("#wsView")).toBeVisible();

  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "SNAPSHOT_ANTIGO" } });
  });
  await expect(page.locator('#chatwrap .bubble', { hasText: "SNAPSHOT_ANTIGO" })).toBeVisible();

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

  await page.locator("#railbody .navitem", { hasText: "Mesa" }).click();
  await expect.poll(() => page.evaluate(() => !!(window as Window & { deskSnapshotCaptured?: boolean }).deskSnapshotCaptured)).toBe(true);
  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { line: (tab: string, line: unknown) => void } }).mock;
    mock.line("t1", { type: "user", message: { role: "user", content: "SNAPSHOT_NOVO" } });
  });
  await page.locator("#railbody .navitem", { hasText: "Mesa" }).click();

  const tile = page.locator('#tiles .tile[data-tab="t1"]');
  await expect(tile.locator(".bubble", { hasText: "SNAPSHOT_NOVO" })).toHaveCount(1);
  await expect(tile.locator(".bubble", { hasText: "SNAPSHOT_ANTIGO" })).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => !!(window as Window & { deskSnapshotReturned?: boolean }).deskSnapshotReturned)).toBe(true);
  await expect(tile.locator(".bubble", { hasText: "SNAPSHOT_ANTIGO" })).toHaveCount(1);
});

/// The desk persists each workspace's MCP pick independently; the provenance picker opens asynchronously
/// and re-opens after each write (ADR 0043, phases 6 and 7).
test("a mesa grava escolhas de MCP em workspaces diferentes", async ({ page }) => {
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
test("a mesa mantém rascunho e anexo ao abrir o workspace", async ({ page }) => {
  await boot(page);
  const tile = page.locator('#tiles .tile[data-tab="t1"]');
  const draft = "Continuar esta fala no workspace";
  await tile.locator(".composer textarea").fill(draft);
  const at = (await tile.locator(".feed").boundingBox())!;
  await page.evaluate(({ x, y }) => {
    (window as unknown as { mock: { drop: (paths: string[], x: number, y: number) => void } }).mock.drop(
      ["/Users/eu/Desktop/contexto.txt"],
      x,
      y,
    );
  }, { x: at.x + at.width / 2, y: at.y + at.height / 2 });
  await expect(tile.locator(".cfiles .injchip")).toContainText("contexto.txt");

  await tile.locator(".topen").click();
  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#chatwrap .composer textarea")).toHaveValue(draft);
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText("contexto.txt");
});

/// Setup and Run stay in the side panel. Free terminals occupy center tabs and can remain alive
/// simultaneously.
test("terminal livre é aba do centro e o Setup fica no painel da direita", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  // Opening a workspace selects its conversation and displays Setup on the right.
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(page.locator("#termview")).toBeHidden();
  await expect(page.locator("#dockstrip .docktab.on")).toHaveText("Setup");

  // The add dropdown opens each new shell in another center tab.
  await page.locator(".tabadd .caret").click();
  await page.locator(".menu .mrow", { hasText: "Terminal novo" }).click();
  const tab = page.locator("#tabbar .tab").filter({ hasText: "Terminal" }).first();
  await expect(tab).toHaveClass(/on/);
  await expect(page.locator("#termview")).toBeVisible();
  await expect(page.locator("#chatwrap")).toBeHidden();
  // Opening a terminal preserves Setup in the side panel.
  await expect(page.locator("#dockstrip .docktab")).toHaveText(["Setup", "Run"]);
  await expect(page.locator("#dock")).toBeVisible();

  // Returning to the conversation keeps the terminal running.
  await page.locator('#tabbar .tab[data-tab="t1"]').click();
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(page.locator("#termview")).toBeHidden();
  await expect(tab).toBeVisible();

  // Closing the last terminal returns the center to the conversation.
  await tab.hover();
  await tab.locator(".tabx").click();
  await expect(page.locator("#tabbar .tab").filter({ hasText: "Terminal" })).toHaveCount(0);
  await expect(page.locator("#chatwrap")).toBeVisible();
});

/// Changes opens only when requested and stays closed after dismissal, even while the agent keeps
/// editing.
test("a aba de Mudanças só existe depois que você a abre", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Contratação pelo portal");

  const tab = page.locator("#tabbar .tab").filter({ hasText: "Alterações" });
  // An initially dirty worktree shows a Changes count without opening a Changes tab.
  await expect(page.locator("#diffcount")).not.toBeEmpty();
  await expect(tab).toHaveCount(0);

  // The next Changes click opens the central diff and creates its tab.
  await page.locator("#tab-diff").click();
  await page.locator("#tab-diff").click();
  await expect(page.locator("#diffview")).toBeVisible();
  await expect(tab).toHaveCount(1);

  await tab.hover();
  await tab.locator(".tabx").click();
  await expect(tab).toHaveCount(0);
  await expect(page.locator("#chatwrap")).toBeVisible();
  // Further dirty state does not reopen a dismissed Changes tab.
  await expect(page.locator("#diffcount")).not.toBeEmpty();
  await expect(tab).toHaveCount(0);
});
