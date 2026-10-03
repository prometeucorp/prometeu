import { expect, test, type Locator, type Page } from "@playwright/test";

async function openAccounts(page: Page, provider = "codex") {
  await page.locator(`#status [data-provider="${provider}"]`).click();
  return page.getByRole("dialog", { name: "Usage limits" });
}

async function action(page: Page, card: Locator, label: string) {
  await card.getByRole("button", { name: "Account actions" }).click();
  await page.getByRole("menuitem", { name: label }).click();
}

test("footer accounts switch globally while preserving the conversation and the other provider’s selection", async ({ page }) => {
  await page.goto("/");
  await page.locator("#railbody .navitem.sub .lbl").getByText("Hello", { exact: true }).click();
  const before = await page.locator("#chatwrap").innerText();
  const panel = await openAccounts(page);
  const work = panel.locator(".uaccount", { hasText: "work@example.com" });
  const quota = await work.locator(".urow").last().boundingBox();
  await page.mouse.click(quota!.x + quota!.width / 2, quota!.y + quota!.height / 2);
  await expect(work.locator(".account-select")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('#status [data-provider="codex"]')).toHaveText("5h 10% · 7d 26%");
  await page.evaluate(async () => {
    const windowWithMock = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string) => Promise<Record<string, { windows: { pct: number }[] }>> };
      mock: { usage: (value: unknown) => void };
    };
    const usage = await windowWithMock.__TAURI_INTERNALS__.invoke("usage");
    usage.codex.windows.forEach((window) => { window.pct = 99; });
    windowWithMock.mock.usage(usage);
  });
  await expect(page.locator('#status [data-provider="codex"] .utext')).toContainText("7d 26%");
  await expect(page.locator('#status [data-provider="claude"]')).toHaveText("5h 16% · 7d 78% · Fable 72%");
  await page.keyboard.press("Escape");
  expect(await page.locator("#chatwrap").innerText()).toBe(before);
  await page.evaluate(() => {
    const mock = (window as unknown as { mock: { accountError: (error: string) => void } }).mock;
    mock.accountError('i18n:{"code":"err.account.disconnected"}');
  });
  await expect(page.locator("#msg")).toHaveText("Reconnect this account before activating it.");
  await page.reload();
  await expect(page.locator('#status [data-provider="codex"]')).toHaveText("5h 10% · 7d 26%");
  await openAccounts(page);
  await expect(work.locator(".account-select")).toHaveAttribute("aria-pressed", "true");
  await openAccounts(page, "claude");
  await expect(panel.locator('[data-account="claude"] .account-select')).toHaveAttribute("aria-pressed", "true");
  const other = panel.locator(".uaccount", { hasText: "work@example.com" }).locator(".account-select");
  await other.focus();
  await page.keyboard.press("Space");
  await expect(other).toHaveAttribute("aria-pressed", "true");
});

test("footer sign-in starts without an alias and supports cancellation and reconnection", async ({ page }) => {
  await page.goto("/");
  const panel = await openAccounts(page, "claude");
  await expect(panel.getByRole("textbox")).toHaveCount(0);
  await panel.getByRole("button", { name: "Add account" }).click();
  await expect(panel.getByRole("status")).toContainText("Complete sign-in in your browser.");
  await panel.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(panel.getByRole("status")).toHaveCount(0);
  const added = panel.locator(".uaccount").last();
  await expect(added.locator(".account-select")).toBeDisabled();
  await action(page, added, "Reconnect");
  await expect(added.locator(".account-select")).toBeEnabled();
  await expect(added.locator("strong")).toHaveText("new@example.com");
  // Connecting an account does not change selection without an explicit choice.
  await expect(panel.locator('[data-account="claude"] .account-select')).toHaveAttribute("aria-pressed", "true");
  await added.locator(".account-select").click();
  await expect(added.locator(".account-select")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('#status [data-provider="claude"]')).toHaveText("5h 11% · 7d 27%");
});

test("removing all accounts clears the selection and preserves the conversation and other provider", async ({ page }) => {
  await page.goto("/");
  await page.locator("#railbody .navitem.sub .lbl").getByText("Hello", { exact: true }).click();
  const before = await page.locator("#chatwrap").innerText();
  const panel = await openAccounts(page, "claude");
  await panel.locator(".uaccount", { hasText: "work@example.com" }).locator(".account-select").click();
  await openAccounts(page);
  const work = panel.locator(".uaccount", { hasText: "work@example.com" });
  await work.locator(".account-select").click();
  await expect(work.locator(".account-select")).toHaveAttribute("aria-pressed", "true");
  await action(page, work, "Remove account");
  const confirm = page.getByRole("dialog", { name: "Remove active account?" });
  await expect(confirm).toContainText("New messages");
  await confirm.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(work.locator(".account-select")).toHaveAttribute("aria-pressed", "true");
  await action(page, work, "Remove account");
  await confirm.getByRole("button", { name: "Remove account", exact: true }).click();
  await expect(work).toHaveCount(0);
  await expect(panel.locator('[data-account="codex"] .account-select')).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator('#status [data-provider="codex"]')).toHaveText("—");
  await action(page, panel.locator(".uaccount"), "Remove account");
  await expect(panel.locator(".uaccount")).toHaveCount(0);
  await expect(panel.getByText("No accounts added.")).toBeVisible();
  await page.keyboard.press("Escape");
  expect(await page.locator("#chatwrap").innerText()).toBe(before);
  await page.reload();
  await openAccounts(page);
  await expect(panel.locator(".uaccount")).toHaveCount(0);
  await expect(page.locator('#status [data-provider="codex"]')).toHaveText("—");
  await panel.getByRole("button", { name: "Add account" }).click();
  await expect(panel.getByRole("button", { name: "Account actions" })).toBeEnabled();
  await expect(panel.locator(".account-select")).toHaveAttribute("aria-pressed", "false");
  await panel.locator(".account-select").click();
  await expect(panel.locator(".account-select")).toHaveAttribute("aria-pressed", "true");
  await openAccounts(page, "claude");
  await expect(panel.locator(".uaccount", { hasText: "work@example.com" }).locator(".account-select")).toHaveAttribute("aria-pressed", "true");
});

