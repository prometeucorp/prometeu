import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Item } from "./menu";
import type { EffectiveItem, McpServer, ProjectTools, Selection } from "./types";

// A separate mock avoids expanding invoke's generic command tuples in mockImplementation.
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), openAt: vi.fn(), selectAt: vi.fn() }));
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));
vi.mock("./menu", () => ({ openAt: mocks.openAt, selectAt: mocks.selectAt }));

import { t, use } from "./i18n";
import { load, openPicker } from "./mcp";
import { open, openFlat, type Pick, type Row } from "./tool-picker";

/// Stands in for the rendered panel: its scroll offset and click listeners are all the picker touches.
function panel() {
  const clicks: (() => void)[] = [];
  return {
    scrollTop: 0,
    addEventListener(type: string, fn: () => void) {
      if (type === "click") clicks.push(fn);
    },
    /// Deliver a click on a row as the menu does: the panel sees it first, then the row's handler
    /// removes the panel, which resets its offset, and runs the item.
    click(label: string) {
      for (const fn of clicks) fn();
      this.scrollTop = 0;
      shown(label).run!();
    },
  };
}

const noProject: ProjectTools = {
  repo: "/Users/me/app",
  file: null,
  hash: "",
  tools: { mcp: null, plugins: null, skills: null },
  pending: false,
  decision: null,
};

function backend(mcp: EffectiveItem[]) {
  mocks.invoke.mockImplementation(async (cmd: string) =>
    cmd === "workspace_tools" ? { mcp, plugins: [], skills: [] } : noProject,
  );
}

const hub = "Registered";
const cli = "Inherited from the CLI";

function pick(rows: Row[], current: Selection | null = null): Pick {
  return {
    workspace: "ws",
    agent: "claude",
    axis: "mcp",
    rows,
    current: () => current,
    set: vi.fn(),
    at: () => ({ x: 0, y: 0 }),
    noneLabel: "No MCP registered",
  };
}

/// A row of the menu the picker opened last, by label.
function shown(label: string) {
  const items = mocks.openAt.mock.lastCall![1] as Item[];
  const item = items.find((i) => i !== "sep" && i.label === label);
  if (!item || item === "sep") throw new Error(`no row labelled ${label}`);
  return item;
}

/// The item of the last opened menu that holds the keyboard selection, if the picker set one.
function selected() {
  const items = mocks.openAt.mock.lastCall![1] as Item[];
  const index = mocks.selectAt.mock.lastCall?.[0] as number | undefined;
  return index === undefined ? undefined : items[index];
}

beforeEach(() => {
  use("en");
  mocks.invoke.mockReset();
  mocks.openAt.mockReset().mockImplementation(() => panel());
  mocks.selectAt.mockReset();
});

describe("provenance badges", () => {
  it("leave out the provenance a section header already states", async () => {
    backend([
      { id: "notion", provenance: "inherited" },
      { id: "metabase", provenance: "cli" },
      { id: "claude.ai Linear", provenance: "removed" },
    ]);
    await open(
      pick([
        { id: "notion", label: "notion", section: hub },
        { id: "metabase", label: "metabase", section: cli, implied: "cli" },
        { id: "claude.ai Linear", label: "claude.ai Linear", section: cli, implied: "cli" },
      ]),
    );
    expect(shown(cli).disabled).toBe(true);
    expect(shown("metabase").badge).toBeUndefined();
    expect(shown("metabase").checked).toBe(true);
    // A removal is the exception the header does not state, so it keeps its badge.
    expect(shown("claude.ai Linear").badge).toBe(t("tools.prov.removed"));
    expect(shown("notion").badge).toBe(t("tools.prov.inherited"));
  });

  it("keep the provenance on each row when the list has no headers to state it", async () => {
    backend([{ id: "metabase", provenance: "cli" }]);
    await open(pick([{ id: "metabase", label: "metabase", section: cli, implied: "cli" }]));
    expect(() => shown(cli)).toThrow();
    expect(shown("metabase").badge).toBe(t("tools.prov.cli"));
  });
});

describe("MCP picker", () => {
  it("lists the CLI base under its header and badges only what this workspace changed there", async () => {
    const notion: McpServer = { id: "notion", config: { type: "http", url: "https://mcp.notion.com/mcp" }, note: "" };
    const metabase: McpServer = { id: "metabase", config: { type: "http", url: "https://metabase.example/mcp" }, note: "capim-backend" };
    const linear: McpServer = {
      id: "claude.ai Linear",
      config: { type: "claudeai-proxy", url: "https://mcp.linear.app/mcp", id: "mcpsrv_1" },
      note: "claude.ai",
    };
    mocks.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "mcp_hub") return [notion];
      if (cmd === "mcp_logins") return [];
      if (cmd === "mcp_inherited") return [metabase, linear];
      if (cmd === "workspace_tools")
        return {
          mcp: [
            { id: "metabase", provenance: "cli" },
            { id: "claude.ai Linear", provenance: "removed" },
          ],
          plugins: [],
          skills: [],
        };
      return noProject;
    });
    await load();
    await openPicker({
      workspace: "ws",
      agent: "claude",
      current: () => ({ base: "inherit", add: [], remove: ["claude.ai Linear"] }),
      set: vi.fn(),
      at: () => ({ x: 0, y: 0 }),
    });
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalled());
    expect(shown(t("tools.section.cli")).disabled).toBe(true);
    expect(shown("metabase").badge).toBeUndefined();
    expect(shown("claude.ai Linear").badge).toBe(t("tools.prov.removed"));
  });
});

