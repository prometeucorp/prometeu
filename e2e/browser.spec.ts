import { expect, test, type Page } from "./fixtures";
import type { Board } from "../src/types";

const previewSelector = 'iframe[data-browser-workspace="sessao-0929"]';
const composerSelector = "#chatwrap .composer textarea";
const contextSelector = "#chatwrap .cfiles .browser-context";
const fileSelector = "#chatwrap .cfiles .injchip:not(.browser-context)";
type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type BrowserTestWindow = Window & {
  mock: { preview(workspace: string, conversation: string): void };
  __TAURI_INTERNALS__: { invoke: Invoke };
  browserWaiting: boolean;
  browserRelease: () => void;
  browserSent: number;
  browserMessages: { session: string; text: string }[];
};

test("browser opens the MCP-requested workspace and conversation without starting scripts", async ({ page }) => {
  const target = await page.evaluate(async () => {
    const window = globalThis as unknown as BrowserTestWindow;
    const board = await window.__TAURI_INTERNALS__.invoke("load_board") as Board;
    return board.workspaces.find((workspace) => workspace.id !== "sessao-0929"
      && !workspace.archived && !workspace.cleaned && !workspace.preparing && !workspace.failed && workspace.tabs.length)!;
  });
  expect(target).toBeTruthy();
  const conversation = target.tabs.at(-1)!.id;
  await page.evaluate(({ workspace, conversation }) => {
    const window = globalThis as unknown as BrowserTestWindow;
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "open_dock") throw new Error("Preview must not start scripts");
      return invoke(command, args);
    };
    window.mock.preview(workspace, conversation);
  }, { workspace: target.id, conversation });
  await expect(page.locator(`iframe[data-browser-workspace="${target.id}"]`)).toBeVisible();
  await expect(page.locator(`#tabbar .tab[data-tab="${conversation}"]`)).toHaveClass(/on/);
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(page.locator("#msg")).not.toContainText("Preview must not start scripts");

  // A stale or mismatched target must not navigate away from the requested workspace.
  await page.evaluate(() => {
    (window as BrowserTestWindow).mock.preview("sessao-0929", "missing-conversation");
  });
  await expect(page.locator(`iframe[data-browser-workspace="${target.id}"]`)).toBeVisible();
});

test.use({ viewport: { width: 1600, height: 1000 } });

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  await expect(page.locator(composerSelector)).toBeVisible();
  await page.evaluate(() => {
    const target = window as BrowserTestWindow;
    const original = target.__TAURI_INTERNALS__.invoke;
    target.browserSent = 0;
    target.browserMessages = [];
    target.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "chat_send") {
        target.browserSent++;
        target.browserMessages.push({ session: String(args?.session), text: String(args?.text) });
      }
      return original(command, args);
    };
  });
  await page.locator("#run-go").click();
  await expect(page.locator("#run-open")).toBeVisible();
});

async function openBrowser(page: Page) {
  await page.locator("#run-open").click();
  await expect(page.locator("#webview")).toBeVisible();
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(page.locator("#side")).toBeHidden();
  await expect(page.locator(previewSelector)).toBeVisible();
  await expect(page.frameLocator(previewSelector).locator("#design-button")).toBeVisible();
}

async function pickElement(page: Page) {
  await page.locator("#winspect").click();
  await expect(page.locator("#winspect")).toHaveAttribute("aria-pressed", "true");
  await page.frameLocator(previewSelector).locator("#design-button").click();
  await expect(page.locator("#webdetails")).toBeVisible();
}

// Complete the native effect first, then hold its response to exercise late delivery deterministically.
async function hold(page: Page, command: string) {
  await page.evaluate(command => {
    const target = window as BrowserTestWindow;
    const original = target.__TAURI_INTERNALS__.invoke;
    let release!: () => void;
    const ready = new Promise<void>(resolve => { release = resolve; });
    let first = true;
    target.browserWaiting = false;
    target.__TAURI_INTERNALS__.invoke = async (next, args) => {
      if (next !== command || !first) return original(next, args);
      first = false;
      const result = await original(next, args);
      target.browserWaiting = true;
      await ready;
      return result;
    };
    target.browserRelease = () => {
      target.__TAURI_INTERNALS__.invoke = original;
      release();
    };
  }, command);
}

