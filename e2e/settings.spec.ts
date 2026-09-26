import { expect, test, type Page } from "@playwright/test";
import type { McpServer } from "../src/types";

async function refreshCatalog(page: Page) {
  await page.evaluate(async () => {
    const app = window as unknown as {
      __TAURI_INTERNALS__: { invoke(command: string): Promise<McpServer[]> };
      mock: { catalog(servers: McpServer[]): void };
    };
    app.mock.catalog(await app.__TAURI_INTERNALS__.invoke("mcp_hub"));
  });
}

// Background catalog updates replace Settings DOM. Real browser focus and native details geometry
// must survive that replacement; pure matching/legacy routing are covered by unit tests instead.
test("@webkit settings retain search, menu focus and expanded sections across refresh", async ({ page }) => {
  await page.goto("/");
  await page.locator("#settings").click();
  await page.locator(".setnavitem").getByText("Resources", { exact: true }).click();
  await page.locator('[data-filter="mcp"]').click();
  const search = page.getByRole("searchbox", { name: "Search resources…" });
  await search.fill("not");
  await refreshCatalog(page);
  await expect(search).toBeFocused();
  await expect(search).toHaveValue("not");
  await search.press("End"); await search.pressSequentially("ion");
  await expect(page.locator(".resource-row:visible")).toHaveCount(1);
  await expect(page.locator('[data-filter="mcp"]')).toHaveAttribute("aria-pressed", "true");
  const more = page.getByRole("button", { name: "More options · notion", exact: true });
  await more.focus(); await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Edit", exact: true })).toBeVisible();
  await refreshCatalog(page);
  await expect(page.getByRole("menuitem")).toHaveCount(0);
  await expect(more).toBeFocused();

  // Container queries must remove the icon's grid slot, even with the full desktop CSS loaded.
  await page.setViewportSize({ width: 900, height: 800 });
  const row = page.locator(".resource-row:visible");
  await expect(row.locator(".resource-glyph")).toBeHidden();
  const copy = (await row.locator(".resource-copy").boundingBox())!;
  const actions = (await row.locator(".resource-actions").boundingBox())!;
  expect(copy.width).toBeGreaterThan(150);
  expect(actions.x).toBeGreaterThan(copy.x + copy.width);
  expect(actions.x + actions.width).toBeLessThanOrEqual(900);
  await page.setViewportSize({ width: 1280, height: 720 });

  const globalSearch = page.getByRole("searchbox", { name: "Search settings…" });
  await globalSearch.fill("acc");
  await globalSearch.pressSequentially("ount");
  await refreshCatalog(page);
  await expect(globalSearch).toBeFocused();
  await expect(globalSearch).toHaveValue("account");
  await globalSearch.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(page.locator('.setnavitem[aria-current="page"]')).toHaveText("Agents");
  const usage = page.locator('[data-settings-disclosure^="account-usage-"]').first();
  await usage.locator("summary").click();
  await expect(usage).toHaveAttribute("open", "");
  const body = page.locator(".setpage");
  await usage.locator("summary").focus();
  const scroll = await body.evaluate(element => element.scrollTop);
  await refreshCatalog(page);
  await expect(usage).toHaveAttribute("open", "");
  await expect(usage.locator("summary")).toBeFocused();
  expect(await body.evaluate(element => element.scrollTop)).toBe(scroll);
});

// The same presentation runs without the app. Browser focus and CSS cascade are the risks here;
// hub actions and compatibility are covered by the adapter unit test and existing product journeys.
test("@webkit resource gallery preserves keyboard focus and compact geometry without the app", async ({ page }) => {
  await page.goto("/design-system.html");
  expect(await page.evaluate(() => "__TAURI_INTERNALS__" in window)).toBe(false);
  const preview = page.locator("#resources-preview");
  const search = preview.getByRole("searchbox", { name: "Search resources…" });
  await search.fill("workspace");
  const more = preview.getByRole("button", { name: "More options · workspace-tools", exact: true });
  await more.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Edit", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(more).toBeFocused();
  await more.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(preview.locator(".resource-description").first()).toContainText("· 1");
  await expect(more).toBeFocused();
  await expect(search).toHaveValue("workspace");
  const choice = preview.getByRole("button", { name: "Example state" });
  await choice.click();
  await page.getByRole("menuitemcheckbox", { name: "Recoverable error", exact: true }).click();
  await expect(preview.getByRole("alert")).toContainText("Could not load resources");
  await preview.getByRole("button", { name: "Try again", exact: true }).click();
  await expect(search).toBeFocused();
  await expect(search).toHaveValue("workspace");
  await expect(more).toBeVisible();
  await search.fill("");
  await choice.click();
  await page.getByRole("menuitemcheckbox", { name: "Long text", exact: true }).click();
  await page.setViewportSize({ width: 460, height: 800 });
  const row = preview.locator(".resource-row").first();
  await expect(row.locator(".resource-glyph")).toBeHidden();
  expect((await row.locator(".resource-copy").boundingBox())!.width).toBeGreaterThan(150);
  const actions = (await row.locator(".resource-actions").boundingBox())!;
  expect(actions.x + actions.width).toBeLessThan(460);
  expect(await preview.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
});

// Reusable rows combine multiple independent controls. Exercise their actual browser focus
// and narrow layout outside a feature, then verify the second production consumer uses them.
test("@webkit desktop components compose independent controls and restore menu focus", async ({ page }) => {
  await page.goto("/design-system.html#components-preview");
  expect(await page.evaluate(() => "__TAURI_INTERNALS__" in window)).toBe(false);
  const preview = page.locator("#components-preview");
  const search = preview.getByRole("searchbox", { name: "Search examples…" });
  await search.fill("assistant");
  await expect(preview.locator(".desktop-item")).toHaveCount(1);
  const more = preview.getByRole("button", { name: "More options · Review assistant", exact: true });
  await more.press("ArrowDown"); await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Remove", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(more).toBeFocused();
  await preview.getByRole("button", { name: "Edit", exact: true }).click();
  await expect(preview.getByRole("status").last()).toContainText("Review assistant");
  const choice = preview.getByRole("button", { name: "Example state" });
  await choice.click(); await page.getByRole("menuitemcheckbox", { name: "Operation in progress", exact: true }).click();
  await expect(more).toBeDisabled();
  await expect(preview.getByRole("button", { name: "Edit", exact: true })).toBeDisabled();
  await choice.click(); await page.getByRole("menuitemcheckbox", { name: "Recoverable error", exact: true }).click();
  await expect(preview.getByRole("alert")).toBeVisible();
  await preview.getByRole("button", { name: "Try again", exact: true }).click();
  await expect(search).toBeFocused();
  await page.setViewportSize({ width: 460, height: 800 });
  expect(await preview.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
  const row = preview.locator(".desktop-item");
  const copy = (await row.locator(".desktop-item-copy").boundingBox())!;
  const controls = (await row.locator(".desktop-item-actions").boundingBox())!;
  expect(copy.width).toBeGreaterThan(100);
  expect(controls.x).toBeGreaterThan(copy.x + copy.width);
  expect(controls.x + controls.width).toBeLessThan(460);

  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto("/");
  await page.locator("#settings").click();
  await page.locator(".setnavitem").getByText("Actions", { exact: true }).click();
  const profile = page.locator(".action-profile.desktop-item").first();
  const trigger = profile.locator(".desktop-overflow");
  await trigger.press("ArrowDown"); await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Remove", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
  expect(await profile.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
});
