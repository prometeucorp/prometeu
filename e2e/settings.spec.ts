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
  await expect(page.locator("#deskView")).toBeVisible();
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