async function waiting(page: Page) {
  await expect.poll(() => page.evaluate(() => (window as BrowserTestWindow).browserWaiting)).toBe(true);
}

async function release(page: Page) {
  await page.evaluate(() => (window as BrowserTestWindow).browserRelease());
}

test("browser shows elements as tags and preserves complete data on send and replay", { tag: "@webkit" }, async ({ page }) => {
  await openBrowser(page);
  const composer = page.locator(composerSelector);
  await composer.fill("/compact");
  await pickElement(page);
  await expect(page.locator("#webdetails .webhtml")).toContainText('id="design-button"');
  await expect(page.locator("#webdetails .webstyles")).toContainText("background-color:");
  await page.locator("#wadd").click();
  await expect(composer).toHaveValue("/compact");
  await expect(page.locator(contextSelector)).toHaveCount(1);
  await expect(page.locator(contextSelector)).toContainText("Selected element");
  await expect(page.locator(fileSelector)).toHaveCount(0);
  await expect(page.locator("#webdetails")).toBeHidden();
  await expect(page.locator(previewSelector)).toBeVisible();
  expect(await page.evaluate(() => (window as BrowserTestWindow).browserSent)).toBe(0);

  await page.locator(`${contextSelector} .browser-context-open`).click();
  const dialog = page.getByRole("dialog", { name: "Selected element", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('id="design-button"');
  await expect(dialog).toContainText("background-color:");
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(composer).toHaveValue("/compact");
  await page.locator("#chatwrap .send").click();
  await expect(composer).toHaveValue("");
  await expect(page.locator(contextSelector)).toHaveCount(0);
  const message = await page.evaluate(() => (window as BrowserTestWindow).browserMessages[0]);
  expect(message.session).toBe("t1");
  expect(message.text.startsWith('<prometeu-browser-element v="1">')).toBe(true);
  expect(message.text.endsWith("/compact")).toBe(true);
  const payload = JSON.parse(message.text.match(/<prometeu-browser-element v="1">\n([^\n]+)\n/)![1]);
  expect(payload.selection).toMatchObject({
    selector: "#design-button", tag: "button", html: expect.stringContaining('id="design-button"'),
    url: expect.stringMatching(/^http:\/\/localhost:\d+\//),
    styles: { "background-color": expect.any(String) },
    rect: { x: expect.any(Number), y: expect.any(Number), width: expect.any(Number), height: expect.any(Number) },
    viewport: { width: expect.any(Number), height: expect.any(Number) },
  });
  expect(payload.image).toMatch(/browser-.*\.png$/);
  expect(message.text).toContain(`@"${payload.image}"`);
  const bubble = page.locator("#chatwrap .turn.user").last();
  await expect(bubble).toContainText("/compact");
  await expect(bubble.locator(".browser-context")).toContainText("Selected element");
  await expect(bubble).not.toContainText('"html"');
  await expect(bubble).not.toContainText("prometeu-browser-element");
  await expect(bubble).not.toContainText(payload.image);
  const snapshot = await page.evaluate(async () => {
    const target = window as BrowserTestWindow;
    return await target.__TAURI_INTERNALS__.invoke("chat_snapshot", { session: "t1" }) as { text: string };
  });
  const recorded = snapshot.text.trim().split("\n").map(line => JSON.parse(line))
    .find(event => event.type === "user.message" && event.content?.[0]?.text === message.text);
  expect(recorded).toBeDefined();
  await page.locator('#tabbar .tab[data-tab="t2"]').click();
  await page.locator('#tabbar .tab[data-tab="t1"]').click();
  await expect(bubble).toContainText("/compact");
  await expect(bubble.locator(".browser-context")).toContainText("Selected element");
  await expect(bubble).not.toContainText('"html"');
  await expect(bubble).not.toContainText(payload.image);
});

test("browser keeps the preview when switching conversations and adds context only to the current target", async ({ page }) => {
  await openBrowser(page);
  const composer = page.locator(composerSelector);
  await composer.fill("Draft for the first conversation.");
  await pickElement(page);
  await page.locator('#tabbar .tab[data-tab="t2"]').click();
  await expect(page.locator(previewSelector)).toBeVisible();
  await expect(page.locator("#webdetails")).toBeVisible();
  await composer.fill("Draft for the second conversation.");
  await page.locator("#wadd").click();
  await expect(composer).toHaveValue("Draft for the second conversation.");
  await expect(page.locator(contextSelector)).toHaveCount(1);
  await expect(page.locator(fileSelector)).toHaveCount(0);
  await page.locator('#tabbar .tab[data-tab="t1"]').click();
  await expect(composer).toHaveValue("Draft for the first conversation.");
  await expect(page.locator("#chatwrap .cfiles .injchip")).toHaveCount(0);
  await expect(page.locator(previewSelector)).toBeVisible();
  await page.locator('#tabbar .tab[data-tab="t2"]').click();
  await expect(composer).toHaveValue("Draft for the second conversation.");
  await expect(page.locator(contextSelector)).toContainText("Selected element");
  await page.locator(contextSelector).getByRole("button", { name: "Remove selected element", exact: true }).click();
  await expect(page.locator("#chatwrap .cfiles .injchip")).toHaveCount(0);
  await expect(composer).toHaveValue("Draft for the second conversation.");
  expect(await page.evaluate(() => (window as BrowserTestWindow).browserSent)).toBe(0);
  await page.locator("#chatwrap .send").click();
  await expect.poll(() => page.evaluate(() => (window as BrowserTestWindow).browserMessages[0]))
    .toEqual({ session: "t2", text: "Draft for the second conversation." });
});

test("browser preserves tag and capture in the pending queue without exposing JSON after resume fails", async ({ page }) => {
  await openBrowser(page);
  const composer = page.locator(composerSelector);
  await composer.fill("Message for a stopped session.");
  await pickElement(page);
  await page.locator("#wadd").click();
  await page.evaluate(() => {
    const target = window as BrowserTestWindow;
    const original = target.__TAURI_INTERNALS__.invoke;
    target.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command !== "chat_send") return original(command, args);
      const message = { session: String(args?.session), text: String(args?.text) };
      target.browserSent++;
      target.browserMessages.push(message);
      // Rust persists the queue before trying to revive a stopped process.
      const board = await original("load_board") as Board;
      const workspace = board.workspaces.find(workspace => workspace.tabs.some(tab => tab.id === message.session))!;
      const tab = workspace.tabs.find(tab => tab.id === message.session)!;
      tab.pending_prompt = message.text;
      tab.status = "desligada";
      await original("focus_tab", { workspace: workspace.id, tab: tab.id });
      throw "Simulated failure after queuing.";
    };
  });
  await page.locator("#chatwrap .send").click();
  await expect.poll(() => page.evaluate(() => (window as BrowserTestWindow).browserMessages.length)).toBe(1);
  await expect(composer).toHaveValue("");
  await expect(page.locator(contextSelector)).toHaveCount(0);
  await expect(page.locator(fileSelector)).toHaveCount(0);
  const pending = page.locator("#chatwrap .turn.user.wait");
  await expect(pending).toContainText("Message for a stopped session.");
  await expect(pending.locator(".browser-context")).toContainText("Selected element");
  await expect(pending).not.toContainText('"html"');
  await expect(pending).not.toContainText("prometeu-browser-element");
  const queued = await page.evaluate(async () => {
    const target = window as BrowserTestWindow;
    const board = await target.__TAURI_INTERNALS__.invoke("load_board") as Board;
    return {
      sent: target.browserMessages[0].text,
      stored: board.workspaces.flatMap(workspace => workspace.tabs).find(tab => tab.id === "t1")!.pending_prompt,
    };
  });
  expect(queued.stored).toBe(queued.sent);
  expect(queued.stored).toContain('"selector":"#design-button"');
  expect(queued.stored).toMatch(/browser-.*\.png/);
  await page.locator('#tabbar .tab[data-tab="t2"]').click();
  await page.locator('#tabbar .tab[data-tab="t1"]').click();
  await expect(pending).toContainText("Message for a stopped session.");
  await expect(pending.locator(".browser-context")).toContainText("Selected element");
  await expect(pending).not.toContainText('"html"');
  expect(await page.evaluate(() => (window as BrowserTestWindow).browserSent)).toBe(1);
});

test("browser delivers a delayed capture to its original conversation after switching tabs", async ({ page }) => {
  await openBrowser(page);
  const composer = page.locator(composerSelector);
  await composer.fill("Capture for the first conversation.");
  await hold(page, "browser_capture");
  await page.locator("#wcapture").click();
  await waiting(page);
  await expect(page.locator("#chatwrap .send")).toBeDisabled();
  await page.locator('#tabbar .tab[data-tab="t2"]').click();
  await composer.fill("Another conversation.");
  await release(page);
  await expect(page.locator("#wcapture")).toBeEnabled();
  await expect(page.locator("#chatwrap .cfiles .injchip")).toHaveCount(0);
  await expect(composer).toHaveValue("Another conversation.");
  await page.locator('#tabbar .tab[data-tab="t1"]').click();
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText(/browser-.*\.png/);
  await expect(composer).toHaveValue("Capture for the first conversation.");
  await expect(page.locator("#chatwrap .send")).toBeEnabled();
  expect(await page.evaluate(() => (window as BrowserTestWindow).browserSent)).toBe(0);
});

test("browser discards a selection resized during capture and allows inspection again", async ({ page }) => {
  await openBrowser(page);
  await hold(page, "browser_capture");
  await page.locator("#winspect").click();
  await expect(page.locator("#winspect")).toHaveAttribute("aria-pressed", "true");
  await page.frameLocator(previewSelector).locator("#design-button").click();
  await waiting(page);
  await page.locator("#websplit").press("ArrowRight");
  await release(page);
  await expect(page.locator("#webdetails")).toBeHidden();
  await pickElement(page);
  await expect(page.locator("#webdetails .webhtml")).toContainText('id="design-button"');
});

test("browser preserves textual context when capture fails", async ({ page }) => {
  await openBrowser(page);
  await page.evaluate(() => {
    const target = window as BrowserTestWindow;
    const original = target.__TAURI_INTERNALS__.invoke;
    target.__TAURI_INTERNALS__.invoke = (command, args) => command === "browser_capture"
      ? Promise.reject('i18n:{"code":"err.browser.captureFailed"}') : original(command, args);
  });
  await page.locator(composerSelector).fill("My reference.");
  await pickElement(page);
  await expect(page.locator("#webdetails .webmeta")).toContainText("No screenshot");
  await page.locator("#wadd").click();
  await expect(page.locator(composerSelector)).toHaveValue("My reference.");
  await expect(page.locator(contextSelector)).toContainText("Selected element");
  await expect(page.locator(fileSelector)).toHaveCount(0);
  expect(await page.evaluate(() => (window as BrowserTestWindow).browserSent)).toBe(0);
});

test("browser allows file drops into chat and preserves conversation and draft when closing the preview", { tag: "@webkit" }, async ({ page }) => {
  await openBrowser(page);
  const composer = page.locator(composerSelector);
  await composer.fill("Use this reference.");
  await composer.evaluate(element => {
    const rect = element.getBoundingClientRect();
    const target = window as unknown as { mock: { drop(paths: string[], x: number, y: number): void } };
    target.mock.drop(["/Users/tester/Desktop/Reference.png"], rect.x + rect.width / 2, rect.y + rect.height / 2);
  });
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText("Reference.png");
  await expect(page.locator(previewSelector)).toBeVisible();
  await page.locator("#tabbar .tab", { hasText: "Browser" }).locator(".tabx").click();
  await expect(page.locator("#webview")).toBeHidden();
  await expect(page.locator(previewSelector)).toHaveCount(0);
  await expect(page.locator("#side")).toBeVisible();
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(composer).toHaveValue("Use this reference.");
  await expect(page.locator("#chatwrap .cfiles .injchip")).toContainText("Reference.png");
});

test("browser adjusts viewport and separator by keyboard and stacks narrow layouts without overlap", { tag: "@webkit" }, async ({ page }) => {
  await openBrowser(page);
  await page.locator("#wwidth").fill("390");
  await page.locator("#wwidth").press("Tab");
  await expect.poll(async () => (await page.locator(previewSelector).boundingBox())?.width).toBe(390);
  const before = (await page.locator("#chatwrap").boundingBox())!;
  await page.locator("#websplit").press("ArrowRight");
  await expect(page.locator("#websplit")).toHaveAttribute("aria-valuenow", "42");
  expect((await page.locator("#chatwrap").boundingBox())!.width).toBeGreaterThan(before.width);
  await page.locator("#websplit").press("Home");
  await expect(page.locator("#websplit")).toHaveAttribute("aria-valuenow", "40");
  await page.setViewportSize({ width: 900, height: 900 });
  await expect(page.locator("#websplit")).toBeHidden();
  await expect(page.locator(previewSelector)).toBeVisible();
  const chat = (await page.locator("#chatwrap").boundingBox())!;
  const preview = (await page.locator("#webview").boundingBox())!;
  expect(preview.y).toBeGreaterThanOrEqual(chat.y + chat.height);
  expect(await page.locator("#tabbody").evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
});

test("browser suspends for menus and dialogs and restores the page without reloading", { tag: "@webkit" }, async ({ page }) => {
  await openBrowser(page);
  const preview = page.locator(previewSelector);
  const name = page.frameLocator(previewSelector).locator("#design-name");
  await name.fill("Preserved project");
  await page.locator('#tabbar .tab[data-tab="t1"]').click({ button: "right" });
  await expect(page.locator(".menu")).toBeVisible();
  await expect(preview).toHaveJSProperty("hidden", true);
  await page.keyboard.press("Escape");
  await expect(page.locator(".menu")).toHaveCount(0);
  await expect(preview).toHaveJSProperty("hidden", false);
  await expect(name).toHaveValue("Preserved project");
  await page.evaluate(() => {
    const dialog = document.createElement("dialog");
    dialog.id = "browser-test-dialog";
    document.body.append(dialog);
    dialog.showModal();
  });
  await expect(preview).toHaveJSProperty("hidden", true);
  await page.keyboard.press("Escape");
  await expect(page.locator("#browser-test-dialog")).not.toHaveAttribute("open");
  await expect(preview).toHaveJSProperty("hidden", false);
  await expect(name).toHaveValue("Preserved project");
  await page.getByRole("button", { name: "Feedback", exact: true }).click();
  await expect(preview).toHaveJSProperty("hidden", true);
  await page.keyboard.press("Escape");
  await expect(preview).toHaveJSProperty("hidden", false);
  await expect(name).toHaveValue("Preserved project");
});

test("browser ignores a delayed opening after leaving the workspace and does not cover another conversation", async ({ page }) => {
  await hold(page, "browser_open");
  await page.locator("#run-open").click();
  await waiting(page);
  await page.locator("#railbody .navitem", { hasText: "Desk" }).click();
  await expect(page.locator("#deskView")).toBeVisible();
  await release(page);
  // Checking the native mock flag catches a leaked view even when its DOM ancestor is hidden.
  await expect(page.locator(previewSelector)).toHaveJSProperty("hidden", true);
  await expect(page.locator("#wsView")).toBeHidden();
  await expect(page.locator("#deskView")).toBeVisible();
  await page.locator('#tiles .tile[data-tab="t3"] .topen').click();
  await expect(page.locator("#side")).toBeVisible();
  await expect(page.locator('#tabbar .tab[data-tab="t3"]')).toHaveClass(/\bon\b/);
  await expect(page.locator("#chatwrap")).toBeVisible();
  await expect(page.locator("#webview")).toBeHidden();
  await expect(page.locator(previewSelector)).toHaveJSProperty("hidden", true);
});
