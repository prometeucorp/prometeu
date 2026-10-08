import { expect, test } from "@playwright/test";

test("desktop organizations select accepted access and keep sharing within the chosen scope", async ({ page }) => {
  await page.addInitScript(() => {
    if (!localStorage.getItem("mock:cloud")) localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "user1", name: "Alice", email: "alice@example.com" }, origin: "https://app.prometeu.co", offline: false }));
    if (!localStorage.getItem("mock:organizations")) localStorage.setItem("mock:organizations", JSON.stringify([
      { id: "organization1", slug: "one", name: "One", member: "membership1", role: "owner" },
      { id: "organization2", slug: "two", name: "Two", member: "membership2", role: "member" },
    ]));
  });
  await page.goto("/");
  await expect(page.locator(".cloud-account")).toContainText("Alice");
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Work and team" }).click();
  await expect(page.getByRole("button", { name: "Create team", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Join with code", exact: true })).toHaveCount(0);
  const choice = () => page.getByLabel("Share workspaces with", { exact: true });
  await choice().click();
  await page.getByRole("menuitemcheckbox", { name: "One", exact: true }).click();
  await expect(page.locator(".setrow", { hasText: "One" }).first()).toContainText("Connected");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:team")!).cloud)).toMatchObject({ user: "user1", slug: "one" });
  expect(await page.evaluate(() => localStorage.getItem("mock:shared"))).toBeNull();
  await page.reload();
  await page.locator("#settings").click();
  await expect(choice()).toContainText("One");
  await choice().click();
  await page.getByRole("menuitemcheckbox", { name: "Two", exact: true }).click();
  await expect(page.locator(".setrow", { hasText: "Two" }).first()).toContainText("Connected");
  expect(await page.evaluate(() => localStorage.getItem("mock:shared"))).toBeNull();
  await page.evaluate(() => localStorage.setItem("mock:organizations", "[]"));
  await page.locator(".cloud-account").click();
  await page.getByRole("menuitem", { name: "Refresh account" }).click();
  await expect(choice()).toHaveCount(0);
  await expect(page.locator("#settingsView")).toContainText("Create organizations");
});

test("footer remote control persists without sharing with the organization", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "user1", name: "Alice", email: "alice@example.com" }, origin: "https://app.prometeu.co", offline: false }));
    localStorage.setItem("mock:organizations", JSON.stringify([
      { id: "organization1", slug: "one", name: "One", member: "membership1", role: "owner" },
    ]));
  });
  await page.goto("/");
  await expect(page.locator("#tiles .tile").first()).toBeVisible();
  // Cloud discovery can rebuild the sidebar during a click; use the stable desk entry point.
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  const control = page.locator("#chatwrap").getByRole("button", { name: "Remote control", exact: true });
  await expect(control).toBeVisible();
  await expect(control).toHaveAttribute("aria-pressed", "false");

  await control.click();
  await expect(control).toHaveAttribute("aria-pressed", "true");
  // Remote control alone grants teammates nothing: the saved rights stay empty (ADR 0090).
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:shared")!)[0])).toEqual([
    "sessao-0929", [], "organization:organization1:membership1", true, { send: [], control: [] },
  ]);

  await page.reload();
  await expect(page.locator("#tiles .tile").first()).toBeVisible();
  await page.locator('#tiles .tile[data-tab="t1"] .topen').click();
  await expect(control).toHaveAttribute("aria-pressed", "true");
  await control.click();
  await expect(control).toHaveAttribute("aria-pressed", "false");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:shared")!))).toEqual([]);
});
