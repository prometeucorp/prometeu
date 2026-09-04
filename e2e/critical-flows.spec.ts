import { expect, test, type Page } from "@playwright/test";

async function boot(page: Page) {
  await page.goto("/");
  await expect(page.locator("#issuesView")).toBeVisible();
  await expect(page.locator("#railbody .navitem.sub").first()).toBeVisible();
}

async function openWorkspace(page: Page, title: string) {
  await page.locator("#railbody .navitem.sub .lbl").getByText(title, { exact: true }).click();
  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#crumb")).toContainText(title);
}

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
  await expect(railWorkspace.locator(".provider svg")).toBeVisible();
  await expect(railWorkspace.locator(".dot")).toHaveCount(0);
  await expect(railWorkspace).toHaveAttribute("title", /Claude Code/);
  await expect(railWorkspace).not.toHaveAttribute("title", /Pronta|Rodando|Desligada/);
  await openWorkspace(page, "Ola");

  // Este workspace nunca foi compartilhado: colaboração não vira um estado
  // permanente na barra. A ação só entra no menu quando há um time configurado.
  await expect(page.locator("#msg")).toBeHidden();
  await expect(page.locator("#share")).toBeHidden();
  await expect(page.locator("#wsmore")).toBeVisible();
  await page.locator("#wsmore").click();
  await expect(page.locator(".menu .mrow", { hasText: "Definir etapa" })).toBeVisible();
  await page.keyboard.press("Escape");

  // Um evento do quadro redesenha o app inteiro. O nó da identidade deve
  // sobreviver em vez de sumir e nascer de novo a cada ferramenta do agente.
  await page.locator("#crumb .nm").evaluate((el) => (el.dataset.stable = "yes"));
  await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const invoke = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__.invoke;
    await invoke("set_stage", { id: "sessao-0929", stage: "Fazendo" });
  });
  await expect(page.locator("#crumb .nm")).toHaveAttribute("data-stable", "yes");
  await expect(page.locator("#msg")).toBeHidden();
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

  // Faz o snapshot de t2 chegar depois de t1. É a ordem que antes conseguia
  // repintar a conversa errada ao clicar rapidamente entre abas.
  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = async function (command, args, options) {
      if (command === "chat_buffer" && args?.session === "t2") {
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

  await page.locator("#railbody > button.navitem").first().click();
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

  // Uma lista longa revela os dois limites do popup: a lateral da folha e o
  // início do rodapé. O mock normal é curto demais e não força rolagem, então
  // ele é repetido — quantas vezes sai do que o mock traz, para uma issue nova
  // no mock não virar um número errado aqui.
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
    // Conectar primeiro: o mock recusa a busca enquanto o Linear está fora.
    await internals.invoke("linear_connect");
    const found = (await internals.invoke("linear_issues", { force: false })) as { issues: unknown[] };
    return found.issues.length;
  });
  // A lista precisa passar do que cabe na tela; é disso que o teste trata.
  expect(total).toBeGreaterThanOrEqual(20);

  await expect(page.locator("#railbody .navitem", { hasText: "Issues" }).locator(".n")).toHaveText(String(total));

  await page.locator("#railbody > button.navitem").first().click();
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

  await page.locator("#railbody > button.navitem").first().click();
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

/// O "@" da caixa aponta um arquivo do workspace para o agente. O que importa
/// aqui é a caixa acabar com um caminho de verdade escrito nela: é isso que o
/// agente lê, e é o que faltava — a lista nunca abria.
test("o @ na caixa completa um caminho do workspace", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("veja @app/adapters/tra");

  const first = page.locator(".menu .mrow").first();
  await expect(first).toContainText("app/adapters/transcriber.rb");

  // Tab escreve o caminho inteiro no lugar do que foi digitado, e não manda a
  // fala.
  await composer.press("Tab");
  await expect(composer).toHaveValue("veja @app/adapters/transcriber.rb ");
  await expect(page.locator("#chatwrap .feed")).not.toContainText("veja @app");
});

