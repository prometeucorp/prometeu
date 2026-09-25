import { expect, test, type Locator, type Page } from "@playwright/test";

async function action(page: Page, row: Locator, name: string) {
  await row.getByRole("button", { name: /^More options · / }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}

test("the optional sidebar account connects, persists and signs out without changing conversations", async ({ page }) => {
  await page.goto("/");
  const account = page.locator(".cloud-account");
  await page.locator("#railbody .navitem.sub .lbl").getByText("Hello", { exact: true }).click();
  const before = await page.locator("#chatwrap").innerText();
  await account.locator(".cloud-label > small").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(account.getByRole("status")).toHaveText("ABCD-EFGH");
  await expect(account).toHaveAttribute("title", /Only authorize if your browser shows this same code/);
  await page.evaluate(() => localStorage.setItem("mock:cloudApproved", "1"));
  await expect(account).toContainText("Gustavo Brancaglione");
  expect(await page.locator("#chatwrap").innerText()).toBe(before);
  await page.reload();
  await expect(account).toHaveAttribute("title", "gustavo@example.com");
  await account.focus(); await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Manage account" })).toBeVisible();
  await page.getByRole("menuitem", { name: "Sign out" }).click();
  await expect(account).toContainText("Create account");
  await page.reload(); await expect(account).toContainText("Create account");
});

test("the optional sidebar account preserves its identity offline and recognizes revocation", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "1", name: "<img src=x>", email: "me@example.com" }, origin: "https://app.prometeu.co", offline: false }));
    localStorage.setItem("mock:cloudOffline", "1");
  });
  await page.goto("/");
  const account = page.locator(".cloud-account");
  await expect(account).toContainText("<img src=x>");
  await expect(account.locator("img")).toHaveCount(0);
  await expect(account).toContainText("Account offline");
  await page.evaluate(() => { localStorage.removeItem("mock:cloudOffline"); localStorage.setItem("mock:cloudExpired", "1"); });
  await account.click(); await page.getByRole("menuitem", { name: "Refresh account" }).click();
  await expect(account).toContainText("Create account");
});

test("the personal catalog keeps skills private, publishes explicitly and creates independent copies", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "1", name: "Person", email: "me@example.com" }, origin: "https://app.prometeu.co", offline: false })));
  await page.goto("/");
  await expect(page.locator("#tiles .tile").first()).toBeVisible();
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Resources" }).click();
  await page.locator('.resource-filters [data-filter="skills"]').click();
  const pending = page.locator(".resource-row", { has: page.locator("b", { hasText: /^cloud-review$/ }) });
  await expect(pending).toContainText("not installed on this Mac");
  await action(page, pending, "Install here");
  await expect(pending).toContainText("in the cloud");
  await page.getByRole("button", { name: "Add resource", exact: true }).click();
  await page.getByRole("menuitem", { name: "Create skill", exact: true }).click();
  let dialog = page.getByRole("dialog", { name: "Create skill" });
  await dialog.getByLabel("Skill name").fill("my-review");
  await dialog.getByLabel("When to use this skill").fill("Before shipping code");
  await dialog.getByLabel("Instructions", { exact: true }).fill("Read the changes and run tests.");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  const local = page.locator(".resource-row", { has: page.locator("b", { hasText: /^my-review$/ }) });
  await expect(local).toContainText("this Mac only");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:catalog") ?? "{}").shared?.["skills:my-review"])).toBeUndefined();
  await action(page, local, "Share in cloud");
  dialog = page.getByRole("dialog", { name: "Share in cloud" });
  await dialog.getByRole("button", { name: "Share in cloud" }).click();
  await expect(local).toContainText("in the cloud");
  await action(page, local, "Create local copy");
  dialog = page.getByRole("dialog", { name: "Create local copy" });
  await dialog.getByLabel("Copy name").fill("my-copy");
  await dialog.getByRole("button", { name: "Create local copy" }).click();
  const copy = page.locator(".resource-row", { has: page.locator("b", { hasText: /^my-copy$/ }) });
  await expect(copy).toContainText("this Mac only");
  await page.evaluate(() => localStorage.setItem("mock:cloudOffline", "1"));
  await action(page, copy, "Edit");
  dialog = page.getByRole("dialog", { name: "Edit skill" });
  await dialog.getByLabel("Instructions", { exact: true }).fill("Local only, even offline.");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:catalog")!).skills.find((s: {id:string}) => s.id === "my-review").content)).toBe("Read the changes and run tests.");
  await action(page, local, "Edit");
  dialog = page.getByRole("dialog", { name: "Edit skill" });
  await dialog.getByLabel("Instructions", { exact: true }).fill("Offline attempt");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog.getByRole("alert")).not.toBeEmpty();
  await expect(dialog.getByLabel("Instructions", { exact: true })).toHaveValue("Offline attempt");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.evaluate(() => localStorage.removeItem("mock:cloudOffline"));
  await action(page, pending, "Remove from this Mac");
  await expect(pending).toContainText("not installed on this Mac");
  await pending.getByRole("button", { name: "More options · cloud-review", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Install here", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
});

test("the personal catalog rejects stale edits over a newer revision", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "1", name: "Person", email: "me@example.com" }, origin: "https://app.prometeu.co", offline: false })));
  await page.goto("/");
  await expect(page.locator("#tiles .tile").first()).toBeVisible();
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Resources" }).click();
  await page.locator('.resource-filters [data-filter="skills"]').click();
  const row = page.locator(".resource-row", { has: page.locator("b", { hasText: /^cloud-review$/ }) });
  await action(page, row, "Install here");
  await action(page, row, "Edit");
  const dialog = page.getByRole("dialog", { name: "Edit skill" });
  await page.evaluate(() => localStorage.setItem("mock:catalog", JSON.stringify({ connected: true, revision: 7, plugins: [], mcp: [],
    skills: [{ id: "cloud-review", local_id: "cloud-review", installed: true, description: "Review", content: "Updated in SaaS" }], shared: { "skills:cloud-review": "cloud-review" } })));
  await dialog.getByLabel("Instructions", { exact: true }).fill("Old draft");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("The catalog changed");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:catalog")!).skills[0].content)).toBe("Updated in SaaS");
});
