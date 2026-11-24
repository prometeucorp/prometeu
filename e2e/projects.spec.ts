import { expect, test } from "./fixtures";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "1", name: "Person", email: "me@example.test" }, origin: "https://app.prometeu.co", offline: false }));
    localStorage.setItem("mock:catalog", JSON.stringify({ connected: true, revision: 0, plugins: [], mcp: [], skills: [], shared: {}, projects: [
      { id: "personal-app", source: "team/personal", note: "Personal app", revision: 0, organization: null, organization_name: null, local_path: null },
    ] }));
    localStorage.setItem("mock:organizationCatalogs", JSON.stringify([
      { id: "acme", name: "Acme team", revision: 0, links: {}, plugins: [], mcp: [], skills: [], projects: [
        { id: "team-app", source: "team/shared", note: "Team app" },
        { id: "local-prometeu", source: "prometeucorp/prometeu", note: "Already registered" },
      ] },
    ]));
    localStorage.setItem("mock:projectOrigins", JSON.stringify({ "/Users/gustavo/dev/prometeu": "prometeucorp/prometeu" }));
    localStorage.setItem("mock:directory", "/tmp/projects");
  });
  await page.goto("/");
  await expect(page.locator("#tiles .tile").first()).toBeVisible();
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Projects" }).click();
  await page.getByRole("button", { name: "Add to this computer" }).click();
});

test("Git projects open without a selected button and keep compact actions readable", { tag: "@webkit" }, async ({ page }) => {
  const dialog = page.getByRole("dialog", { name: "Projects", exact: true });
  const local = dialog.getByRole("button", { name: "Add local folder", exact: true });
  await expect(dialog.getByText("local-prometeu")).toHaveCount(0);
  await expect(dialog.locator(".sheettop b")).toBeFocused();
  await expect(dialog.locator("button:focus-visible")).toHaveCount(0);
  await page.keyboard.press("Tab");
  await expect(local).toBeFocused();
  await expect(local).toHaveCSS("outline-style", "solid");
  expect((await local.boundingBox())!.width).toBeLessThan((await dialog.boundingBox())!.width / 2);
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 480 });
    expect(await dialog.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    const save = dialog.getByRole("button", { name: "Add to this computer", exact: true });
    await expect(save).toBeVisible();
    const bounds = (await save.boundingBox())!;
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(480);
  }
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Add to this computer", exact: true })).toBeFocused();
});