/// O evento nativo do Tauri traz o caminho verdadeiro, mas a posição final do
/// drop pode vir deslocada no macOS. O alvo que já acendeu continua valendo, e
/// o arquivo usa o mesmo rascunho de anexos do botão "+" — sem invadir o texto.
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
    // O `over` está na caixa; o `drop` termina fora da viewport, como uma
    // coordenada nativa deslocada pela barra da janela.
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

/// Entre dois caminhos que combinam igual, o que o agente acabou de mexer vem
/// na frente: no meio de um trabalho, o "@" quase sempre é sobre o arquivo que
/// acabou de aparecer na conversa.
test("o arquivo que o agente acabou de ler sobe na lista do @", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const composer = page.locator("#chatwrap .composer textarea");
  await composer.fill("veja @waha");
  await expect(page.locator(".menu .mrow").first()).toContainText("app/adapters/waha.rb");

  // O agente lê outro arquivo da mesma pasta. A lista do "@" passa a oferecê-lo
  // primeiro, mesmo com "adapters" combinando igual nos dois.
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

/// Um workspace de três repositórios com cem arquivos mudados: o diff inteiro
/// são dezenas de milhares de linhas, e montá-las de uma vez travava a tela por
/// segundos e deixava a rolagem arrastando. O que este teste guarda é a regra —
/// só o que está perto da tela é montado — porque ela é invisível enquanto
/// funciona, e o que ela evita só aparece no workspace grande de alguém.
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
      if (command === "workspace_diff") {
        return [
          { name: "prometeu", base: "origin/main", ahead: 9, unpushed: 0, dirty: 0, files: files("um", 60) },
          { name: "njord", base: "origin/develop", ahead: 3, unpushed: 0, dirty: 0, files: files("dois", 50) },
        ];
      }
      return original.call(this, command, args, options);
    };
  });

  await openWorkspace(page, "Contratação pelo portal");
  // O diff é conferido de novo enquanto a tela dele está aberta: é por aí que
  // ele chega, sem depender de o agente mexer em nada.
  await page.locator("#tab-diff").click();
  await expect(page.locator("#difflist .diffrepo")).toHaveCount(2, { timeout: 10_000 });
  await expect(page.locator("#difflist .diffsum .state")).toContainText("tudo empurrado");

  await page.locator("#review").click();
  const dlist = page.locator("#dlist");
  await expect(dlist.locator(".dfile")).toHaveCount(110);
  // Os 110 cabeçalhos existem; as 6.600 linhas, não — só as de quem está perto
  // da tela. Sem preguiça isto passava de 40 mil nós.
  const linhas = await dlist.locator(".drow").count();
  expect(linhas).toBeGreaterThan(0);
  expect(linhas).toBeLessThan(2_000);

  // O lugar de cada arquivo já está guardado: a rolagem tem a altura do diff
  // inteiro antes de ele existir, e por isso não anda sozinha enquanto se lê.
  const altura = await dlist.evaluate((el) => el.scrollHeight);
  expect(altura).toBeGreaterThan(100_000);

  // Clicar num arquivo lá do fim da lista leva até ele — montado.
  await page.locator("#difflist .diffrow").last().click();
  await expect(dlist.locator(".dfile").last().locator(".drow").first()).toBeVisible();
});

