import { expect, test } from "@playwright/test";

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
  await page.locator(".setnavitem", { hasText: "Work and team" }).click();
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
