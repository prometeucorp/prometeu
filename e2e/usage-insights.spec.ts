import { expect, test } from "@playwright/test";

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
