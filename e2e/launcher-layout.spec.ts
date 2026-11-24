import { expect, test, type Locator, type Page } from "./fixtures";

const email = "person.with.a.very.long.surname@department.company.example.com";

async function seed(page: Page) {
  await page.addInitScript(email => {
    localStorage.setItem("prometeu:model-choice", JSON.stringify({ agent: "codex", model: "gpt-5.6-sol" }));
    localStorage.setItem("prometeu:effort", "high");
    localStorage.setItem("mock:accounts", JSON.stringify({ accounts: [
      { id: "codex", provider: "codex", email, plan: "pro", connected: true, revision: 0 },
    ], active: { codex: "codex" }, login: null }));
  }, email);
  await page.goto("/");
  await expect(page.locator("#railbody .navitem.sub").first()).toBeVisible();
}

async function inside(locator: Locator, container: Locator) {
  const item = (await locator.boundingBox())!;
  const bounds = (await container.boundingBox())!;
  expect(item.x).toBeGreaterThanOrEqual(bounds.x);
  expect(item.x + item.width).toBeLessThanOrEqual(bounds.x + bounds.width + 1);
  expect(item.y).toBeGreaterThanOrEqual(bounds.y);
  expect(item.y + item.height).toBeLessThanOrEqual(bounds.y + bounds.height + 1);
}

// Cover the two-column and stacked layouts without a locale/viewport matrix.
for (const [width, height] of [[1280, 800], [390, 720]] as const) {
  test(`launcher layout: readable settings and fixed actions at ${width}×${height}`, { tag: "@webkit" }, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await seed(page);
    await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
    const scroll = page.locator(".launcher-scroll");
    const footer = page.locator(".launcher-footer");
    await expect(page.locator("#d-model")).toContainText("GPT-5.6-Sol");
    const footerTop = (await footer.boundingBox())!.y;
    if (width === 1280) expect(await scroll.evaluate(el => el.scrollHeight - el.clientHeight)).toBeLessThanOrEqual(1);
    for (const id of ["d-project", "d-base", "d-issuebtn", "d-model", "d-effort", "d-account", "d-mcp", "d-plugins"]) {
      const control = page.locator(`#${id}`);
      await control.scrollIntoViewIfNeeded();
      await inside(control, scroll);
      expect(await control.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
      expect((await footer.boundingBox())!.y).toBe(footerTop);
    }
    await inside(page.locator("#d-go"), footer);
    expect((await footer.boundingBox())!.y + (await footer.boundingBox())!.height).toBeLessThanOrEqual(height);
    await expect(page.locator(".launcher-account-name")).toHaveText(email);
    expect(await scroll.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);

    await page.locator("#d-account").click();
    const dialog = page.locator(".account-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.locator("strong")).toHaveText(email);
    await inside(dialog.locator(".ui-form-footer .pri"), dialog);
    expect(await dialog.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    await page.keyboard.press("Escape");
    await expect(page.locator("#d-account")).toBeFocused();

    await page.locator("#d-model").click();
    await expect(page.locator(".ui-search-picker")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator("#d-model")).toBeFocused();
    await expect(page.locator(".launcher")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator("#veil")).toBeHidden();
  });
}

test("launcher layout: native branch controls preserve worktree and multiple-repository rules", async ({ page }) => {
  await seed(page);
  await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
  await expect(page.locator("#d-nb")).toBeChecked();
  await expect(page.locator("#d-nb")).toBeEnabled();
  await page.locator("#d-nb").uncheck();
  await expect(page.locator("#d-go")).toBeDisabled();
  await page.locator("#d-base").click();
  await page.locator("#d-picker").getByRole("button", { name: "origin/coworker-feature" }).click();
  await expect(page.locator("#d-basename")).toHaveText("origin/coworker-feature");
  await expect(page.locator("#d-go")).toBeEnabled();
  await page.locator("#d-wt").uncheck();
  await expect(page.locator("#d-base")).toBeDisabled();
  await page.locator("#d-more").click();
  await page.getByRole("menuitem", { name: "prometeu", exact: true }).click();
  await expect(page.locator("#d-wt")).toBeChecked();
  await expect(page.locator("#d-wt")).toBeDisabled();
  await expect(page.locator("#d-nb")).toBeChecked();
  await expect(page.locator("#d-nb")).toBeDisabled();
  await expect(page.locator("#d-hint")).not.toContainText("coworker-feature");
  await page.getByTitle("Remove prometeu from this workspace", { exact: true }).click();
  await expect(page.locator("#d-wt")).toBeEnabled();
});

// Creation recovery needs browser coverage: a toast truncated the instructions,
// while unit tests cannot verify wrapping and scroll access in the real layout.
test("launcher errors keep complete upgrade instructions readable until dismissed", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 720 });
  await seed(page);
  await page.evaluate(() => {
    const backend = (window as unknown as { __TAURI_INTERNALS__: {
      invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
    } }).__TAURI_INTERNALS__;
    const invoke = backend.invoke;
    backend.invoke = (command, args) => command === "create_workspace"
      ? Promise.reject('i18n:{"code":"err.windows.runtimeUpgrade"}')
      : invoke(command, args);
  });
  await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
  await page.locator("#d-go").click();
  const dialog = page.getByRole("dialog", { name: "Could not create workspace" });
  const message = dialog.locator(".ui-form-body p");
  await expect(message).toContainText("Also close all project terminal tabs.");
  await expect(message).toContainText("then unarchive those workspaces.");
  await expect(message).toContainText("The chat’s Stop button does not release the runtime.");
  expect(await message.evaluate(el => {
    const style = getComputedStyle(el);
    return style.whiteSpace !== "nowrap" && el.scrollWidth <= el.clientWidth + 1 && el.scrollHeight <= el.clientHeight + 1;
  })).toBe(true);
  await inside(message, dialog.locator(".ui-form-body"));
  await page.clock.install();
  await page.clock.fastForward(10_000);
  await expect(dialog).toBeVisible();
  const close = dialog.getByRole("button", { name: "Close", exact: true });
  await inside(close, dialog);
  await close.click();
  await expect(dialog).toBeHidden();
});
