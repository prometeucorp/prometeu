import { expect, test } from "./fixtures";

// Live query updates replace session rows while a read-only dialog remains open. Native dialog
// focus restoration and WebKit overflow keyboard behavior need real browser verification.
test("@webkit usage history preserves session details and focus across live updates", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("prometeu:idioma", "en"));
  await page.goto("/");
  const complete = async (id: string, input: number) => {
    await page.evaluate(({ id, input }) => {
      const app = window as unknown as { mock: { line(tab: string, event: unknown): void } };
      app.mock.line("t1", { v: 1, type: "assistant.block", at: Date.now(), messageId: id, index: 0, block: { kind: "text", text: "Review complete." } });
      app.mock.line("t1", { v: 1, type: "turn.completed", at: Date.now(), messageId: id, outcome: "ok", message: "", durationMs: 100,
        costUsd: null, usage: { usageScope: "mainAgent", complete: true, selectedModel: null, observedModels: null, usageByModel: null,
          usage: { inputTokens: input, outputTokens: 20, cacheReadTokens: 80, cacheWriteTokens: null, reasoningTokens: null,
            contextUsed: null, contextWindow: null, peakContext: null, modelCalls: null, compactions: null, cacheRebuilds: null, costUsd: null } } });
    }, { id, input });
  };
  // Avoid the unrelated default demonstration history; this scenario only exercises live capture.
  await page.evaluate(() => localStorage.setItem("mock:telemetry", "[]"));
  await complete("history-first", 100);
  await page.locator("#settings").click();
  await page.locator(".setnavitem").getByText("Analytics", { exact: true }).click();
  const screen = page.locator(".telemetry-settings");
  const session = screen.locator(".history-session").first();
  await expect(session).toBeVisible();
  await session.focus(); await session.press("Enter");
  const detail = page.locator(".history-dialog");
  await expect(detail).toBeVisible();
  await expect(detail.locator("dd").first()).toHaveText("100");
  await page.keyboard.press("Tab");
  const close = detail.getByRole("button", { name: "Close", exact: true });
  await expect(close).toBeFocused();
  await complete("history-second", 200);
  await expect(detail.locator("dd").first()).toHaveText("300");
  await expect(close).toBeFocused();
  await close.press("Escape");
  await expect(detail).toBeHidden();
  await expect(session).toBeFocused();
  await page.setViewportSize({ width: 900, height: 800 });
  const region = screen.getByRole("region", { name: "Sessions", exact: true });
  await region.focus(); await region.press("ArrowRight");
  await expect.poll(() => region.evaluate(node => node.scrollLeft)).toBeGreaterThan(0);
  await screen.getByRole("button", { name: "Clear local history", exact: true }).click();
  await page.getByRole("dialog", { name: "Clear local history", exact: true }).getByRole("button", { name: "Clear local history", exact: true }).click();
  await expect(screen.locator(".history-session")).toHaveCount(0);
  await expect(screen.locator(".history-empty")).toBeVisible();
});

test("context actions preserve the draft and return keyboard focus", { tag: "@webkit" }, async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("prometeu:idioma", "en"));
  await page.goto("/");
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  await expect(page.locator("#chatwrap .feed .turn").first()).toBeVisible();
  await page.evaluate(() => {
    const w = window as unknown as { mock: { line: (tab: string, event: unknown) => void } };
    w.mock.line("t1", { v: 1, type: "context.updated", at: Date.now(), used: 140000, window: 200000 });
  });
  const composer = page.locator("#chatwrap .composer");
  const draft = composer.locator("textarea");
  await draft.fill("Keep this unsent draft");
  const gauge = composer.getByRole("button", { name: "Context: 140k / 200k", exact: true });
  await expect(gauge).toBeVisible();
  await gauge.focus(); await gauge.press("Enter");
  const panel = page.getByRole("dialog", { name: "Conversation context", exact: true });
  await expect(panel).toBeVisible();
  await panel.press("Tab");
  await expect(panel.getByRole("button", { name: "Compact", exact: true })).toBeFocused();

  // Background turn changes must refresh open controls without losing the keyboard position.
  await page.evaluate(() => {
    const w = window as unknown as { mock: { line: (tab: string, event: unknown) => void } };
    w.mock.line("t1", { v: 1, type: "session.state", at: Date.now(), state: "busy" });
  });
  await expect(panel.getByRole("button", { name: "Compact", exact: true })).toBeDisabled();
  await expect(panel.getByRole("button", { name: "Context report", exact: true })).toBeDisabled();
  await expect(panel).toBeFocused();
  await panel.press("Tab");
  await expect(panel.getByRole("button", { name: "New conversation", exact: true })).toBeFocused();
  await page.evaluate(() => {
    const w = window as unknown as { mock: { line: (tab: string, event: unknown) => void } };
    w.mock.line("t1", { v: 1, type: "session.state", at: Date.now(), state: "ready" });
  });
  await expect(panel.getByRole("button", { name: "Compact", exact: true })).toBeEnabled();
  await expect(panel.getByRole("button", { name: "Context report", exact: true })).toBeEnabled();
  await expect(panel.getByRole("button", { name: "New conversation", exact: true })).toBeFocused();
  await panel.press("Escape");
  await expect(gauge).toBeFocused();
  await gauge.press("Enter");
  await panel.getByRole("button", { name: "Context report", exact: true }).click();
  await expect(panel).toBeHidden();
  await expect(draft).toHaveValue("Keep this unsent draft");
  await expect(page.locator("#chatwrap .feed")).toContainText("/context");

  // A desk frame belongs to its own workspace even though the previous full view was elsewhere.
  await page.getByRole("button", { name: "Desk", exact: true }).click();
  const tile = page.locator('#tiles .tile[data-tab="t3"]');
  const expected = await page.evaluate(async () => {
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string) => Promise<import("../src/types").Board> } };
    const board = await w.__TAURI_INTERNALS__.invoke("load_board");
    const workspace = board.workspaces.find(item => item.tabs.some(tab => tab.id === "t3"))!;
    return { id: workspace.id, title: workspace.title, count: workspace.tabs.length };
  });
  await tile.locator(".context-gauge").click();
  await page.getByRole("dialog", { name: "Conversation context", exact: true }).getByRole("button", { name: "New conversation", exact: true }).click();
  await expect(page.locator("#crumb .nm")).toHaveText(expected.title);
  await expect(page.locator("#tabbar .tab[data-tab]")).toHaveCount(expected.count + 1);
});