/// O arquivo abre pronto para escrever — não há botão de editar. O que este
/// teste guarda é o que quebra sozinho: o quadro bate a cada ferramenta que o
/// agente usa e redesenha o arquivo aberto; se o redesenho não respeitar o que
/// está sendo escrito, o texto some no meio da frase.
test("escrever no arquivo aberto sobrevive ao redesenho do quadro e salva", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  await page.locator("#tab-files").click();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vpre")).toContainText("Controle financeiro pessoal");

  // Sem nada escrito não há o que salvar nem o que desfazer.
  await expect(page.locator("#vtext")).toBeVisible();
  await expect(page.locator("#vsave")).toBeHidden();
  await expect(page.locator("#vcancel")).toBeHidden();

  const texto = "# Njord\n\nCorrigido à mão pelo E2E.\n";
  await page.locator("#vtext").fill(texto);
  // As cores acompanham: o que se lê é o <pre>, e ele já mostra o texto novo.
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

  // Nem clicar em outro arquivo e voltar: o rascunho espera, e volta de onde
  // parou. Um clique errado não custa o que já foi escrito.
  await page.locator("#tree .treerow", { hasText: ".gitignore" }).click();
  await expect(page.locator("#vpre")).toContainText("Ignore bundler config");
  await expect(page.locator("#vsave")).toBeHidden();
  await page.locator("#tree .treerow", { hasText: "CLAUDE.md" }).click();
  await expect(page.locator("#vtext")).toHaveValue(texto);
  await expect(page.locator("#vsave")).toBeVisible();

  await page.locator("#vsave").click();
  await expect(page.locator("#vsave")).toBeHidden();
  await expect(page.locator("#vcrumb")).not.toHaveClass(/\bdirty\b/);

  // Salvou de verdade: o redesenho seguinte lê o disco e acha o que foi escrito.
  await board();
  await expect(page.locator("#vpre")).toContainText("Corrigido à mão pelo E2E.");
  await expect(page.locator("#vpre")).not.toContainText("Controle financeiro pessoal");

  // E o arquivo que só passou pela tela no meio da edição continua intacto:
  // salvar escreve no arquivo que está sendo editado, não no último aberto.
  await page.locator("#tree .treerow", { hasText: ".gitignore" }).click();
  await expect(page.locator("#vpre")).toContainText("Ignore bundler config");
  await expect(page.locator("#vpre")).not.toContainText("Corrigido à mão pelo E2E.");
});

/// Ler o diff e ir mexer no arquivo são o mesmo movimento: o duplo clique
/// atravessa da lista de Mudanças, e do diff empilhado no centro, para o
/// arquivo inteiro aberto no viewer.
test("duplo clique numa mudança abre o arquivo no viewer", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  await page.locator("#tab-diff").click();
  const row = page.locator("#difflist .diffrow", { hasText: "style.css" }).first();
  await expect(row).toBeVisible();

  // Um clique é ir até o arquivo no diff do centro, e não abrir.
  await row.click();
  await expect(page.locator("#dlist .dhead", { hasText: "style.css" }).first()).toBeVisible();
  await expect(page.locator("#viewer")).toBeHidden();

  await row.dblclick();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vcrumb")).toContainText("style.css");
  await expect(page.locator("#vpre")).toContainText("padding: 12px");

  // E o mesmo gesto no cabeçalho do arquivo dentro do diff empilhado.
  await page.locator("#tab-diff").click();
  await page.locator("#dlist .dhead", { hasText: "style.css" }).first().dblclick();
  await expect(page.locator("#viewer")).toBeVisible();
  await expect(page.locator("#vcrumb")).toContainText("style.css");
});

/// O filtro do que está fora de commit escondia repositório inteiro: num
/// workspace com três repos, se o que ainda não foi commitado estava só num
/// deles, os outros dois sumiam da lista sem deixar rastro — e a tela parecia
/// estar deixando de mostrar mudança. Agora o repositório continua ali,
/// dizendo que o dele está todo commitado.
test("o filtro de fora de commit não faz repositório sumir da lista", async ({ page }) => {
  await boot(page);

  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    const files = (repo: string, n: number, dirty: boolean) =>
      Array.from({ length: n }, (_, i) => ({
        path: `${repo}/arquivo${i}.ts`,
        added: 3,
        removed: 1,
        new_file: false,
        deleted: false,
        dirty,
        patch: "@@ -1,1 +1,1 @@\n-velho\n+novo",
      }));
    internals.invoke = async function (command, args, options) {
      if (command === "workspace_diff") {
        return [
          // Tudo commitado: é este que sumia quando o filtro ligava.
          { name: "prometeu", base: "origin/main", ahead: 9, unpushed: 0, dirty: 0, files: files("um", 8, false) },
          { name: "njord", base: "origin/develop", ahead: 4, unpushed: 0, dirty: 2, files: files("dois", 2, true) },
        ];
      }
      return original.call(this, command, args, options);
    };
  });

  await openWorkspace(page, "Contratação pelo portal");
  await page.locator("#tab-diff").click();

  const repos = page.locator("#difflist .diffrepo");
  await expect(repos).toHaveCount(2, { timeout: 10_000 });
  await expect(page.locator("#difflist .diffrow")).toHaveCount(10);

  // Liga o filtro: só os dois arquivos do njord ficam, mas os dois
  // repositórios continuam na lista.
  await page.locator("#difflist .diffsum .dirtyf").click();
  await expect(page.locator("#difflist .diffrow")).toHaveCount(2);
  await expect(repos).toHaveCount(2);
  await expect(repos.first()).toHaveClass(/\bquiet\b/);
  await expect(repos.first()).toContainText("prometeu");
  await expect(repos.first()).toContainText("tudo commitado");
});