test("Git projects clone in a batch, retain success and retry only failures", async ({ page }) => {
  const dialog = page.getByRole("dialog", { name: "Projects", exact: true });
  const personal = dialog.getByRole("checkbox", { name: /personal-app/ });
  const shared = dialog.getByRole("checkbox", { name: /team-app/ });
  await expect(shared).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Add to this computer" })).toBeDisabled();
  await personal.check(); await shared.check();
  await dialog.getByRole("button", { name: "Choose destination folder" }).click();
  await page.evaluate(() => localStorage.setItem("mock:projectFail", "team-app"));
  await dialog.getByRole("button", { name: "Add to this computer" }).click();
  await expect(dialog.getByRole("alert")).toContainText("Some projects failed");
  const failure = dialog.locator(".setrow", { hasText: "team-app" }).getByRole("status");
  await expect(failure).toContainText("Repository access denied");
  await expect(failure).toContainText("Permission denied (publickey).");
  await expect(failure).toHaveClass(/bad/);
  await expect(personal).not.toBeChecked();
  await expect(shared).toBeChecked();
  await expect(dialog).toContainText("/tmp/projects/personal-app");
  await page.evaluate(() => localStorage.removeItem("mock:projectFail"));
  await dialog.getByRole("button", { name: "Add to this computer" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.locator("#railbody")).toContainText("personal-app");
  await expect(page.locator("#railbody")).toContainText("team-app");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:catalog")!).revision)).toBe(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("mock:catalog")!).projects)).toHaveLength(1);
});

// Protect the project-to-conversation journey and typed input across nested dialog failure/retry.
// Rust tests cover filesystem behavior; only the browser proves focus and launcher wiring,
// including retaining newer repository choices and the typed prompt during branch discovery.
test("a new project preserves its name on failure and starts its first workspace without a worktree", async ({ page }) => {
  const projects = page.getByRole("dialog", { name: "Projects", exact: true });
  await projects.getByRole("button", { name: "Create new project" }).click();
  const dialog = page.getByRole("dialog", { name: "Create new project", exact: true });
  const name = dialog.getByRole("textbox", { name: "Project name" });
  await expect(name).toBeFocused();
  await name.fill("prometeu");
  await page.evaluate(() => localStorage.setItem("mock:directory", "/Users/gustavo/dev"));
  await dialog.getByRole("button", { name: "Parent folder" }).click();
  await name.press("Enter");
  await expect(dialog.getByRole("alert")).toContainText("already exists");
  await expect(name).toHaveValue("prometeu");
  await name.fill("New project");
  await page.evaluate(() => localStorage.setItem("mock:directory", "/tmp/projects"));
  await dialog.getByRole("button", { name: "Parent folder" }).click();
  await name.press("Enter");
  await expect(dialog).not.toBeVisible();
  await expect(projects).not.toBeVisible();
  await page.getByRole("button", { name: "New workspace in New project", exact: true }).click();
  await expect(page.locator("#d-project")).toContainText("New project");
  await expect(page.locator("#d-wt")).toBeDisabled();
  await expect(page.locator("#d-wt")).not.toBeChecked();
  await expect(page.locator("#d-nb")).toBeDisabled();
  await expect(page.locator("#d-repo-hint")).toContainText("no commits yet");
  await page.locator("#d-prompt").fill("Start my new project");
  const selectProject = async (name: string) => {
    await page.locator("#d-project").click();
    await page.getByRole("menuitemcheckbox", { name, exact: true }).click();
  };
  await selectProject("prometeu");
  await expect(page.locator("#d-wt")).toBeEnabled();
  await expect(page.locator("#d-wt")).toBeChecked();
  await expect(page.locator("#d-nb")).toBeChecked();
  await page.locator("#d-nb").uncheck();
  await selectProject("New project");
  await expect(page.locator("#d-wt")).toBeDisabled();
  await selectProject("njord");
  await expect(page.locator("#d-wt")).toBeEnabled();
  await expect(page.locator("#d-wt")).toBeChecked();
  await expect(page.locator("#d-nb")).not.toBeChecked();
  await page.locator("#d-wt").uncheck();
  await selectProject("New project");
  await expect(page.locator("#d-wt")).toBeDisabled();
  await page.evaluate(() => {
    type Invoke = (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    const w = window as Window & { __TAURI_INTERNALS__: { invoke: Invoke }; releaseBranches?: () => void };
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async function (command, args, options) {
      if (command === "list_branches" && args?.project === "p2") {
        w.__TAURI_INTERNALS__.invoke = original;
        await new Promise<void>(resolve => { w.releaseBranches = resolve; });
      }
      return original.call(this, command, args, options);
    };
  });
  await selectProject("prometeu");
  await page.locator("#d-more").click();
  await page.getByRole("menuitem", { name: "njord", exact: true }).click();
  await expect(page.locator("#d-wt")).toBeChecked();
  await page.evaluate(() => (window as Window & { releaseBranches: () => void }).releaseBranches());
  await expect(page.locator("#d-basename")).toHaveText("origin/main");
  await expect(page.locator("#d-wt")).toBeChecked();
  await expect(page.locator("#d-nb")).toBeChecked();
  await expect(page.locator("#d-wt")).toBeDisabled();
  await expect(page.locator("#d-nb")).toBeDisabled();
  await selectProject("New project");
  await expect(page.locator("#d-wt")).toBeDisabled();
  await expect(page.locator("#d-wt")).not.toBeChecked();
  await expect(page.locator("#d-prompt")).toHaveValue("Start my new project");
  await page.locator("#d-prompt").press("Enter");
  await expect(page.locator("#crumb")).toContainText("Start my new project");
  await expect(page.locator("#chatwrap .composer textarea")).toBeVisible();
});

// Remote projects are installed in Settings before launching; the launcher only offers the
// selected machine's registered projects. The clone retry proves the dialog keeps its input.
test("remote projects stay on their machine and install from settings", async ({ page }) => {
  await page.keyboard.press("Escape");
  await page.evaluate(async () => {
    const app = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> } };
    for (const id of ["vps", "other"]) {
      await app.__TAURI_INTERNALS__.invoke("machine_save", { machine: { id, name: id, ssh: id, limits: { memory_max: "", cpu_quota: "" } } });
      await app.__TAURI_INTERNALS__.invoke("machine_install", { id });
      await app.__TAURI_INTERNALS__.invoke("machine_claude_code", { id, code: "mock-code" });
    }
    await app.__TAURI_INTERNALS__.invoke("remote_add_project", { machine: "other", path: "/home/me/other-project", url: null });
  });
  await page.getByRole("button", { name: "Computer", exact: true }).click();
  await page.getByRole("menuitemcheckbox", { name: "vps", exact: true }).click();
  const settings = page.locator("#settingsView");
  await expect(settings).toContainText("No projects registered");
  await expect(settings.locator(".desktop-item")).toHaveCount(0);
  await settings.getByRole("button", { name: "Add existing folder", exact: true }).click();
  const folders = page.getByRole("dialog", { name: "Add projects on vps", exact: true });
  await folders.getByRole("button", { name: "code", exact: true }).click();
  await folders.getByRole("checkbox", { name: "api", exact: true }).check();
  await folders.getByRole("button", { name: "Add projects", exact: true }).click();
  await expect(folders).not.toBeVisible();
  await expect(settings.getByRole("button", { name: "Add existing folder", exact: true })).toBeFocused();
  await expect(settings.locator(".desktop-item")).toHaveCount(1);
  await expect(settings.locator(".desktop-item")).toContainText("/home/me/code/api");

  await settings.getByRole("button", { name: "Clone repository", exact: true }).click();
  const clone = page.getByRole("dialog", { name: "Clone repository on vps", exact: true });
  await clone.getByRole("textbox", { name: "Repository URL", exact: true }).fill("git@github.com:team/new-app.git");
  await page.evaluate(() => {
    const app = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> } };
    const original = app.__TAURI_INTERNALS__.invoke;
    app.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "remote_add_project") {
        app.__TAURI_INTERNALS__.invoke = original;
        throw "Permission denied (publickey).";
      }
      return original(command, args);
    };
  });
  await clone.getByRole("button", { name: "Clone repository", exact: true }).click();
  await expect(clone.getByRole("alert")).toContainText("Permission denied (publickey).");
  await expect(clone.getByRole("textbox", { name: "Repository URL", exact: true })).toHaveValue("git@github.com:team/new-app.git");
  await clone.getByRole("button", { name: "Clone repository", exact: true }).click();
  await expect(clone).not.toBeVisible();
  await expect(settings.locator(".desktop-item")).toHaveCount(2);

  await page.getByRole("button", { name: "New workspace in api", exact: true }).click();
  await expect(page.locator(".launcher").getByRole("button", { name: "Add existing folder", exact: true })).toHaveCount(0);
  await page.locator("#d-project").click();
  await expect(page.getByRole("menuitemcheckbox", { name: "new-app", exact: true })).toHaveCount(1);
  await expect(page.getByRole("menuitemcheckbox", { name: "prometeu", exact: true })).toHaveCount(0);
  await expect(page.getByRole("menuitemcheckbox", { name: "other-project", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.locator("#d-machine").click();
  await page.getByRole("menuitemcheckbox", { name: "This computer", exact: true }).click();
  await expect(page.locator("#d-project")).toContainText("njord");
});
