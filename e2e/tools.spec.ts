import { expect, test, type Page } from "@playwright/test";
import type { Board, ProjectTools, Selection, WorkspaceTools } from "../src/types";

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
declare global {
  interface Window { toolTestInvoke?: Invoke }
}

async function boot(page: Page) {
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.evaluate(() => {
    window.toolTestInvoke = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__.invoke;
  });
}

async function openWorkspace(page: Page, id: string) {
  await page.locator(`.railworkspace[data-workspace="${id}"] .navitem.sub`).click();
  await expect(page.locator("#wsView")).toBeVisible();
}

const row = (page: Page, text: string) => page.locator(".menu .mrow").filter({ hasText: text }).first();

test("tools: project trust accepts an empty replacement", async ({ page }) => {
  await boot(page);
  await page.evaluate(async () => {
    const invoke = window.toolTestInvoke!;
    // An empty declaration still needs consent even though no item can appear pending.
    const project = await invoke("project_tools", { id: "ui-2231" }) as ProjectTools;
    project.tools.plugins = null;
    project.tools.mcp = { base: "none", add: [], remove: [] } satisfies Selection;
    await invoke("set_tools_global", { mcp: { base: "inherit", add: ["capim-ds"], remove: [] } });
  });
  await openWorkspace(page, "ui-2231");
  await page.locator("#chatwrap .mcpbtn").click();
  await expect(row(page, "Trust this project's tools")).toBeVisible();
  await row(page, "Trust this project's tools").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("replaces all");
  await dialog.getByRole("button", { name: "Trust and enable" }).click();
  await expect(dialog).toHaveCount(0);
  const project = await page.evaluate(() => window.toolTestInvoke!("project_tools", { id: "ui-2231" })) as ProjectTools;
  expect(project.pending).toBe(false);
  expect(project.decision?.approved).toBe(true);
  // The workspace menu keeps the declaration accessible after approval, without pending rows.
  await page.locator('.railworkspace[data-workspace="ui-2231"] .navitem.sub').click({ button: "right" });
  await row(page, "Project tools").click();
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("tools: an old dialog cannot approve a new declaration", async ({ page }) => {
  await boot(page);
  await page.evaluate(() => {
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    let first = true;
    internals.invoke = async (command, args) => {
      const result = await original(command, args);
      if (command === "project_tools" && first) {
        first = false;
        return { ...(result as ProjectTools), hash: "previously-displayed-version" };
      }
      return result;
    };
  });
  const open = async () => {
    await page.locator('.railworkspace[data-workspace="ui-2231"] .navitem.sub').click({ button: "right" });
    await row(page, "Project tools").click();
  };
  await open();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "Trust and enable" }).click();
  await expect(dialog).toContainText("The declaration changed");
  expect(await page.evaluate(async () => (await window.toolTestInvoke!("load_board") as Board).tool_trust ?? [])).toEqual([]);
  await dialog.getByRole("button", { name: "Not now" }).click();
  await open();
  await dialog.getByRole("button", { name: "Trust and enable" }).click();
  await expect(dialog).toHaveCount(0);
});

/// Account connectors (ADR 0063) can put dozens of servers in a Claude workspace's CLI base; forty
/// stand in for them ahead of the mock's own base.
async function longBase(page: Page) {
  await page.evaluate(() => {
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: Invoke } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    const ids = Array.from({ length: 40 }, (_, i) => `claude.ai Connector ${i + 1}`);
    internals.invoke = async (command, args) => {
      const result = await original(command, args);
      if (command === "mcp_inherited" && (result as unknown[]).length) {
        const connectors = ids.map((id) => ({ id, config: { type: "claudeai-proxy", url: "https://mcp.example.com/mcp", id }, note: "claude.ai" }));
        return [...connectors, ...(result as unknown[])];
      }
      if (command === "workspace_tools") {
        const tools = result as WorkspaceTools;
        return { ...tools, mcp: [...ids.map((id) => ({ id, provenance: "cli" })), ...tools.mcp] };
      }
      return result;
    };
  });
}

test("tools: the CLI base follows the tab’s provider, scrolls when long and built-in MCP requires opt-in", async ({ page }) => {
  await boot(page);
  await page.evaluate(async () => {
    const invoke = window.toolTestInvoke!;
    const board = await invoke("load_board") as Board;
    board.workspaces[0].tabs[1].choice = { agent: "codex", model: "gpt-5.6-sol", effort: "high" };
    await invoke("set_stage", { id: "sessao-0929", stage: "Fazendo" });
  });
  await longBase(page);
  await openWorkspace(page, "sessao-0929");
  const button = page.locator("#chatwrap .mcpbtn");
  await button.click();
  await expect(row(page, "metabase")).toBeVisible();
  await expect(row(page, "applies when starting a conversation")).toBeVisible();
  // The long list stays inside the window, above the composer control that opened it.
  const anchor = (await button.boundingBox())!;
  const panel = (await page.locator(".menu.tools").boundingBox())!;
  expect(panel.y).toBeGreaterThanOrEqual(0);
  expect(panel.y + panel.height).toBeLessThanOrEqual(anchor.y);
  await expect(row(page, "prometeu")).toHaveAttribute("aria-checked", "false");
  await row(page, "prometeu").click();
  await expect(row(page, "prometeu")).toHaveAttribute("aria-checked", "true");
  // The last rows are reachable by scrolling the panel, and a choice reopens it where it was.
  await row(page, "claude.ai Linear").click();
  await expect(row(page, "claude.ai Linear")).toHaveAttribute("aria-checked", "false");
  await expect(row(page, "claude.ai Linear")).toBeInViewport();
  await page.keyboard.press("Escape");
  await page.locator('#tabbar .tab[data-tab="t2"]').click();
  await button.click();
  await expect(page.locator(".menu")).toBeVisible();
  await expect(row(page, "metabase")).toHaveCount(0);
  await expect(row(page, "n8n")).toHaveCount(0);
  await row(page, "Select none").click();
  await expect(row(page, "Inherit defaults")).toBeVisible();
  await row(page, "Inherit defaults").click();
  await expect.poll(() => page.evaluate(async () => (await window.toolTestInvoke!("load_board") as Board).workspaces[0].mcp)).toBeNull();
});
