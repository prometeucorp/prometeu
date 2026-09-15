import { expect, test, type Locator } from "@playwright/test";
import type { Board, Choice } from "../src/types";

type ComposerWindow = Window & {
  mock: { line: (tab: string, event: unknown) => void };
  controls: unknown[];
  __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
};

async function fits(composer: Locator) {
  await expect(composer.getByRole("button", { name: /^(Enviar|Send)$/ })).toBeVisible();
  expect(await composer.evaluate(root => {
    const outer = root.getBoundingClientRect();
    const controls = [...root.querySelectorAll("button")]
      .filter(button => button.getClientRects().length)
      .map(button => button.getBoundingClientRect());
    return root.scrollWidth <= root.clientWidth && controls.every((box, i) =>
      box.left >= outer.left && box.right <= outer.right && box.bottom <= outer.bottom &&
      controls.slice(i + 1).every(other => box.right <= other.left || other.right <= box.left || box.bottom <= other.top || other.bottom <= box.top));
  })).toBe(true);
  const send = await composer.locator(".send").boundingBox();
  expect(send!.width).toBe(28);
  expect(send!.height).toBe(28);
}

for (const [agent, model, locale, remoteLabel, busyLabel] of [
  ["claude", "opus[1m]", "pt-BR", "Controle remoto", "Trabalhando…"],
  ["codex", "gpt-5.6-sol", "en", "Remote control", "Working…"],
] as const) {
  test(`rodapé da conversa mantém controles acessíveis sem sobreposição: ${agent}, ${locale}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1600, height: 1000 });
    await page.addInitScript(locale => {
      localStorage.setItem("prometeu:idioma", locale);
      localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "user1", name: "Alice", email: "alice@example.com" }, origin: "https://app.prometeu.co", offline: false }));
      localStorage.setItem("mock:organizations", JSON.stringify([
        { id: "organization1", slug: "one", name: "One", member: "membership1", role: "owner" },
      ]));
    }, locale);
    await page.goto("/");
    await page.locator('.railworkspace[data-workspace="sessao-0929"] > .navitem').click();
    await page.evaluate(async choice => {
      const w = window as ComposerWindow;
      const { invoke } = w.__TAURI_INTERNALS__;
      const board = await invoke("load_board") as Board;
      const workspace = board.workspaces[0];
      workspace.tabs[0].choice = choice;
      workspace.tabs[0].status = "rodando";
      workspace.mcp = { base: "none", add: ["capim-ds"], remove: [] };
      workspace.plugins = { base: "none", add: ["caveman", "ponytail"], remove: [] };
      await invoke("set_stage", { id: workspace.id, stage: workspace.stage });
      w.mock.line("t1", { v: 1, at: 1, type: "session.state", state: "busy" });
      w.controls = [];
      w.__TAURI_INTERNALS__.invoke = (command, args) => {
        if (command === "chat_control") w.controls.push(args);
        return invoke(command, args);
      };
    }, { agent, model, effort: "xhigh" } satisfies Choice);

    const composer = page.locator("#chatwrap .composer");
    const remote = composer.getByRole("button", { name: remoteLabel, exact: true });
    await expect(composer.getByRole("status")).toHaveText(busyLabel);
    await expect(remote).toHaveAttribute("aria-pressed", "false");
    await remote.focus();
    await remote.press("Space");
    await expect(remote).toHaveAttribute("aria-pressed", "true");
    await expect(remote).not.toHaveCSS("box-shadow", "none");

    for (const width of [860, 600, 440, 320]) {
      await composer.evaluate((root, width) => { root.style.width = `${width}px`; }, width);
      await fits(composer);
      if (width === 860 || width === 320) await composer.screenshot({ path: testInfo.outputPath(`composer-${width}.png`) });
    }
    await expect(remote.locator("span")).toBeHidden();
    await remote.press("Enter");
    await expect(remote).toHaveAttribute("aria-pressed", "false");
    await expect(remote).toHaveCSS("box-shadow", "none");
    await composer.getByRole("button", { name: /^(Parar|Stop)$/ }).click();
    expect(await page.evaluate(() => (window as ComposerWindow).controls)).toEqual([
      { session: "t1", frame: { v: 1, type: "turn.interrupt" } },
    ]);

    await composer.locator("textarea").fill("Mensagem pelo rodapé");
    await composer.getByRole("button", { name: /^(Enviar|Send)$/ }).click();
    await expect(page.locator("#chatwrap .feed")).toContainText("Entendi: Mensagem pelo rodapé");

    await page.locator("#railbody .navitem", { hasText: /^(Mesa|Desk)$/ }).click();
    const tile = page.locator('#tiles .tile[data-tab="t1"]');
    await tile.evaluate(root => { root.style.width = "320px"; });
    await fits(tile.locator(".composer"));
    await expect(tile.getByRole("button", { name: remoteLabel, exact: true })).toBeVisible();
  });
}