/// O hub é um só: trocar de Claude para Codex muda o adapter, não a escolha do
/// workspace. MCP e plugins continuam visíveis para os dois providers.
test("escolher um GPT mantém o seletor de plugins do lançador", async ({ page }) => {
  await boot(page);
  await page.locator("#rail button, #railbody .navitem").filter({ hasText: "Criar" }).first().click();
  await expect(page.locator("#d-plugins")).toBeVisible();

  await page.locator("#d-model").click();
  await page.locator(".menu .mrow", { hasText: "GPT-5.6-Sol" }).first().click();
  await expect(page.locator("#d-plugins")).toBeVisible();
  await expect(page.locator("#d-mcp")).toBeVisible();
});

/// Trocar de modelo com a conversa de pé: a escolha entra na aba, o processo
/// cai e a caixa passa a dizer que escrever retoma. O esforço sobe um degrau
/// por clique, como no rodapé do lançador. Sair do CLI não se oferece — a
/// lista só tem os modelos do agente que já está de pé, porque o `--resume` do
/// Claude Code não abre a thread do Codex.
test("trocar o modelo de uma conversa de pé desliga o processo e mantém a aba", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  const model = page.locator(".composer .mdl");
  const effort = page.locator(".composer .effort");
  await expect(model).toContainText("Opus · 1M");
  await expect(effort).toContainText("Alto");

  await model.click();
  await expect(page.locator(".menu .mrow", { hasText: "GPT-5.6-Sol" })).toHaveCount(0);
  await page.locator(".menu .mrow", { hasText: "Sonnet" }).first().click();

  await expect(model).toContainText("Sonnet");
  await expect(page.locator(".composer textarea")).toHaveAttribute(
    "placeholder",
    /escrever retoma/,
  );
  // A aba continua ali: o que caiu foi o processo, não a conversa.
  await expect(page.locator("#tabbar .tab").first()).toContainText("conversa 1");

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

  const pills = page.locator("#iteams .tpill");
  await expect(pills).toHaveCount(3, { timeout: 10_000 });
  await expect(pills.first()).toHaveClass(/\bon\b/);
  await expect(page.locator("#ilist .irow")).toHaveCount(7);

  // Escolher um time deixa só as issues dele.
  await pills.filter({ hasText: "INF" }).click();
  await expect(page.locator("#ilist .irow")).toHaveCount(2);
  await expect(page.locator("#ilist .irow .iid").first()).toContainText("INF-");

  // Buscar não muda quais pílulas existem — muda quantas issues cada uma
  // mostraria. O time escolhido continua escolhido.
  await page.locator("#ibar input").fill("runner");
  await expect(pills).toHaveCount(3);
  await expect(pills.filter({ hasText: "INF" })).toHaveClass(/\bon\b/);
  await expect(pills.filter({ hasText: "MOA" })).toContainText("0");
  await expect(page.locator("#ilist .irow")).toHaveCount(1);

  // E voltar para "Todos" devolve o que a busca achou em qualquer time.
  await page.locator("#ibar input").fill("linear");
  await expect(page.locator("#ilist .iempty")).toBeVisible();
  await pills.first().click();
  await expect(page.locator("#ilist .irow")).toHaveCount(1);
});

