import { expect, test, type Page } from "@playwright/test";

async function open(page: Page, title = "Hello") {
  await page.goto("/");
  await page.locator("#railbody .navitem.sub .lbl").getByText(title, { exact: true }).click();
  await page.locator("#tab-diff").click();
  await expect(mode(page, "changes")).toHaveAttribute("aria-current", "true");
}
const group = (page: Page, scope: string) => page.locator(`.git-group[data-scope="${scope}"]`);
const mode = (page: Page, scope: string) => page.locator(`.git-nav [data-mode="${scope}"]`);

test("Git: partial staging shows two diffs and commits exclude later changes", { tag: "@webkit" }, async ({ page }) => {
  await open(page);
  const staged = group(page, "staged"), changes = group(page, "changes");
  await expect(changes.locator('.git-file[data-path="src/style.css"]')).toHaveCount(1);
  await expect(staged).toHaveCount(0);
  await expect(page.locator("#git-message")).toHaveCount(0);
  await mode(page, "staged").click();
  await expect(staged.locator('.git-file[data-path="src/style.css"]')).toHaveCount(1);
  await staged.locator(".git-file-name").first().click();
  await expect(page.locator(".git-review-scope")).toContainText("Staged changes");
  await expect(page.locator('#dlist .dfile[data-key$="src/style.css"] .drow').first()).toBeVisible();
  const patchCells = () => page.locator('#dlist .dfile[data-key$="src/style.css"] .drow > :is(.dhunk, .dno, .dsign, code)');
  const stagedPatch = await patchCells().allTextContents();
  await mode(page, "changes").click();
  await changes.locator('.git-file[data-path="src/style.css"] .git-file-name').click();
  await expect(page.locator(".git-review-scope")).toContainText("Working tree changes");
  expect(await patchCells().allTextContents()).not.toEqual(stagedPatch);
  // One click opens the entire group as a stacked diff; remaining files are reached by scrolling.
  await expect(page.locator("#dlist .dfile")).toHaveCount(2);
  await expect(page.locator('#dlist .dfile[data-key$="public/logo.png"]')).toBeVisible();
  await mode(page, "staged").click();
  await page.locator("#git-message").fill("   ");
  await expect(page.locator("#git-commit")).toBeDisabled();
  await page.locator("#git-message").fill("fix: stage only the reviewed snapshot");
  await page.locator("#git-commit").click();
  await expect(staged.locator(".git-file")).toHaveCount(0);
  await expect(mode(page, "staged")).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("button", { name: "Push ↑2", exact: true })).toBeEnabled();
  await expect(page.locator("#git-message")).toHaveValue("");
  await page.getByRole("button", { name: "Push ↑2", exact: true }).click();
  await expect(page.getByRole("button", { name: "Push ↑0", exact: true })).toBeDisabled();
  await mode(page, "changes").click();
  await expect(changes.locator('.git-file[data-path="src/style.css"]')).toHaveCount(1);
  await mode(page, "history").click();
  await expect(page.locator(".git-history-row").first()).toContainText("reviewed snapshot");
  await expect(page.locator("#dlist .dfile")).toHaveCount(1);
  await expect(patchCells()).toHaveText(stagedPatch);
  const previous = page.locator(".git-history-row").nth(1);
  await previous.focus();
  await page.keyboard.press("Enter");
  await expect(previous).toHaveAttribute("aria-current", "true");
  await expect(previous).toBeFocused();
});

test("Git: commit drafts survive refresh and errors preserve the stage", async ({ page }) => {
  await open(page);
  await mode(page, "staged").click();
  await page.locator("#git-message").fill("message still being edited");
  await page.evaluate(() => {
    const internals = (window as any).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = (command: string, args: any, options: any) => command === "workspace_git_action" && args.operation === "commit"
      ? Promise.reject('i18n:{"code":"err.git.changed"}') : original(command, args, options);
  });
  await page.locator(".git-panel").getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.locator("#git-message")).toHaveValue("message still being edited");
  await page.locator("#git-commit").click();
  await expect(page.locator("#msg")).toContainText("Git state changed");
  await expect(group(page, "staged").locator(".git-file")).toHaveCount(1);
  await expect(page.locator("#git-message")).toHaveValue("message still being edited");
});

