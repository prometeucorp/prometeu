import { expect, test } from "@playwright/test";

test("claiming a team issue keeps keyboard focus after the available row disappears", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    (window as unknown as { __linearOpenCalls: number }).__linearOpenCalls = 0;
    internals.invoke = function (command, args, options) {
      if (command === "linear_open") (window as unknown as { __linearOpenCalls: number }).__linearOpenCalls++;
      if (command !== "linear_claim") return original.call(this, command, args, options);
      return new Promise(resolve => setTimeout(resolve, 500)).then(() => original.call(this, command, args, options));
    };
    await internals.invoke("linear_connect");
  });

  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  const tabs = page.locator("#itabs .itab");
  await tabs.last().click();
  const row = page.locator("#ilist .irow.available", { hasText: "MOA-211" });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Claim" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#ilist")).toHaveAttribute("aria-busy", "true");
  await expect(row.getByRole("button", { name: "Claim" })).toHaveAttribute("aria-disabled", "true");
  await expect(row).toHaveCount(0);
  await expect(tabs.first().locator(".c")).toHaveText("8");
  await expect(tabs.last().locator(".c")).toHaveText("2");
  await expect(tabs.last()).toBeFocused();
  expect(await page.evaluate(() => (window as unknown as { __linearOpenCalls: number }).__linearOpenCalls)).toBe(0);

  await tabs.first().click();
  await expect(page.locator("#ilist .irow", { hasText: "MOA-211" })).toBeVisible();
});

test("a read-only Linear connection opens the assignment permission in settings", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("mock:linearReadOnly", "1"));
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.evaluate(async () => {
    type Invoke = (command: string) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    await internals.invoke("linear_connect");
  });

  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  await page.locator("#itabs .itab").last().click();
  await page.locator("#ilist .irow.available").first().getByRole("button", { name: "Authorize" }).click();
  await expect(page.locator("#settingsView")).toBeVisible();
  await expect(page.getByRole("button", { name: "Authorize assignments" })).toBeVisible();

  await page.evaluate(() => {
    localStorage.setItem("mock:linearConnectFailure", "1");
    localStorage.setItem("mock:linearIssuesFailure", "1");
  });
  await page.getByRole("button", { name: "Authorize assignments" }).click();
  await expect(page.getByRole("button", { name: "Authorize assignments" })).toBeEnabled();
  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  await expect(page.locator("#itabs .itab").first().locator(".c")).toHaveText("7");
  await expect(page.locator("#ilist .irow").first()).toBeVisible();
  await page.locator("#itabs .itab").last().click();
  await page.locator("#ilist .irow.available").first().getByRole("button", { name: "Authorize" }).click();

  await page.evaluate(() => {
    localStorage.removeItem("mock:linearConnectFailure");
    localStorage.removeItem("mock:linearIssuesFailure");
    localStorage.removeItem("mock:linearReadOnly");
  });
  await page.getByRole("button", { name: "Authorize assignments" }).click();
  await expect(page.getByRole("button", { name: "Authorize assignments" })).toHaveCount(0);
});

test("an unavailable team issue list keeps assigned issues visible", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("mock:linearAvailableFailure", "1"));
  await page.goto("/");
  await page.evaluate(async () => {
    type Invoke = (command: string) => Promise<unknown>;
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    await internals.invoke("linear_connect");
  });

  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  const tabs = page.locator("#itabs .itab");
  await expect(tabs.first().locator(".c")).toHaveText("7");
  await expect(page.locator("#ilist .irow").first()).toBeVisible();
  await tabs.last().click();
  await expect(page.locator("#ilist")).toContainText("Could not fetch");
  await page.evaluate(() => localStorage.removeItem("mock:linearAvailableFailure"));
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.locator("#ilist .irow.available").first()).toBeVisible();
});