/// Instalar plugin era assunto de fora do app: instalar no CLI, e depois
/// importar. Agora é daqui — nome, o que ele deve fazer, e um agente escreve a
/// pasta. O que este teste guarda é o caminho inteiro: o pedido, o que o
/// agente vai escrevendo (que é o que faz a espera parecer trabalho) e o
/// plugin já na lista no fim.
test("cria um plugin pelo Prometeu e ele entra na lista", async ({ page }) => {
  await boot(page);
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Plugins" }).click();
  await page.locator(".setrow.head button", { hasText: "Criar plugin" }).click();

  await page.locator(".sheet.hubedit input").fill("Diário do dia");
  await page.locator(".sheet.hubedit textarea").fill("Um comando que resume o dia num arquivo datado.");
  await page.locator(".sheetbar button", { hasText: "Criar" }).click();

  // Enquanto ele escreve, a folha mostra o que está saindo — e o pedido sai da
  // frente, para ninguém achar que ainda pode mexer nele.
  await expect(page.locator(".sheet.hubedit .mstep").first()).toBeVisible();
  await expect(page.locator(".sheet.hubedit textarea")).toHaveCount(0);
  await expect(page.locator(".sheet.hubedit .mstep", { hasText: "plugin.json" })).toBeVisible();

  // No fim a folha fecha sozinha e o plugin está cadastrado, com a pasta do
  // Prometeu como origem.
  await expect(page.locator(".sheet.hubedit")).toHaveCount(0);
  const row = page.locator(".setrow", { hasText: "diario-do-dia" });
  await expect(row).toBeVisible();
  await expect(row).toContainText("~/.prometeu/plugins/diario-do-dia");
});

/// Instalar plugin era assunto de fora do app: instalar no CLI e importar
/// depois. Agora é o endereço do repositório e mais nada — e o repositório que
/// traz vários pergunta quais antes de cadastrar.
test("instala um plugin pelo endereço do repositório", async ({ page }) => {
  await boot(page);
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Plugins" }).click();

  await page.locator(".setrow.head button", { hasText: "Instalar plugin" }).click();
  await page.locator(".sheet.hubedit input").fill("gbrancaglione/exemplo");
  await page.locator(".sheetbar button", { hasText: "Instalar" }).click();

  // Um plugin só não é escolha: ele entra e a folha fecha.
  await expect(page.locator(".sheet.hubedit")).toHaveCount(0);
  const row = page.locator(".setrow", { hasText: "exemplo" });
  await expect(row).toContainText("github.com/gbrancaglione/exemplo");
  await expect(row.locator("button", { hasText: "Atualizar" })).toBeVisible();

  // O repositório com vários pergunta quais — e só entra o que foi marcado.
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

/// Marcar plugin numa conversa que já existe: a marca é da tela, e não da
/// resposta do back. Cada gravação derruba o processo da conversa e republica
/// o quadro inteiro; esperar por ela para mover a marca fazia o menu parecer
/// travado — e marcar três coisas seguidas fazia isso três vezes.
test("marcar plugins na conversa responde na hora e grava uma vez só", async ({ page }) => {
  await boot(page);
  await openWorkspace(page, "Ola");

  await page.locator(".plugbtn").click();
  const row = (name: string) => page.locator(".menu .mrow").filter({ hasText: name }).first();
  await row("caveman").click();
  // O quadro ainda não voltou, e a marca já está no lugar novo.
  await expect(row("caveman").locator(".mc svg")).toBeVisible();
  await row("ponytail").click();
  await expect(row("caveman").locator(".mc svg")).toBeVisible();
  await expect(row("ponytail").locator(".mc svg")).toBeVisible();

  // Fechado o menu, a tela alcança o quadro: os dois cliques viraram uma
  // gravação, e o rodapé conta os dois.
  await page.keyboard.press("Escape");
  await expect(page.locator(".plugbtn")).toContainText("2 plugins");
  expect(await page.evaluate(() => (window as unknown as { mock: { writes: () => number } }).mock.writes())).toBe(1);

  // Mexer no MCP logo em seguida é outra gravação, e não a mesma: uma espera
  // não pode engolir a outra.
  await page.locator(".mcpbtn").click();
  await page.locator(".menu .mrow").filter({ hasText: "capim-ds" }).first().click();
  await page.keyboard.press("Escape");
  await expect(page.locator(".mcpbtn")).toContainText("capim-ds");
  await expect(page.locator(".plugbtn")).toContainText("2 plugins");
});