test("Git: conflicts use the reviewed result and allow completing a merge without index changes", async ({ page }) => {
  await open(page, "App icon");
  await mode(page, "staged").click();
  await expect(page.locator("#git-commit")).toBeDisabled();
  await mode(page, "changes").click();
  await group(page, "conflict").locator(".git-file-name").click();
  await page.getByRole("button", { name: "Use current", exact: true }).click();
  await page.getByRole("button", { name: "Save resolution", exact: true }).click();
  await expect(group(page, "conflict")).toHaveCount(0);
  await mode(page, "staged").click();
  await expect(group(page, "staged").locator(".git-file")).toHaveCount(0);
  await page.locator("#git-message").fill("merge: preserve the current version");
  await expect(page.locator("#git-commit")).toHaveText("Complete merge");
  await expect(page.locator("#git-commit")).toBeEnabled();
  await page.locator("#git-commit").click();
  await expect(page.locator("#git-commit")).toHaveText("Commit 0 files");
  await expect(page.locator("#git-commit")).toBeDisabled();
});

test("Git: temporary status errors preserve drafts and restore the conflict editor", async ({ page }) => {
  await open(page, "App icon");
  await mode(page, "staged").click();
  await page.locator("#git-message").fill("merge: unsent draft");
  await mode(page, "changes").click();
  await group(page, "conflict").locator(".git-file-name").click();
  await page.locator(".git-conflict-result").fill(".card {\n  gap: 10px;\n}\n");
  await page.evaluate(() => {
    const w = window as any, original = w.__TAURI_INTERNALS__.invoke;
    w.failGitStatus = true;
    w.__TAURI_INTERNALS__.invoke = (command: string, args: any, options: any) =>
      command === "workspace_git_status" && w.failGitStatus
        ? Promise.reject('i18n:{"code":"err.git.changed"}') : original(command, args, options);
  });
  const refresh = page.locator(".git-panel").getByRole("button", { name: "Refresh", exact: true });
  await refresh.click();
  await expect(page.locator("#dlist .git-error")).toContainText("Git state changed");
  await page.evaluate(() => { (window as any).failGitStatus = false; });
  await refresh.click();
  await expect(page.locator("#dlist .git-error")).toHaveCount(0);
  await expect(page.locator(".git-conflict-result")).toHaveValue(".card {\n  gap: 10px;\n}\n");
  await expect(page.getByRole("button", { name: "Save resolution", exact: true })).toBeEnabled();
  await mode(page, "staged").click();
  await expect(page.locator("#git-message")).toHaveValue("merge: unsent draft");
  await expect(page.locator("#git-commit")).toBeDisabled();
});

test("Git: stale stage buttons cannot stage another repository’s files while loading", async ({ page }) => {
  await open(page, "Hiring through the portal");
  await group(page, "changes").locator('.git-file[data-path="src/style.css"] .git-file-name').click();
  const stageAll = group(page, "changes").getByRole("button", { name: "Stage all", exact: true });
  await expect(stageAll).toBeEnabled();
  await stageAll.evaluate(button => { (window as any).oldGitStage = button; });
  await page.evaluate(() => {
    const w = window as any, original = w.__TAURI_INTERNALS__.invoke;
    w.gitActions = [];
    w.delayedGitDiffs = [];
    w.delayGitDiff = true;
    w.__TAURI_INTERNALS__.invoke = (command: string, args: any, options: any) => {
      if (command === "workspace_git_action") w.gitActions.push(structuredClone(args));
      if (command === "workspace_git_diff" && args.repo === 1 && w.delayGitDiff) {
        return new Promise(resolve => w.delayedGitDiffs.push(() => resolve(original(command, args, options))));
      }
      return original(command, args, options);
    };
  });
  await page.getByRole("button", { name: "Repository", exact: true }).click();
  await page.locator(".menu .mrow", { hasText: "njord" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).delayedGitDiffs.length)).toBeGreaterThan(0);
  await expect(page.getByRole("button", { name: "Repository", exact: true })).toContainText("njord");
  expect(await page.evaluate(() => (window as any).oldGitStage.isConnected)).toBe(false);
  await page.evaluate(() => { (window as any).oldGitStage.click(); });
  expect(await page.evaluate(() => (window as any).gitActions)).toEqual([]);
  await page.evaluate(() => {
    const w = window as any;
    w.delayGitDiff = false;
    w.delayedGitDiffs.splice(0).forEach((release: () => void) => release());
  });
  await expect(group(page, "changes").locator(".git-file")).toHaveCount(0);
  await expect(mode(page, "changes")).toHaveAttribute("aria-current", "true");
  await mode(page, "staged").click();
  await expect(page.locator("#dlist .dfile").first()).toBeVisible();
  await expect(group(page, "staged").locator(".git-file")).toHaveCount(1);
  expect(await page.evaluate(() => (window as any).gitActions)).toEqual([]);
});

