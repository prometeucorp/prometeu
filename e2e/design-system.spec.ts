import { expect, test } from "@playwright/test";
import { createServer, type Server } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve, sep } from "node:path";

// Test an independent static consumer without desktop assets or modules.
let server: Server;
let baseURL: string;
test.beforeAll(async () => {
  const root = resolve("packages/design-system");
  server = createServer(async (request, response) => {
    const pathname = decodeURIComponent(new URL(request.url!, "http://localhost").pathname);
    const file = resolve(root, `.${pathname}`);
    if (!file.startsWith(root + sep)) { response.writeHead(404).end(); return; }
    try {
      const content = await readFile(file);
      const type = file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : file.endsWith(".svg") ? "image/svg+xml" : "text/html";
      response.writeHead(200, { "Content-Type": type }); response.end(content);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  baseURL = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
});
test.afterAll(async () => { await new Promise<void>(resolve => server.close(() => resolve())); });

test("design system supports menus, submenus, passwords and recoverable form errors", { tag: "@webkit" }, async ({ page }) => {
  const scripts: string[] = [];
  page.on("request", request => { if (request.resourceType() === "script") scripts.push(request.url()); });
  await page.goto(`${baseURL}/index.html`);
  const password = page.getByLabel("Password", { exact: true });
  await password.fill("safe-example");
  await page.getByRole("button", { name: "Show password", exact: true }).click();
  await expect(password).toHaveAttribute("type", "text");
  await page.getByRole("button", { name: "Hide password", exact: true }).click();
  await expect(password).toHaveAttribute("type", "password");
  await expect(password).toHaveValue("safe-example");

  const menu = page.getByRole("button", { name: "Project actions" });
  await menu.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Rename" })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: "Export" })).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("menuitem", { name: "Copy" })).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByRole("menuitem", { name: "Export" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toBeFocused();
  await expect(menu).toHaveAttribute("aria-expanded", "false");

  const trigger = page.getByRole("button", { name: "Edit profile", exact: true });
  await trigger.click();
  const dialog = page.getByRole("dialog");
  const name = dialog.getByLabel("Profile name");
  await expect(name).toBeFocused();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(name).toBeFocused();
  await name.fill("Team");
  const project = dialog.getByLabel("Project", { exact: true });
  await project.click();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await expect(project).toHaveAttribute("aria-expanded", "false");
  await project.click();
  await page.keyboard.press("ArrowDown");
  await expect(dialog.getByRole("menuitemcheckbox", { name: "Prometeu Cloud" })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(dialog.getByRole("menuitemcheckbox", { name: "Prometeu Desktop" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(project).toHaveText("Prometeu Desktop");
  await dialog.getByLabel("Simulate a save error").check();
  await dialog.getByRole("button", { name: "Save", exact: true }).focus();
  await page.keyboard.press("Tab");
  await expect(name).toBeFocused();
  await name.press("Enter");
  await expect(dialog.getByRole("button", { name: "Save", exact: true })).toBeDisabled();
  await expect(dialog.getByRole("alert")).toHaveText("Could not save. Try again.");
  await expect(name).toHaveValue("Team");
  await dialog.getByLabel("Simulate a save error").uncheck();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
  await page.setViewportSize({ width: 390, height: 640 });
  await trigger.click();
  expect(await dialog.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  expect(scripts.every(url => url.startsWith(baseURL))).toBe(true);
});

test("design system opens confirmations without highlighting actions and keeps keyboard focus", { tag: "@webkit" }, async ({ page }) => {
  await page.goto(`${baseURL}/index.html`);
  const trigger = page.getByRole("button", { name: "Delete account", exact: true });
  for (const keyboard of [false, true]) {
    if (keyboard) await trigger.press("Enter"); else await trigger.click();
    const dialog = page.getByRole("dialog", { name: "Delete account?", exact: true });
    await expect(dialog.locator(".sheettop b")).toBeFocused();
    await expect(dialog.locator("button:focus-visible")).toHaveCount(0);
    await page.keyboard.press("Shift+Tab");
    await expect(dialog.getByRole("button", { name: "Delete", exact: true })).toBeFocused();
    await page.keyboard.press("Tab");
    const cancel = dialog.getByRole("button", { name: "Cancel", exact: true });
    await expect(cancel).toBeFocused();
    await expect(cancel).toHaveCSS("outline-style", "solid");
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
  }
});

test("design system validates selection and preserves the submitter while blocking duplicate submissions", { tag: "@webkit" }, async ({ page }) => {
  await page.goto(`${baseURL}/index.html`);
  const result = await page.evaluate(async () => {
    const { enhance, field, select, button } = await import("/dist/index.js");
    const form = document.createElement("form"); form.dataset.uiForm = "true";
    const choice = select("", [["", "Choose"], ["cloud", "Cloud"]], { name: "project", required: true });
    const submit = button("Autorizar"); submit.type = "submit"; submit.name = "decision"; submit.value = "approve";
    form.append(field('<img src=x onerror="alert(1)">', choice.root), submit); document.body.append(form);
    const cleanup = enhance(form);
    const invalid = !form.reportValidity() && document.activeElement === choice.control;
    choice.value = "cloud";
    const valid = form.checkValidity();
    const first = new SubmitEvent("submit", { cancelable: true, bubbles: true, submitter: submit });
    const second = new SubmitEvent("submit", { cancelable: true, bubbles: true, submitter: submit });
    form.dispatchEvent(first); form.dispatchEvent(second);
    const data = new FormData(form, submit);
    const result = { invalid, valid, injected: form.querySelectorAll("img").length, blocked: second.defaultPrevented, busy: form.getAttribute("aria-busy"),
      decision: data.get("decision"), project: data.get("project") };
    window.dispatchEvent(new PageTransitionEvent("pageshow"));
    const reset = !form.hasAttribute("aria-busy") && !submit.hasAttribute("aria-disabled");
    cleanup(); form.remove(); return { ...result, reset };
  });
  expect(result).toEqual({ invalid: true, valid: true, injected: 0, blocked: true, busy: "true", decision: "approve", project: "cloud", reset: true });
});

// A browser is needed to resolve the legacy/shared CSS cascade; this compares the same compact
// primitive in both hosts without asserting unrelated page typography or repeating interaction tests.
test("desktop legacy styles preserve compact shared button spacing and color", async ({ page }) => {
  await page.goto(`${baseURL}/index.html`);
  await page.evaluate(() => document.body.classList.remove("ui-comfortable"));
  const styles = (node: HTMLElement) => {
    const style = getComputedStyle(node);
    return { padding: style.padding, color: style.color, minHeight: style.minHeight };
  };
  const standalone = await page.getByRole("button", { name: "Cancel", exact: true }).evaluate(styles);
  await page.goto("/design-system.html");
  expect(await page.getByRole("button", { name: "Cancel", exact: true }).evaluate(styles)).toEqual(standalone);
});
