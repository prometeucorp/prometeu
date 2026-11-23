import { describe, expect, it, vi, type Mock } from "vitest";

vi.mock("./ipc", () => ({ invoke: vi.fn() }));

import { invoke } from "./ipc";
// `vi.mocked(invoke)` makes the checker relate the whole IPC registry to the mock type, which costs minutes.
const invokeMock = invoke as unknown as Mock;
import { inheritedOf, loadInherited, onChange, toDraft, toServer, type Draft } from "./mcp";
import type { McpServer } from "./types";

/// Translate editable form fields into the provider configuration shape.
describe("form and registry", () => {
  it("parses an entered command into program and arguments", () => {
    const server = toServer({
      stdio: true,
      id: " echo ",
      cmd: "  npx -y @modelcontextprotocol/server-everything  ",
      url: "",
      pairs: [],
      note: " the test server ",
    });
    expect(server).toEqual({
      id: "echo",
      config: {
        type: "stdio",
        command: "npx",
        args: ["-y", "@modelcontextprotocol/server-everything"],
        env: {},
      },
      note: "the test server",
    });
  });

  /// The same key/value entry becomes a local environment variable or a remote header depending on transport.
  it("stores key-value pairs as stdio environment variables or remote headers", () => {
    const pairs: [string, string][] = [["X-Key", "abracadabra"]];
    const local = toServer({ stdio: true, id: "local", cmd: "node s.js", url: "", pairs, note: "" });
    expect(local!.config.env).toEqual({ "X-Key": "abracadabra" });
    const remote = toServer({ stdio: false, id: "remote", cmd: "", url: "https://x/mcp", pairs, note: "" });
    expect(remote!.config.headers).toEqual({ "X-Key": "abracadabra" });
    expect(remote!.config.url).toBe("https://x/mcp");
  });

  /// Ignore newly added blank rows instead of creating variables with empty names.
  it("excludes unnamed pairs from registration", () => {
    const server = toServer({
      stdio: true,
      id: "x",
      cmd: "node s.js",
      url: "",
      pairs: [
        ["", "leftover"],
        [" TOKEN ", " abc "],
      ],
      note: "",
    });
    expect(server!.config.env).toEqual({ TOKEN: "abc" });
  });

  it("requires a name and the address for the selected transport", () => {
    const base: Draft = { stdio: false, id: "", cmd: "node s.js", url: "https://x", pairs: [], note: "" };
    expect(toServer(base)).toBeNull();
    expect(toServer({ ...base, id: "x", url: "" })).toBeNull();
    // Ignore empty fields belonging to the other transport type.
    expect(toServer({ ...base, id: "x", cmd: "" })).not.toBeNull();
  });

  it("restores a registered server to the form as entered", () => {
    const server = {
      id: "capim-ds",
      config: { type: "stdio", command: "npx", args: ["-y", "@capim/ds-mcp"], env: { TOKEN: "abc" } },
      note: "design system",
    };
    const draft = toDraft(server);
    expect(draft.stdio).toBe(true);
    expect(draft.cmd).toBe("npx -y @capim/ds-mcp");
    expect(draft.pairs).toEqual([["TOKEN", "abc"]]);
    expect(toServer(draft)).toEqual(server);
  });

  it("restores remote URLs and headers to their original fields", () => {
    const draft = toDraft({
      id: "notion",
      config: { type: "http", url: "https://mcp.notion.com/mcp", headers: { "X-Id": "7" } },
      note: "",
    });
    expect(draft.stdio).toBe(false);
    expect(draft.url).toBe("https://mcp.notion.com/mcp");
    expect(draft.pairs).toEqual([["X-Id", "7"]]);
  });

  it("starts an empty form with a local program", () => {
    expect(toDraft(null)).toEqual({ stdio: true, id: "", cmd: "", url: "", pairs: [], note: "" });
  });
});

/// The CLI-inherited base (ADR 0046) arrives per workspace and repaints gated buttons.
describe("inherited CLI configuration", () => {
  const metabase: McpServer = { id: "metabase", config: { type: "http", url: "https://x/mcp" }, note: "" };

  it("fetches once per workspace and notifies when the result arrives", async () => {
    invokeMock.mockResolvedValue([metabase]);
    let painted = 0;
    const forget = onChange(() => painted++);
    expect(inheritedOf("ws-base")).toEqual([]);
    loadInherited("ws-base");
    loadInherited("ws-base");
    await vi.waitFor(() => expect(inheritedOf("ws-base")).toEqual([metabase]));
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("mcp_inherited", { id: "ws-base", agent: undefined });
    expect(painted).toBe(1);
    forget();
  });

  it("separates inherited configuration by provider in the same workspace", async () => {
    invokeMock.mockResolvedValueOnce([metabase]).mockResolvedValueOnce([]);
    loadInherited("mixed", "claude");
    loadInherited("mixed", "codex");
    await vi.waitFor(() => expect(inheritedOf("mixed", "claude")).toEqual([metabase]));
    expect(inheritedOf("mixed", "codex")).toEqual([]);
    expect(invokeMock).toHaveBeenCalledWith("mcp_inherited", { id: "mixed", agent: "codex" });
  });

  it("keeps the configuration empty and button hidden without a backend, then retries on the next paint", async () => {
    invokeMock.mockRejectedValue("mcp.inherited.failed");
    let painted = 0;
    const forget = onChange(() => painted++);
    loadInherited("ws-failure");
    // The announce still fires so gated buttons repaint with the empty base.
    await vi.waitFor(() => expect(painted).toBe(1));
    expect(inheritedOf("ws-failure")).toEqual([]);
    // The failure left no cache entry, so a later paint retries the discovery.
    invokeMock.mockResolvedValue([metabase]);
    loadInherited("ws-failure");
    await vi.waitFor(() => expect(inheritedOf("ws-failure")).toEqual([metabase]));
    forget();
  });
});