test("legacy accounts ignore aliases and render email as text", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator('#status [data-provider="codex"]')).toBeVisible();
  const email = 'personal+"<img src=x onerror=alert(1)>"@example.com';
  await page.evaluate(async (email) => {
    const backend = (window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string) => Promise<any> } }).__TAURI_INTERNALS__;
    const saved = await backend.invoke("accounts");
    const account = saved.accounts.find((account: { id: string }) => account.id === "codex");
    account.label = "Old alias";
    account.email = email;
    localStorage.setItem("mock:accounts", JSON.stringify(saved));
  }, email);
  await page.reload();
  const panel = await openAccounts(page);
  await expect(panel.locator('[data-account="codex"] strong')).toHaveText(email);
  await expect(panel.locator('[data-account="codex"] strong')).toHaveAttribute("title", email);
  await expect(panel.getByText("Old alias")).toHaveCount(0);
  await expect(panel.getByRole("textbox")).toHaveCount(0);
  await expect(panel.locator("img")).toHaveCount(0);
});

test("accounts: Antigravity uses the external account without creating a login or activating it automatically", async ({ page }) => {
  await page.goto("/");
  const panel = await openAccounts(page, "antigravity");
  await panel.getByRole("button", { name: "Use agy account" }).click();
  const added = panel.locator('[data-account="antigravity"]');
  await expect(added).toContainText("Antigravity account");
  await expect(added.locator(".account-select")).toHaveAttribute("aria-pressed", "false");
  await expect(added.locator(".account-select")).toBeFocused();
  await expect(page.getByRole("dialog")).toHaveCount(1);
  await expect(panel.getByRole("button", { name: "Use agy account" })).toHaveCount(0);
  await expect(panel).toContainText("agy account added");
  await added.locator(".account-select").click();
  await expect(added.locator(".account-select")).toHaveAttribute("aria-pressed", "true");
  await expect(panel).toContainText("Gemini Models");
  await expect(panel).toContainText("Claude and GPT models");
  await expect(panel).toContainText("59% free");
  await expect(panel).toContainText("97% free");
  await expect(page.locator('#status [data-provider="antigravity"]')).not.toHaveText("—");
});


test("accounts: removing the last account without a CLI preserves focus on the group", { tag: "@webkit" }, async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("mock:antigravityMissing", "1");
    localStorage.setItem("mock:accounts", JSON.stringify({ accounts: [
      { id: "antigravity", provider: "antigravity", connected: true, revision: 0, authMethod: "external" },
    ], active: {}, login: null }));
  });
  await page.goto("/");
  // Initial navigation must finish before Settings can remain the active screen.
  await expect(page.locator("#deskView")).toBeVisible();
  await page.locator("#settings").click();
  await page.locator("#settingsView").getByRole("button", { name: "Agents", exact: true }).click();
  const group = page.locator('#settingsView [data-provider-accounts="antigravity"]');
  await action(page, group.locator(".uaccount"), "Remove account");
  await expect(group.locator(".uaccount")).toHaveCount(0);
  await expect(group).toContainText("Install Antigravity");
  await expect(group.getByRole("button", { name: "Use agy account" })).toBeDisabled();
  await expect(group).toBeFocused();
});

test("accounts: the launcher preserves its prompt until a connected account is selected", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("prometeu:model", "gemini-3.8-flash-high");
    localStorage.setItem("mock:accounts", JSON.stringify({ accounts: [
      { id: "antigravity", provider: "antigravity", email: "person@example.com", connected: true, revision: 0, authMethod: "external" },
    ], active: {}, login: null }));
  });
  await page.goto("/");
  await page.locator('#status [data-provider="antigravity"]').waitFor();
  await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
  await page.locator("#d-model").click();
  await page.locator(".ui-search-picker-choice", { hasText: "Gemini 3.8 Flash (High)" }).click();
  const prompt = page.locator("#d-prompt");
  await prompt.fill("My preserved request");
  const workspaceIds = () => page.evaluate(async () => {
    const backend = (window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string) => Promise<{ workspaces: { id: string }[] }> } }).__TAURI_INTERNALS__;
    return (await backend.invoke("load_board")).workspaces.map(workspace => workspace.id);
  });
  const before = await workspaceIds();
  await prompt.press("Enter");
  const picker = page.getByRole("dialog", { name: "Use this account", exact: true });
  await expect(picker).toBeVisible();
  expect(await workspaceIds()).toEqual(before);
  await expect(prompt).toHaveValue("My preserved request");
  await page.keyboard.press("Escape");
  await expect(picker).toHaveCount(0);
  await expect(prompt).toBeVisible();
  await expect(prompt).toHaveValue("My preserved request");
  await prompt.press("Enter");
  await expect(picker).toBeVisible();
  const account = picker.locator('[data-account="antigravity"]');
  await expect(account).toContainText("Use this account");
  await expect(account.locator(".account-select")).toHaveAttribute("aria-pressed", "false");
  await account.locator(".account-select").click();
  await picker.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(picker).toHaveCount(0);
  await expect(page.locator("#d-account")).toContainText("Antigravity account");
  await expect(prompt).toHaveValue("My preserved request");
  await prompt.press("Enter");
  await expect(page.locator("#veil")).toBeHidden();
  await expect(page.locator("#crumb")).toContainText("My preserved request");
});
