import { test, expect } from "@playwright/test";

// The new catalog renders the production components without application bootstrap.
// Two readers in one browser expose shared observer/cache bugs that parser unit tests cannot detect.
test("@webkit isolated component stories keep diff instances and keyboard focus independent", async ({ page }) => {
  await page.goto("/design-system.html?component=diff&state=two-instances");
  expect(await page.evaluate(() => "__TAURI_INTERNALS__" in window)).toBe(false);
  const readers = page.locator(".story-diff");
  await expect(readers).toHaveCount(2);
  await expect(readers.nth(0).locator(".drow").first()).toBeVisible();
  await expect(readers.nth(1).locator(".drow").first()).toBeVisible();
  const first = readers.nth(0).getByRole("button", { name: "Toggle diff for src/example.ts" });
  await first.press("Enter");
  await expect(first).toHaveAttribute("aria-expanded", "false");
  await expect(readers.nth(1).locator(".dbody")).toBeVisible();
  await readers.nth(1).getByRole("checkbox").check();
  await expect(readers.nth(0).getByRole("checkbox")).not.toBeChecked();
  await expect(readers.nth(1).getByRole("checkbox")).toBeChecked();

  await page.getByRole("searchbox", { name: "Search components…" }).fill("requestCard");
  await page.locator('.catalog-list [data-component="chat-request"]').click();
  await expect(page).toHaveURL(/component=chat-request/);
  await page.getByRole("button", { name: "Changed files", exact: false }).click();
  await page.locator(".ask .row button").click();
  await expect(page.locator(".story-feedback")).toContainText('"outcome":"answer"');
  await page.getByRole("button", { name: "Component state" }).click();
  await page.getByRole("menuitemcheckbox", { name: "plan", exact: true }).click();
  await expect(page).toHaveURL(/state=plan/);
  await page.locator(".ask .row button").nth(2).click();
  await expect(page.locator(".ask textarea")).toBeFocused();

  await page.goto("/design-system.html?component=composer&state=attachments&embed=1");
  await page.setViewportSize({ width: 460, height: 800 });
  await page.getByRole("button", { name: "Remove attachment", exact: true }).click();
  await expect(page.locator(".injchip")).toHaveCount(0);
  const area = page.getByRole("textbox"); await area.fill("Example request");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.locator(".story-feedback")).toHaveText("Example request");
  expect(await page.locator("#story-canvas").evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
});
