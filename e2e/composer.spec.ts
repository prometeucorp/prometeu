import { expect, test, type Locator } from "./fixtures";
import type { Board, Choice } from "../src/types";

type ComposerWindow = Window & {
  mock: { line: (tab: string, event: unknown) => void; exitAgent: (tab: string) => void };
  controls: unknown[];
  nativePastesUntilFailure?: number;
  __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
};

// Real xterm mouse propagation, paste modes, flex sizing and focus need browser coverage.
test("native conversation keeps a full terminal and opens product tools on demand @webkit", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.locator('#tiles .tile[data-tab="t1"]')).toBeVisible();
  await page.evaluate(async () => {
    const w = window as ComposerWindow;
    const { invoke } = w.__TAURI_INTERNALS__;
    const board = await invoke("load_board") as Board;
    board.workspaces[0].tabs[0].conversation_mode = "terminal";
    board.workspaces[0].tabs[0].choice = { agent: "codex", model: "gpt-5.6-sol", effort: "xhigh" };
    w.controls = [];
    w.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "agent_terminal_snapshot" && args?.session === "t1") {
        return invoke(command, args).then(snapshot => ({
          ...snapshot as object,
          data: [...new TextEncoder().encode("\x1b[?2004h\x1b[?1003h\x1b[?1006h\x1b]8;;https://example.com/review\x07Review\x1b]8;;\x07\r\nMock agent terminal")],
        }));
      }
      if (command === "open_external") {
        w.controls.push({ command, ...args });
        return Promise.resolve();
      }
      if (command === "pty_write" && args?.session === "t1" && typeof args.data === "string"
        && args.data.startsWith("\x1b[200~") && w.nativePastesUntilFailure && --w.nativePastesUntilFailure === 0) {
        return Promise.reject('i18n:{"code":"err.terminalAgent.notReady"}');
      }
      if ((command === "pty_write" || command === "pty_resize" || command === "chat_send") && args?.session === "t1") w.controls.push({ command, ...args });
      return invoke(command, args);
    };
    await invoke("set_unread", { id: "sessao-0929", unread: false });
  });
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  const terminal = page.locator("#chatwrap .conversation-terminal");
  const input = terminal.locator(".xterm-helper-textarea");
  await expect(terminal).toBeVisible();
  await expect(input).toBeFocused();
  const bounds = await terminal.boundingBox();
  expect(bounds!.height).toBeGreaterThan(200);

  // One link gesture must not also reach the CLI, which can open the same URL itself.
  // Mocked xterm unit tests cannot prove screen/terminal/document listener ordering.
  const screen = terminal.locator(".xterm-screen");
  const screenBounds = (await screen.boundingBox())!;
  await page.mouse.move(screenBounds.x + 10, screenBounds.y + 8);
  await expect(terminal).toHaveAttribute("title", "https://example.com/review");
  await page.evaluate(() => { (window as ComposerWindow).controls = []; });
  await page.mouse.down();
  await page.mouse.up();
  const linkControls = await page.evaluate(() => (window as ComposerWindow).controls) as { command: string; data?: string; url?: string }[];
  expect(linkControls.filter(value => value.command === "open_external")).toEqual([
    { command: "open_external", url: "https://example.com/review" },
  ]);
  expect(linkControls.filter(value => value.command === "pty_write" && /^\x1b\[<0;/.test(value.data ?? ""))).toEqual([]);

  await page.mouse.click(screenBounds.x + 10, screenBounds.y + 30);
  await expect.poll(() => page.evaluate(() => (window as ComposerWindow).controls.some(value =>
    (value as { command: string }).command === "pty_write" && /^\x1b\[<0;/.test((value as { data?: string }).data ?? "")))).toBe(true);
  // Restore ordinary terminal input for the remaining composer checks.
  await page.evaluate(async () => {
    const w = window as ComposerWindow;
    const control = w.controls.find(value => (value as { generation?: string }).generation) as { generation: string };
    await w.__TAURI_INTERNALS__.invoke("pty_write", { session: "t1", generation: control.generation, data: "\x1b[?1003l\x1b[?1006l" });
    w.controls = [];
  });
  await page.keyboard.type("native input");
  await expect.poll(() => page.evaluate(() => (window as ComposerWindow).controls.filter(value => (value as { command: string }).command === "pty_write").length)).toBeGreaterThan(0);
  expect(await page.evaluate(() => (window as ComposerWindow).controls.every(value => typeof (value as { generation?: string }).generation === "string"))).toBe(true);

  // Open PR drafts into the native composer instead of queueing behind pending input.
  const prompt = await page.evaluate(() => (window as ComposerWindow).__TAURI_INTERNALS__.invoke("pr_prompt", { id: "sessao-0929" })) as string;
  await page.locator("#pr").click();
  await expect.poll(() => page.evaluate(text => (window as ComposerWindow).controls.some(value =>
    (value as { command: string; data?: string }).command === "pty_write" && (value as { data?: string }).data === `\x1b[200~${text}\x1b[201~`), prompt)).toBe(true);
  expect(await page.evaluate(() => (window as ComposerWindow).controls.some(value => (value as { command: string }).command === "chat_send"))).toBe(false);
  await expect(input).toBeFocused();

  const conversation = page.locator("#chatwrap");
  await expect(conversation.getByRole("button", { name: "Transcript and controls", exact: true })).toHaveCount(0);
  await expect(conversation.getByRole("button", { name: "Terminal", exact: true })).toHaveCount(0);
  await expect(conversation.locator(".feed")).toBeHidden();
  await expect(conversation.locator(".composer")).toBeHidden();
  await expect(conversation.locator(".mdl")).toBeHidden();
  await expect(conversation.locator(".effort")).toBeHidden();
  await expect(conversation.locator(".conversation-surface-toolbar")).toHaveCount(0);
  await page.locator("#wsmore").click();
  await page.getByRole("menuitem", { name: "Conversation tools", exact: true }).click();
  const tools = page.getByRole("dialog", { name: "Conversation tools", exact: true });
  await expect(tools.locator(".actionsbtn")).toBeVisible();
  await expect(tools.locator(".mdl")).toBeHidden();
  await expect(tools.locator(".effort")).toBeHidden();
  await expect(tools.locator(".composer textarea")).toBeHidden();

  await page.evaluate(() => { (window as ComposerWindow).controls = []; });
  await tools.locator(".addfile").click();
  await expect(tools).toBeHidden();
  await expect(input).toBeFocused();
  await expect(conversation.locator(".cfiles")).toBeHidden();
  const writes = await page.evaluate(() => (window as ComposerWindow).controls
    .filter(value => (value as { command: string }).command === "pty_write")) as { data: string; generation: string }[];
  expect(writes).toHaveLength(2);
  expect(writes[0].data).toMatch(/^\x1b\[200~\n@[^\n]*spec\.md\n\x1b\[201~$/);
  expect(writes[1].data).toBe("\x1b[200~/Users/gustavo/Desktop/screenshot.png\x1b[201~");
  expect(writes.every(write => !write.data.includes("\r") && write.generation === writes[0].generation)).toBe(true);
  await expect(conversation.locator(".composer")).toBeHidden();
  expect((await terminal.boundingBox())!.height).toBe(bounds!.height);

  // A failed second paste retains that file without duplicating the document already inserted.
  // Only the browser establishes modal focus and recovery through the real xterm input boundary.
  await page.locator('#tabbar .tab[data-tab="t1"]').click({ button: "right" });
  await page.getByRole("menuitem", { name: "Conversation tools", exact: true }).click();
  await page.evaluate(() => {
    const w = window as ComposerWindow;
    w.controls = [];
    w.nativePastesUntilFailure = 2;
  });
  await tools.locator(".addfile").click();
  await expect(tools.locator(".cfiles")).not.toContainText("spec.md");
  await expect(tools.locator(".cfiles")).toContainText("screenshot.png");
  const insert = tools.getByRole("button", { name: "Insert into terminal", exact: true });
  await expect(insert).toBeEnabled();
  await insert.click();
  await expect(tools).toBeHidden();
  await expect(input).toBeFocused();
  await expect(conversation.locator(".cfiles")).toBeHidden();
  const retried = await page.evaluate(() => (window as ComposerWindow).controls
    .filter(value => (value as { command: string }).command === "pty_write")) as { data: string }[];
  expect(retried).toHaveLength(2);
  expect(retried[0].data).toBe(writes[0].data);
  expect(retried[1].data).toBe(writes[1].data);

  await page.locator("#railbody .navitem", { hasText: "Desk" }).click();
  await page.locator('#tiles .tile[data-tab="t2"] .topen').click();
  await expect(page.locator("#chatwrap .composer textarea")).toBeVisible();
  await expect(page.locator("#chatwrap .conversation-terminal")).toBeHidden();
});

async function fits(composer: Locator) {
  await expect(composer.getByRole("button", { name: "Send", exact: true })).toBeVisible();
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

test("open pr resumes an exited native agent instead of drafting into it", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator('#tiles .tile[data-tab="t1"]')).toBeVisible();
  await page.evaluate(async () => {
    const w = window as ComposerWindow;
    const { invoke } = w.__TAURI_INTERNALS__;
    const board = await invoke("load_board") as Board;
    board.workspaces[0].tabs[0].conversation_mode = "terminal";
    w.controls = [];
    w.__TAURI_INTERNALS__.invoke = (command, args) => {
      if ((command === "pty_write" || command === "chat_send") && args?.session === "t1") w.controls.push({ command, ...args });
      return invoke(command, args);
    };
    await invoke("set_unread", { id: "sessao-0929", unread: false });
  });
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  await expect(page.locator("#chatwrap .conversation-terminal .xterm-helper-textarea")).toBeFocused();
  await page.evaluate(() => (window as ComposerWindow).mock.exitAgent("t1"));
  const prompt = await page.evaluate(() => (window as ComposerWindow).__TAURI_INTERNALS__.invoke("pr_prompt", { id: "sessao-0929" })) as string;
  await page.locator("#pr").click();
  await expect.poll(() => page.evaluate(() => (window as ComposerWindow).controls)).toEqual([{ command: "chat_send", session: "t1", text: prompt }]);
});

test("conversation footer keeps controls accessible without overlap", { tag: "@webkit" }, async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.addInitScript(() => {
    localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "user1", name: "Alice", email: "alice@example.com" }, origin: "https://app.prometeu.co", offline: false }));
    localStorage.setItem("mock:organizations", JSON.stringify([
      { id: "organization1", slug: "one", name: "One", member: "membership1", role: "owner" },
    ]));
  });
  await page.goto("/");
  await expect(page.locator("#tiles .tile").first()).toBeVisible();
  // Cloud discovery can rebuild the sidebar between pointer down and click; desk tiles stay mounted.
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  // Wait for the initial snapshot before emitting live state, or its ready state can overwrite busy.
  await expect(page.locator("#chatwrap .feed .turn").first()).toBeVisible();
  await page.evaluate(async choice => {
    const w = window as ComposerWindow;
    const { invoke } = w.__TAURI_INTERNALS__;
    const board = await invoke("load_board") as Board;
    const workspace = board.workspaces[0];
    workspace.tabs[0].choice = choice;
    workspace.tabs[0].status = "rodando";
    workspace.mcp = { base: "none", add: ["capim-ds"], remove: [] };
    workspace.plugins = { base: "none", add: ["caveman", "ponytail"], remove: [] };
    await invoke("set_unread", { id: workspace.id, unread: false });
    w.mock.line("t1", { v: 1, at: 1, type: "session.state", state: "busy" });
    w.controls = [];
    w.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "chat_control") w.controls.push(args);
      return invoke(command, args);
    };
  }, { agent: "codex", model: "gpt-5.6-sol", effort: "xhigh" } satisfies Choice);

  const composer = page.locator("#chatwrap .composer");
  const remote = composer.getByRole("button", { name: "Remote control", exact: true });
  await expect(composer.getByRole("status")).toHaveText("Working…");
  await expect(remote).toHaveAttribute("aria-pressed", "false");
  await remote.focus();
  await remote.press("Space");
  await expect(remote).toHaveAttribute("aria-pressed", "true");
  await expect(remote).not.toHaveCSS("box-shadow", "none");

  for (const width of [860, 600, 440, 320]) {
    await composer.evaluate((root, width) => { root.style.width = `${width}px`; }, width);
    await fits(composer);
  }
  await expect(remote.locator("span")).toBeHidden();
  await remote.press("Enter");
  await expect(remote).toHaveAttribute("aria-pressed", "false");
  await expect(remote).toHaveCSS("box-shadow", "none");
  await composer.getByRole("button", { name: "Stop", exact: true }).click();
  expect(await page.evaluate(() => (window as ComposerWindow).controls)).toEqual([
    { session: "t1", frame: { v: 1, type: "turn.interrupt" } },
  ]);

  await composer.locator("textarea").fill("Message from the footer");
  await composer.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.locator("#chatwrap .feed")).toContainText("Understood: Message from the footer");

  await page.locator("#railbody .navitem", { hasText: "Desk" }).click();
  const tile = page.locator('#tiles .tile[data-tab="t1"]');
  await tile.evaluate(root => { root.style.width = "320px"; });
  await fits(tile.locator(".composer"));
  await expect(tile.getByRole("button", { name: "Remote control", exact: true })).toBeVisible();
});