test("Git: keyboard review preserves the stage and a new patch requires another review", { tag: "@webkit" }, async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    const w = window as any, original = w.__TAURI_INTERNALS__.invoke;
    w.gitActions = [];
    w.changedPatch = false;
    w.__TAURI_INTERNALS__.invoke = async (command: string, args: any, options: any) => {
      if (command === "workspace_git_action") w.gitActions.push(structuredClone(args));
      const result = await original(command, args, options);
      if (command === "workspace_git_diff" && args.scope === "changes" && w.changedPatch) {
        const file = result.files.find((file: any) => file.path === "src/style.css");
        file.patch += "\n@@ -400,0 +401,1 @@\n+/* change after review */";
        file.added++;
      }
      return result;
    };
  });
  await group(page, "changes").locator('.git-file[data-path="src/style.css"] .git-file-name').click();
  const file = page.locator('#dlist .dfile[data-key$="src/style.css"]');
  const reviewed = file.locator('input.dseen[type="checkbox"]');
  await expect(page.locator("#dprogress")).toContainText(/0.*2/);
  await reviewed.focus();
  await page.keyboard.press("Space");
  await expect(reviewed).toBeChecked();
  await expect(reviewed).toBeFocused();
  await expect(file.locator(".dbody")).toBeVisible();
  await expect(page.locator("#dprogress")).toContainText(/1.*2/);
  await expect(group(page, "changes").locator(".git-file")).toHaveCount(2);

  await page.locator("#dnext button").click();
  const binary = page.locator('#dlist .dfile[data-key$="public/logo.png"]');
  await expect(binary.locator(".dnote")).toBeVisible();
  await binary.locator(".dseen").check();
  await expect(page.locator("#dprogress")).toContainText(/2.*2/);
  await expect(page.locator("#dnext button")).toBeDisabled();
  await mode(page, "staged").click();
  await expect(group(page, "staged").locator(".git-file")).toHaveCount(1);
  await expect(file.locator(".dseen")).not.toBeChecked();
  await mode(page, "changes").click();
  await expect(reviewed).toBeChecked();

  const refresh = page.locator(".git-panel").getByRole("button", { name: "Refresh", exact: true });
  await refresh.click();
  await expect(reviewed).toBeChecked();
  await page.evaluate(() => { (window as any).changedPatch = true; });
  await refresh.click();
  await expect(file.locator(".dbody")).toContainText("change after review");
  await expect(reviewed).not.toBeChecked();
  await expect(binary.locator(".dseen")).toBeChecked();
  await expect(page.locator("#dprogress")).toContainText(/1.*2/);
  await expect(page.locator("#dnext button")).toBeEnabled();
  expect(await page.evaluate(() => (window as any).gitActions)).toEqual([]);

  await file.locator(".dbody").focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("c");
  await page.getByRole("textbox", { name: "Review note", exact: true }).fill("Use the shared spacing tokens.");
  // Removing a file from the rendered snapshot must not discard unfinished writing.
  await page.locator("#git-filter").fill("logo");
  await expect(page.getByRole("textbox", { name: "Review note", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Review notes (1)", exact: true }).click();
  await page.getByRole("button", { name: "Continue editing", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Review note", exact: true })).toHaveValue("Use the shared spacing tokens.");
  await page.getByRole("textbox", { name: "Review note", exact: true }).focus();
  await page.keyboard.press("Control+Enter");
  await page.locator("#git-filter").fill("");
  await expect(file.locator(".review-note")).toContainText("Use the shared spacing tokens.");
  await open(page);
  await group(page, "changes").locator('.git-file[data-path="src/style.css"] .git-file-name').click();
  await expect(file.locator(".review-note")).toContainText("Use the shared spacing tokens.");
  await page.getByRole("button", { name: "Review notes (1)", exact: true }).click();
  const summary = page.getByRole("dialog");
  await summary.getByRole("button", { name: "Send to agent (1)", exact: true }).click();
  await page.getByRole("menuitem").first().click();
  await expect(page.locator(".composer .review-context")).toHaveCount(1);
  await expect(page.locator("#chatwrap .composer textarea")).toHaveValue("");
  await page.evaluate(() => {
    const w = window as any, original = w.__TAURI_INTERNALS__.invoke;
    w.reviewSends = [];
    w.__TAURI_INTERNALS__.invoke = (command: string, args: any, options: any) => {
      if (command === "chat_send") w.reviewSends.push(args.text);
      return original(command, args, options);
    };
  });
  await page.getByRole("button", { name: "Remove review from draft", exact: true }).click();
  await expect(page.locator("#chatwrap .composer .review-context")).toHaveCount(0);
  for (let round = 0; round < 2; round++) {
    await page.locator("#tab-diff").click();
    await page.getByRole("button", { name: "Review notes (1)", exact: true }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Send to agent (1)", exact: true }).click();
    await page.getByRole("menuitem").first().click();
    await expect(page.locator("#chatwrap .composer .review-context")).toHaveCount(1);
  }
  expect(await page.evaluate(() => (window as any).reviewSends)).toEqual([]);
  await page.locator("#chatwrap .composer textarea").fill("Address these notes.");
  await page.locator("#chatwrap .composer .send").click();
  const sent = await page.evaluate(() => (window as any).reviewSends as string[]);
  expect(sent).toHaveLength(1);
  expect(sent[0]).toMatch(/^<prometeu-review v="1">/);
  expect(sent[0]).toContain("Use the shared spacing tokens.");
  expect(sent[0]).toMatch(/Address these notes\.$/);
  await expect(page.locator("#chatwrap .turn.user .review-context").last()).toBeVisible();
  // Inspect replay after streaming finishes moving the transcript.
  await expect(page.locator("#chatwrap .composer .stop")).toBeHidden();
  await page.locator("#chatwrap .turn.user .review-context").last().getByRole("button").click();
  await expect(page.getByRole("dialog")).toContainText("Use the shared spacing tokens.");
});

test("Git: unified and side-by-side diffs preserve line numbers, review and keyboard collapse", { tag: "@webkit" }, async ({ page }) => {
  await open(page);
  await group(page, "changes").locator('.git-file[data-path="src/style.css"] .git-file-name').click();
  const file = page.locator('#dlist .dfile[data-key$="src/style.css"]');
  const toggle = file.locator(".dtoggle");
  await expect(file.locator(".drow.ctx").first().locator(".dno")).toHaveCount(2);
  await file.locator(".dseen").check();
  await toggle.focus();
  await page.keyboard.press("Enter");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(toggle).toBeFocused();
  await expect(file.locator(".dbody")).toBeHidden();

  const split = page.locator('#dlayout [data-layout="split"]');
  await split.click();
  await expect(split).toHaveAttribute("aria-pressed", "true");
  await expect(file.locator(".dbody")).toBeHidden();
  await expect(file.locator(".dseen")).toBeChecked();
  await toggle.click();
  await expect(file.locator(".dbody.dlayout-split")).toBeVisible();
  const row = file.locator(".drow.dsplit").first();
  await expect(row.locator(":scope > *")).toHaveCount(6);
  await expect(row.locator(".dno")).toHaveCount(2);
  await expect(row.locator("code")).toHaveCount(2);
  await expect(file.locator(".dbody")).toContainText(".foot .chip { padding: 0; }");
  await page.locator('#dlayout [data-layout="unified"]').click();
  await expect(file.locator(".dbody")).toHaveClass(/\bdlayout-unified\b/);
  await expect(file.locator(".dseen")).toBeChecked();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");

  await page.locator('#dlayout [data-layout="split"]').click();
  const oldLine = file.locator('.dno.before[data-review-index]').first();
  await oldLine.hover();
  await oldLine.getByRole("button", { name: "Add review note", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Review note", exact: true })).toBeFocused();
  await page.getByRole("textbox", { name: "Review note", exact: true }).fill("Keep the original behavior.");
  await page.getByRole("button", { name: "Save note", exact: true }).click();
  await expect(file.locator(".review-note")).toContainText("Keep the original behavior.");
  const saved = await page.evaluate(() => {
    const key = Object.keys(localStorage).find(key => key.startsWith("prometeu:review:"))!;
    return JSON.parse(localStorage.getItem(key)!).comments[0];
  });
  expect(saved.anchor.side).toBe("before");
  expect(saved.anchor.old).not.toBeNull();
  expect(saved.anchor.new).toBeNull();
  await file.getByRole("button", { name: "Note on file", exact: true }).click();
  await page.getByRole("textbox", { name: "Review note", exact: true }).fill("Document the change.");
  await page.keyboard.press("Escape");
  await expect(file.locator(".review-note")).toHaveCount(1);
  await file.getByRole("button", { name: "Resolve", exact: true }).click();
  await expect(file.locator(".review-state")).toHaveText("Resolved");
  await file.getByRole("button", { name: "Reopen", exact: true }).click();
  await expect(file.locator(".review-state")).toHaveText("Draft");
});

test("Git: filtering preserves focus, staging preserves scope and drafts stay in the selected repository", async ({ page }) => {
  await open(page, "Hiring through the portal");
  const filter = page.locator("#git-filter");
  const files = group(page, "changes").locator(".git-file");
  await filter.fill("logo");
  await expect(filter).toBeFocused();
  await expect(files).toHaveCount(1);
  await expect(files).toHaveAttribute("data-path", "public/logo.png");
  const stage = files.locator(".git-file-action");
  await expect(stage).not.toHaveText(/^[+−-]$/);
  await expect(stage).toHaveCSS("opacity", "1");
  await stage.click();
  await expect(mode(page, "changes")).toHaveAttribute("aria-current", "true");
  await expect(files).toHaveCount(0);
  await expect(page.locator("#git-message")).toHaveCount(0);
  await page.locator(".git-filter button").click();
  await expect(filter).toHaveValue("");
  await expect(filter).toBeFocused();
  await expect(files).toHaveCount(1);

  await mode(page, "staged").click();
  await expect(group(page, "staged").locator(".git-file")).toHaveCount(2);
  const unstage = group(page, "staged").locator('[data-path="public/logo.png"] .git-file-action');
  await expect(unstage).not.toHaveText(/^[+−-]$/);
  await unstage.click();
  await expect(mode(page, "staged")).toHaveAttribute("aria-current", "true");
  await expect(group(page, "staged").locator(".git-file")).toHaveCount(1);
  await page.locator("#git-message").fill("fix: prometeu draft");
  const picker = page.getByRole("button", { name: "Repository", exact: true });
  await picker.click();
  await page.locator(".menu .mrow", { hasText: "njord" }).click();
  await expect(mode(page, "changes")).toHaveAttribute("aria-current", "true");
  await mode(page, "staged").click();
  await expect(page.locator("#git-message")).toHaveValue("");
  await page.locator("#git-message").fill("fix: njord draft");
  await picker.click();
  await page.locator(".menu .mrow", { hasText: "prometeu" }).click();
  await mode(page, "staged").click();
  await expect(page.locator("#git-message")).toHaveValue("fix: prometeu draft");
});
