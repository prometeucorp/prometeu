import { expect, test, type Page } from "@playwright/test";

async function boot(page: Page) {
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
}

async function hold(page: Page, command: string) {
  await page.evaluate(command => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const target = window as unknown as {
      __TAURI_INTERNALS__: { invoke: Invoke };
      auditWaiting: boolean;
      auditRelease: (error?: string) => void;
    };
    const original = target.__TAURI_INTERNALS__.invoke;
    let release!: (error?: string) => void;
    const ready = new Promise<string | undefined>(done => { release = done; });
    target.auditWaiting = false;
    target.__TAURI_INTERNALS__.invoke = async (next, args) => {
      if (next === command) {
        target.auditWaiting = true;
        const error = await ready;
        if (error) throw new Error(error);
      }
      return original(next, args);
    };
    target.auditRelease = error => {
      target.__TAURI_INTERNALS__.invoke = original;
      release(error);
    };
  }, command);
}
async function waiting(page: Page) {
  await expect.poll(() => page.evaluate(() => (window as unknown as { auditWaiting: boolean }).auditWaiting)).toBe(true);
}
async function release(page: Page, error?: string) {
  await page.evaluate(error => (window as unknown as { auditRelease: (error?: string) => void }).auditRelease(error), error);
}

async function openFile(page: Page, path: string) {
  await page.locator("#tree .treerow", { hasText: path }).click();
  await expect(page.locator("#vcrumb")).toContainText(path);
}

async function openEditor(page: Page) {
  await boot(page);
  await page.locator("#railbody .navitem.sub .lbl").getByText("Hello", { exact: true }).click();
  await page.locator("#tab-files").click();
  await openFile(page, "CLAUDE.md");
}

test("file saving preserves edits made while the write is pending", { tag: "@webkit" }, async ({ page }) => {
  await openEditor(page);
  const otherRow = await page.locator("#tree .treerow", { hasText: ".gitignore" }).elementHandle();
  await page.locator("#vtext").fill("submitted text");
  await hold(page, "write_file");
  await page.locator("#vsave").click();
  await waiting(page);
  await expect(page.locator("#vsave")).toBeDisabled();
  await page.locator("#vtext").fill("newer draft");
  await release(page);
  await expect(page.locator("#vsave")).toBeEnabled();
  await expect(page.locator("#vtext")).toHaveValue("newer draft");
  // Saving this file updates marks without detaching another file's click target.
  await page.waitForTimeout(300);
  expect(await otherRow!.evaluate(node => node.isConnected)).toBe(true);
  await openFile(page, ".gitignore");
  await openFile(page, "CLAUDE.md");
  await expect(page.locator("#vtext")).toHaveValue("newer draft");
  await page.locator("#vsave").click();
  await expect(page.locator("#vsave")).toBeHidden();
});

test("finishing a save preserves another file's selection and draft", { tag: "@webkit" }, async ({ page }) => {
  await openEditor(page);
  await page.locator("#vtext").fill("submitted text");
  await hold(page, "write_file");
  await page.locator("#vsave").click();
  await waiting(page);
  await openFile(page, ".gitignore");
  await page.locator("#vtext").fill("other file draft");
  await release(page);
  await expect(page.locator("#vcrumb")).toContainText(".gitignore");
  await expect(page.locator("#vtext")).toHaveValue("other file draft");
  await openFile(page, "CLAUDE.md");
  await openFile(page, ".gitignore");
  await expect(page.locator("#vtext")).toHaveValue("other file draft");
});

test("file saving preserves rejected edits and clears a rejected undo-to-original draft", async ({ page }) => {
  await openEditor(page);
  const original = await page.locator("#vtext").inputValue();
  await page.locator("#vtext").fill("submitted text");
  await hold(page, "write_file");
  await page.locator("#vsave").click();
  await waiting(page);
  await page.locator("#vtext").fill("newer draft");
  await release(page, "write rejected");
  await expect(page.locator("#vsave")).toBeEnabled();
  await openFile(page, ".gitignore");
  await openFile(page, "CLAUDE.md");
  await expect(page.locator("#vtext")).toHaveValue("newer draft");

  await hold(page, "write_file");
  await page.locator("#vsave").click();
  await waiting(page);
  await page.locator("#vtext").fill(original);
  await release(page, "write rejected");
  await expect(page.locator("#msg")).toContainText("write rejected");
  await expect(page.locator("#vsave")).toBeHidden();
  await expect(page.locator("#vcrumb")).not.toHaveClass(/\bdirty\b/);
});