// A turn that ends while its subagents run must keep the Stop button reachable: the reducer test
// proves `working`, but only the browser proves the composer still paints it and answers a click.
test("the footer keeps Stop while background tasks outlive the turn", async ({ page }) => {
  await page.goto("/");
  await page.locator('.railworkspace[data-workspace="sessao-0929"] > .navitem').click();
  await expect(page.locator("#wsView")).toBeVisible();
  await expect(page.locator("#chatwrap .feed .turn").first()).toBeVisible();
  await page.evaluate(() => {
    const w = window as ComposerWindow;
    const { invoke } = w.__TAURI_INTERNALS__;
    w.controls = [];
    w.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "chat_control") w.controls.push(args);
      return invoke(command, args);
    };
    w.mock.line("t1", { v: 1, at: 1, type: "session.state", state: "busy" });
    w.mock.line("t1", { v: 1, at: 1, type: "background.changed",
      tasks: [{ id: "bg1", description: "Audit repository quality", toolId: null }] });
  });

  const composer = page.locator("#chatwrap .composer");
  const stop = composer.getByRole("button", { name: "Stop", exact: true });
  await expect(stop).toBeVisible();
  await page.evaluate(() => (window as ComposerWindow).mock.line("t1", {
    v: 1, at: 2, type: "turn.completed", outcome: "ok", message: "", durationMs: null, costUsd: null,
  }));
  await expect(composer.getByRole("status")).toHaveText("Working…");
  await expect(stop).toBeVisible();
  await stop.click();
  expect(await page.evaluate(() => (window as ComposerWindow).controls)).toEqual([
    { session: "t1", frame: { v: 1, type: "turn.interrupt" } },
  ]);

  await page.evaluate(() => (window as ComposerWindow).mock.line("t1",
    { v: 1, at: 3, type: "background.changed", tasks: [] }));
  await expect(stop).toBeHidden();
  await expect(composer).not.toHaveClass(/\bbusy\b/);
});
