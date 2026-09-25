import { expect, test, type Locator, type Page } from "@playwright/test";
import type { McpServer } from "../src/types";

type McpWindow = Window & {
  __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
  mcpCalls: string[];
  releaseMcpLogin?: () => void;
  mock: { catalog: (servers: McpServer[]) => void };
};

async function action(page: Page, row: Locator, name: string) {
  await row.getByRole("button", { name: /^More options · / }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}

async function openTools(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem("mock:cloud", JSON.stringify({ user: { id: "1", name: "Person", email: "me@example.com" }, origin: "https://app.prometeu.co", offline: false }));
    localStorage.setItem("mock:cloudOffline", "1");
  });
  await page.goto("/");
  await expect(page.locator("#tiles .tile").first()).toBeVisible();
  await page.evaluate(() => {
    const w = window as McpWindow;
    const invoke = w.__TAURI_INTERNALS__.invoke;
    w.mcpCalls = [];
    w.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command.startsWith("mcp_")) w.mcpCalls.push(command);
      if (command === "mcp_login") {
        if (localStorage.getItem("test:denyMcpLogin")) throw 'i18n:{"code":"err.mcp.auth.denied"}';
        if (localStorage.getItem("test:holdMcpLogin")) await new Promise<void>((resolve) => { w.releaseMcpLogin = resolve; });
      }
      if (command === "mcp_check" && localStorage.getItem("test:failMcpCheck")) return {
        steps: [{ key: "connect", ok: false, note: "", detail: "connection refused" }],
        probe: { ok: false, auth: false, tools: 0, name: "", detail: "connection refused" },
      };
      return invoke(command, args);
    };
  });
  await page.locator("#settings").click();
  await page.locator(".setnavitem", { hasText: "Resources" }).click();
  await page.locator('.resource-filters [data-filter="mcp"]').click();
}

