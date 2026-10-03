import { test, expect } from "@playwright/test";

// This isolated shell has its own composition: browser keyboard submission and DOM replacement
// must retain the draft and history through stop/resume. Rust tests cover real process transport.
test("WSL preview composes conversation controls and preserves history on resume", async ({ page }) => {
  await page.goto("/wsl-preview.html");
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await page.getByRole("button", { name: "Start / resume" }).click();
  const draft = page.getByRole("textbox", { name: "Message", exact: true });
  await expect(draft).toBeEnabled();
  await draft.fill("First conversation message");
  await draft.press("Enter");
  await expect(page.locator(".wsl-transcript .me")).toHaveText("First conversation message");
  await expect(page.locator(".wsl-transcript .bot")).toContainText("First conversation message");
  await expect(draft).toHaveValue("");
  await draft.fill("Keep this unsent draft");
  await page.getByRole("button", { name: "Open terminal", exact: true }).click();
  const shell = page.locator(".wsl-terminal .xterm-helper-textarea");
  await shell.pressSequentially("terminal input");
  await expect(page.locator(".wsl-terminal .xterm-rows")).toContainText("terminal input");
  // xterm's hidden textarea must receive keys without submitting or clearing the chat draft.
  await expect(draft).toHaveValue("Keep this unsent draft");
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(draft).toBeEnabled();
  await expect(page.getByRole("button", { name: "Start / resume" })).toBeDisabled();
  await expect(page.locator(".wsl-terminal .xterm-rows")).toContainText("terminal input");
  await expect(draft).toHaveValue("Keep this unsent draft");
  await page.getByRole("button", { name: "Close terminal", exact: true }).click();
  await expect(draft).toBeEnabled();
  await expect(page.getByRole("button", { name: "Open terminal", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await expect(draft).toBeDisabled();
  await page.getByRole("button", { name: "Start / resume" }).click();
  await expect(draft).toHaveValue("Keep this unsent draft");
  await expect(page.locator(".wsl-transcript .me")).toHaveCount(1);
  await page.getByRole("button", { name: "Add workspace", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Add workspace" });
  await dialog.getByRole("textbox", { name: "Workspace name" }).fill("Second workspace");
  await dialog.getByRole("textbox", { name: "Project directory (Linux path)" }).fill("/second/project");
  await dialog.getByRole("button", { name: "Create workspace", exact: true }).click();
  const second = page.getByRole("button", { name: "Second workspace", exact: true });
  await second.focus(); await second.press("Enter");
  await expect(second).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".wsl-transcript .me")).toHaveCount(0);
  await expect(draft).toHaveValue("");
  await page.getByRole("button", { name: "Start / resume" }).click();
  await draft.fill("Second workspace draft");
  await page.locator('[data-workspace="primary"]').click();
  await expect(draft).toHaveValue("Keep this unsent draft");
  await expect(page.locator(".wsl-transcript .me")).toHaveText("First conversation message");
  await expect(page.locator('[data-workspace="primary"]')).toBeFocused();
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await expect(page.getByRole("button", { name: "Connect", exact: true })).toBeVisible();
  await expect(draft).toHaveValue("Keep this unsent draft");
});
