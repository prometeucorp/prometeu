import { expect, test } from "@playwright/test";

// The workspace journey needs a browser: row navigation must not swallow its nested action,
// and provider tabs/native dialogs must preserve focus while asynchronous content is replaced.
test("GitHub issues and requested PR reviews launch workspaces without opening the source link", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("prometeu:model-choice", JSON.stringify({ agent: "codex", model: "gpt-5.6-sol" }));
    localStorage.setItem("mock:accounts", JSON.stringify({ accounts: [
      { id: "codex", provider: "codex", email: "user@example.com", plan: "pro", connected: true, revision: 0 },
    ], active: { codex: "codex" }, login: null }));
  });
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  const visited: string[] = [];
  page.on("console", message => { if (message.text().startsWith("open in GitHub:")) visited.push(message.text()); });
  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  await page.locator("#issues-provider-linear").focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.locator("#issues-provider-github")).toBeFocused();
  const row = page.locator("#github-list .irow", { hasText: "Restore terminal state" });
  await expect(row).toBeVisible();
  await row.focus(); await page.keyboard.press("Enter");
  await expect.poll(() => visited.length).toBe(1);
  await row.getByRole("button", { name: "Open workspace", exact: true }).click();
  await expect(page.locator(".launcher")).toBeVisible();
  await expect(page.locator("#d-project")).toContainText("prometeu");
  await expect(page.locator("#d-issue")).toContainText("prometeucorp/prometeu#428");
  await page.locator("#d-go").click();
  await expect(page.locator("#veil")).toBeHidden();
  expect(visited).toHaveLength(1);

  await page.locator("#railbody .navitem", { hasText: "Issues" }).click();
  await page.getByRole("button", { name: "Choose repositories", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Choose repositories" });
  await dialog.getByRole("checkbox", { name: "prometeucorp/prometeu", exact: true }).check();
  await page.evaluate(() => localStorage.setItem("mock:githubLogin", "another-user"));
  await dialog.getByRole("button", { name: "Save selection", exact: true }).click();
  await expect(dialog).toContainText("Your GitHub account changed.");
  expect(await page.evaluate(() => localStorage.getItem("mock:githubRepositories:another-user"))).toBeNull();
  await page.evaluate(() => localStorage.removeItem("mock:githubLogin"));
  await dialog.getByRole("button", { name: "Save selection", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Choose repositories", exact: true })).toBeFocused();
  await page.locator("#github-repositories").click();
  await expect(page.locator("#github-list .irow")).toHaveCount(2);

  await page.locator("#github-repositories").focus();
  await page.keyboard.press("End");
  await expect(page.locator("#github-reviews")).toBeFocused();
  await page.locator("#github-list .irow").getByRole("button", { name: "Open workspace", exact: true }).click();
  await expect(page.locator("#d-basename")).toContainText("prometeu-pr-438/github-pr-438-preview");
  await expect(page.locator("#d-wt")).toBeChecked();
  await expect(page.locator("#d-nb")).not.toBeChecked();
  await expect(page.locator("#d-nb")).toBeDisabled();
  await expect(page.locator("#d-project")).toBeDisabled();
  await expect(page.locator("#d-base")).toBeDisabled();
  await page.locator("#d-go").click();
  await expect(page.locator("#veil")).toBeHidden();
  expect(visited).toHaveLength(1);
});

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