test("tools: Cloud MCP checks and authenticates locally with recoverable failures and no copy", async ({ page }) => {
  await openTools(page);
  const row = page.locator(".mcp-server", { has: page.locator("b", { hasText: /^notion$/ }) });
  const catalogBefore = await page.evaluate(() => localStorage.getItem("mock:catalog"));
  await expect(row).toContainText("Personal");
  await expect(row.getByRole("status")).toHaveText("Not checked on this Mac");
  await action(page, row, "Test connection");
  await expect(row.getByRole("status")).toHaveText("Authentication required on this Mac");

  await page.evaluate(() => localStorage.setItem("test:denyMcpLogin", "1"));
  await action(page, row, "Authenticate");
  await expect(row.getByRole("status")).toContainText("Connection error");
  await row.getByRole("button", { name: "More options · notion", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Authenticate", exact: true })).toBeEnabled();
  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    localStorage.removeItem("test:denyMcpLogin");
    localStorage.setItem("test:holdMcpLogin", "1");
  });
  await action(page, row, "Authenticate");
  await expect(row).toHaveAttribute("aria-busy", "true");
  await expect(row.getByRole("button", { name: "More options · notion", exact: true })).toBeDisabled();
  // A settings redraw must not allow a second login while browser consent is pending.
  await page.locator(".setnavitem", { hasText: "General" }).click();
  await page.locator(".setnavitem", { hasText: "Resources" }).click();
  await page.locator('.resource-filters [data-filter="mcp"]').click();
  await expect(row.getByRole("button", { name: "More options · notion", exact: true })).toBeDisabled();
  await page.evaluate(() => (window as McpWindow).releaseMcpLogin!());
  await expect(row.getByRole("status")).toHaveText("signed in");

  // A saved token does not hide a failed connection check.
  await page.evaluate(() => localStorage.setItem("test:failMcpCheck", "1"));
  await action(page, row, "Test connection");
  await expect(row.getByRole("status")).toHaveText("Connection error · connection refused");
  await page.evaluate(() => localStorage.removeItem("test:failMcpCheck"));
  await action(page, row, "Test connection");
  await expect(row.getByRole("status")).toHaveText("signed in");
  await action(page, row, "Sign out");
  await expect(row.getByRole("status")).toHaveText("Authentication required on this Mac");
  expect(await page.evaluate(() => localStorage.getItem("mock:catalog"))).toBe(catalogBefore);
  expect(await page.evaluate(() => (window as McpWindow).mcpCalls)).not.toContain("mcp_save");
  await expect(page.locator(".mcp-server", { has: page.locator("b", { hasText: /^notion$/ }) })).toHaveCount(1);

  const local = page.locator(".mcp-server", { has: page.locator("b", { hasText: /^capim-ds$/ }) });
  await local.getByRole("button", { name: "More options · capim-ds", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Authenticate", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await action(page, local, "Test connection");
  await expect(local.getByRole("status")).toHaveText("signed in");
  await expect(page.locator(".mcp-server", { has: page.locator("b", { hasText: /^prometeu$/ }) }).getByRole("button")).toHaveCount(0);
  await page.setViewportSize({ width: 900, height: 800 });
  await row.getByRole("button", { name: "More options · notion", exact: true }).click();
  const bounds = await page.getByRole("menuitem", { name: "Delete from cloud", exact: true }).boundingBox();
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(900);
  await page.keyboard.press("Escape");
});

test("tools: editor authentication neither saves the catalog nor publishes the draft", async ({ page }) => {
  await openTools(page);
  const row = page.locator(".mcp-server", { has: page.locator("b", { hasText: /^notion$/ }) });
  await action(page, row, "Edit");
  // The catalog revision can change after opening the editor without blocking local OAuth.
  const changed = JSON.stringify({ connected: true, revision: 7, plugins: [], mcp: ["notion"], skills: [], shared: { "mcp:notion": "notion" } });
  await page.evaluate((value) => localStorage.setItem("mock:catalog", value), changed);
  await page.locator("#veil").getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.locator("#veil").getByRole("button", { name: "Sign out", exact: true })).toBeVisible();
  await page.locator("#veil").getByRole("button", { name: "Cancel", exact: true }).click();
  // A saved login without a cached check offers sign-out, not another browser OAuth flow.
  await expect(row.getByRole("status")).toHaveText("Not checked on this Mac");
  await row.getByRole("button", { name: "More options · notion", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Authenticate", exact: true })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Sign out", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await action(page, row, "Edit");
  await page.locator("#veil").getByRole("button", { name: "Sign out", exact: true }).click();
  await page.locator("#veil .mhead").click();
  await page.locator("#veil").getByLabel("Remote MCP server URL", { exact: true }).fill("https://mcp.capim.test/mcp");
  await page.locator("#veil").getByLabel("Remote MCP server URL", { exact: true }).press("Tab");
  await expect(page.locator("#veil .mcheck")).toContainText("Server checked");
  await page.locator("#veil").getByRole("button", { name: "Continue", exact: true }).click();
  await page.locator("#veil").getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.locator("#veil .hint")).toHaveText("Save connection changes before authenticating.");
  const calls = await page.evaluate(() => (window as McpWindow).mcpCalls);
  expect(calls.filter((command) => command === "mcp_login")).toHaveLength(1);
  expect(calls).not.toContain("mcp_save");
  expect(await page.evaluate(() => localStorage.getItem("mock:catalog"))).toBe(changed);
});

test("tools: refresh preserves the pending login lock and discards stale results", async ({ page }) => {
  await openTools(page);
  const row = page.locator(".mcp-server", { has: page.locator("b", { hasText: /^notion$/ }) });
  await page.evaluate(() => localStorage.setItem("test:holdMcpLogin", "1"));
  await action(page, row, "Authenticate");
  await expect(row).toHaveAttribute("aria-busy", "true");
  await page.evaluate(async () => {
    const w = window as McpWindow;
    const servers = await w.__TAURI_INTERNALS__.invoke("mcp_hub") as McpServer[];
    w.mock.catalog(servers.map((server) => server.id === "notion"
      ? { ...server, config: { ...server.config, url: "https://broken.test/mcp" } } : server));
  });
  await expect(row).toContainText("https://broken.test/mcp");
  await expect(row).toHaveAttribute("aria-busy", "true");
  await expect(row.getByRole("button", { name: "More options · notion", exact: true })).toBeDisabled();
  await page.evaluate(() => (window as McpWindow).releaseMcpLogin!());
  await expect(row).toHaveAttribute("aria-busy", "false");
  await expect(row.getByRole("status")).toHaveText("Not checked on this Mac");
  await action(page, row, "Test connection");
  await expect(row.getByRole("status")).toHaveText("Connection error · connection refused");
  const calls = await page.evaluate(() => (window as McpWindow).mcpCalls);
  expect(calls.filter((command) => command === "mcp_login")).toHaveLength(1);
  expect(calls.filter((command) => command === "mcp_check")).toHaveLength(2);
});
