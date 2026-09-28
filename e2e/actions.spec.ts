import { expect, test, type Page } from "@playwright/test";
import type { Board } from "../src/types";

async function settings(page: Page) {
  await page.locator("#settings").click();
  await page.locator(".setnavitem").getByText("Actions", { exact: true }).click();
}
async function workspace(page: Page) {
  await page.locator("#railbody .navitem.sub .lbl").getByText("Hello", { exact: true }).click();
  await expect(page.locator("#wsView")).toBeVisible();
}

test("reusable commands fill an editable prompt and persist after reopening", async ({ page }) => {
  await page.goto("/");
  // Initial navigation must finish before Settings can remain the active screen.
  await expect(page.locator("#deskView")).toBeVisible();
  await settings(page);
  await page.getByRole("button", { name: "New command", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Configure command" });
  await dialog.getByLabel("Command (without /)", { exact: true }).fill("explain");
  await dialog.getByLabel("Prompt text or initial task request").fill("Explain the changes with examples.");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog).toBeHidden();
  await page.reload();
  await workspace(page);
  const area = page.locator("#chatwrap .composer textarea");
  const count = await page.locator("#chatwrap .turn.user").count();
  await area.fill("/");
  const suggestions = page.locator(".menu.cmds");
  await expect(suggestions.locator(".mrow", { hasText: "/explain" }).locator(".mbadge")).toHaveText("Prometeu");
  await expect(suggestions.locator(".mrow", { hasText: "/review" }).locator(".mbadge")).toHaveText("Prometeu");
  await expect(suggestions.locator(".mrow", { hasText: "/compact" })).toBeVisible();
  await expect(suggestions.locator(".mrow", { hasText: "/compact" }).locator(".mbadge")).toHaveCount(0);
  await area.fill("/explain file.ts");
  await area.press("Enter");
  await expect(area).toHaveValue("Explain the changes with examples.\n\nfile.ts");
  await expect(page.locator("#chatwrap .turn.user")).toHaveCount(count);
  await area.fill("other context");
  await page.locator("#chatwrap .actionsbtn").click();
  await page.locator(".menu .mrow", { hasText: "/explain" }).click();
  await expect(area).toHaveValue("Explain the changes with examples.\n\nother context");
});


test("model picker: editing instructions preserves a profile’s historical model and effort", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.evaluate(async () => {
    type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const board = await invoke("load_board") as Board;
    const profile = board.actions.profiles[0];
    profile.provider_rule = "fixed"; profile.candidates = [];
    profile.choice = { agent: "codex", model: "retired-model", effort: "retired-effort" };
    await invoke("actions_save", { catalog: board.actions });
  });
  await settings(page);
  await page.locator(".action-profile").first().getByRole("button", { name: "Edit", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Configure agent" });
  await expect(dialog.getByLabel("Model", { exact: true })).toContainText("retired-model");
  await dialog.getByLabel("Agent prompt and instructions", { exact: true }).fill("Preserve my previous selection.");
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(dialog).toBeHidden();
  const choice = await page.evaluate(async () => {
    type Invoke = (command: string) => Promise<Board>;
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    return (await invoke("load_board")).actions.profiles[0].choice;
  });
  expect(choice).toEqual({ agent: "codex", model: "retired-model", effort: "retired-effort" });
});