test("file saving retains undo-to-original as a draft after a successful pending write", async ({ page }) => {
  await openEditor(page);
  const original = await page.locator("#vtext").inputValue();
  await page.locator("#vtext").fill("submitted text");
  await hold(page, "write_file");
  await page.locator("#vsave").click();
  await waiting(page);
  await page.locator("#vtext").fill(original);
  await release(page);
  await expect(page.locator("#vsave")).toBeEnabled();
  await openFile(page, ".gitignore");
  await openFile(page, "CLAUDE.md");
  await expect(page.locator("#vtext")).toHaveValue(original);
  await page.locator("#vsave").click();
  await expect(page.locator("#vsave")).toBeHidden();
});

test("file saving keeps the selected draft when an older file read finishes", async ({ page }) => {
  await openEditor(page);
  await page.locator("#vtext").fill("selected draft");
  await hold(page, "read_file");
  await page.locator("#tree .treerow", { hasText: ".gitignore" }).click();
  await waiting(page);
  await page.locator('#tabbar .tab').filter({ hasText: "CLAUDE.md" }).click();
  await release(page);
  await expect(page.locator("#vcrumb")).toContainText("CLAUDE.md");
  await expect(page.locator("#vtext")).toHaveValue("selected draft");
});

test("cleanup keeps its dialog open while deleting worktrees", async ({ page }) => {
  await boot(page);
  await page.getByRole("button", { name: /^Archived/ }).click();
  await page.locator("#aclean").click();
  const dialog = page.locator("dialog.clean");
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();
  await expect(dialog).toContainText("existing branches stay");
  await expect(dialog.locator("#c-go")).toBeEnabled();
  await hold(page, "cleanup_worktree");
  await dialog.locator("#c-go").click();
  await waiting(page);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(":scope > form")).toHaveAttribute("aria-busy", "true");
  await release(page);
  await expect(dialog).toHaveCount(0);
});

test("archiving offers cleanup for only that workspace", async ({ page }) => {
  await boot(page);
  const workspace = page.locator('.railworkspace[data-workspace="sessao-0929"] .navitem.sub');
  await workspace.click({ button: "right" });
  await page.locator(".menu .mrow", { hasText: "Archive" }).click();

  const dialog = page.locator("dialog.clean");
  await expect(dialog).toContainText("Workspace archived. Remove the worktree from disk?");
  await expect(dialog.getByRole("button", { name: "Keep worktree", exact: true })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toHaveCount(0);
  await expect(dialog.locator(".cleanrow")).toHaveCount(1);
  await expect(dialog.locator(".cleanrow")).toContainText("Hello");
  await expect(dialog.locator("#c-go")).toBeEnabled();
  await dialog.locator("#c-go").click();
  await expect(dialog).toHaveCount(0);

  await page.getByRole("button", { name: /^Archived/ }).click();
  await expect(page.locator(".arow", { hasText: "Hello" })).toContainText("worktree removed");
});

test("finishing offers cleanup for only that workspace", async ({ page }) => {
  await boot(page);
  const workspace = page.locator('.railworkspace[data-workspace="dock-1130"] .navitem.sub');
  await workspace.click({ button: "right" });
  await page.locator(".menu .mrow", { hasText: "Finish" }).click();

  const dialog = page.locator("dialog.clean");
  await expect(dialog).toContainText("Workspace archived. Remove the worktree from disk?");
  await expect(dialog.locator(".cleanrow")).toHaveCount(1);
  await expect(dialog.locator(".cleanrow")).toContainText("Dock port per worktree");
  await dialog.getByRole("button", { name: "Keep worktree", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  await page.getByRole("button", { name: /^Archived/ }).click();
  await expect(page.locator(".arow", { hasText: "Dock port per worktree" })).not.toContainText("worktree removed");
});

test("cleanup keeps the archived worktree available to restore", async ({ page }) => {
  await boot(page);
  const workspace = page.locator('.railworkspace[data-workspace="sessao-0929"] .navitem.sub');
  await workspace.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Archive", exact: true }).click();

  const dialog = page.locator("dialog.clean");
  await expect(dialog).toContainText("Workspace archived. Remove the worktree from disk?");
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toHaveCount(0);
  await dialog.getByRole("button", { name: "Keep worktree", exact: true }).click();
  await expect(dialog).toHaveCount(0);

  await page.getByRole("button", { name: /^Archived/ }).click();
  const archived = page.locator(".arow", { hasText: "Hello" });
  await expect(archived).toBeVisible();
  await archived.getByRole("button", { name: "Unarchive", exact: true }).click();
  await expect(workspace).toBeVisible();
});