describe("reopening after a choice", () => {
  const rows: Row[] = [
    { id: "notion", label: "notion", section: hub },
    { id: "metabase", label: "metabase", section: cli, implied: "cli" },
  ];

  it.each(["metabase", "Select none", "Inherit defaults"])(
    "keeps the list where the person scrolled after %s",
    async (label) => {
      backend([{ id: "metabase", provenance: "cli" }]);
      const first = panel();
      const second = panel();
      mocks.openAt.mockReturnValueOnce(first).mockReturnValueOnce(second);
      await open(pick(rows, { base: "inherit", add: [], remove: [] }));
      expect(first.scrollTop).toBe(0);
      first.scrollTop = 240;
      first.click(label);
      await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(2));
      expect(second.scrollTop).toBe(240);
    },
  );

  it.each(["metabase", "Select none", "Inherit defaults"])(
    "puts the keyboard selection back on %s after choosing it",
    async (label) => {
      backend([{ id: "metabase", provenance: "cli" }]);
      const first = panel();
      mocks.openAt.mockReturnValueOnce(first);
      await open(pick(rows, { base: "inherit", add: [], remove: [] }));
      expect(mocks.selectAt).not.toHaveBeenCalled();
      first.click(label);
      await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(2));
      expect(selected()).toMatchObject({ label });
    },
  );

  it("keeps Select none selected when a CLI server has the same name", async () => {
    const label = t("tools.selectNone");
    backend([{ id: label, provenance: "cli" }]);
    let current: Selection | null = null;
    const p = pick([{ id: label, label, section: cli, implied: "cli" }]);
    p.current = () => current;
    p.set = vi.fn((sel) => { current = sel; });
    await open(p);
    const items = mocks.openAt.mock.lastCall![1] as Item[];
    const footer = items.slice().reverse().find((i) => i !== "sep" && i.label === label)!;
    if (footer === "sep") throw new Error("expected footer action");
    footer.run!();
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(2));
    const restored = selected();
    expect(restored).toMatchObject({ label });
    if (!restored || restored === "sep") throw new Error("expected selected action");
    restored.run!(); // Enter on the reopened menu must repeat the footer action.
    expect(p.set).toHaveBeenLastCalledWith({ base: "none", add: [], remove: [] });
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(3));
  });

  it.each(["workspace", "flat"])("restores duplicate row labels by identity in the %s picker", async (kind) => {
    backend([]);
    const p = pick([
      { id: "first", label: "Shared label" },
      { id: "second", label: "Shared label" },
    ]);
    let current: Selection | null = null;
    p.current = () => current;
    p.set = vi.fn((sel) => {
      current = sel;
      // Identity must survive changes in menu order too.
      p.rows.unshift({ id: "new", label: "Shared label" });
    });
    if (kind === "workspace") await open(p);
    else openFlat({ ...p, at: () => ({ x: 0, y: 0 }) });
    const items = mocks.openAt.mock.lastCall![1] as Item[];
    const row = items.filter((i) => i !== "sep" && i.label === "Shared label")[1];
    if (row === "sep") throw new Error("expected tool row");
    row.run!();
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(2));
    const restored = selected();
    expect(restored).toMatchObject({ label: "Shared label", checked: true });
    if (!restored || restored === "sep") throw new Error("expected selected row");
    restored.run!();
    expect(p.set).toHaveBeenLastCalledWith({ base: "inherit", add: [], remove: ["second"] });
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(3));
  });

  it.each(["workspace", "flat"])("does not select a namesake when reset disappears in the %s picker", async (kind) => {
    backend([]);
    const label = t("tools.reset");
    const p = pick([{ id: label, label }]);
    let current: Selection | null = { base: "none", add: [], remove: [] };
    p.current = () => current;
    p.set = vi.fn((sel) => { current = sel; });
    if (kind === "workspace") await open(p);
    else openFlat({ ...p, at: () => ({ x: 0, y: 0 }) });
    const items = mocks.openAt.mock.lastCall![1] as Item[];
    const reset = items.slice().reverse().find((i) => i !== "sep" && i.label === label)!;
    if (reset === "sep") throw new Error("expected reset action");
    reset.run!();
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(2));
    expect(selected()).toBeUndefined();
  });

  it("puts the keyboard selection back on the flat row just chosen", async () => {
    const first = panel();
    mocks.openAt.mockReturnValueOnce(first);
    openFlat({ rows, current: () => null, set: vi.fn(), at: () => ({ x: 0, y: 0 }), noneLabel: "No MCP registered" });
    first.click("metabase");
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(2));
    expect(selected()).toMatchObject({ label: "metabase" });
  });

  it("keeps the flat list where the person scrolled after a choice", async () => {
    const first = panel();
    const second = panel();
    mocks.openAt.mockReturnValueOnce(first).mockReturnValueOnce(second);
    openFlat({ rows, current: () => null, set: vi.fn(), at: () => ({ x: 0, y: 0 }), noneLabel: "No MCP registered" });
    first.scrollTop = 180;
    first.click("metabase");
    await vi.waitFor(() => expect(mocks.openAt).toHaveBeenCalledTimes(2));
    expect(second.scrollTop).toBe(180);
  });
});
