import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TerminalSession } from "./types";

const invoke = vi.fn();
vi.mock("./ipc", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
import * as terminals from "./terminals";

const home: TerminalSession = { id: "terminal-home", folder: "/Users/me" };
const named: TerminalSession = { id: "terminal-named", folder: "/Users/me/code", name: "server" };

describe("terminal session names", () => {
  it("prefer the person's name, then the folder's", () => {
    const root: TerminalSession = { id: "terminal-root", folder: "/" };
    expect([home, named, root].map(terminals.title)).toEqual(["me", "server", "/"]);
  });
});

describe("terminal sessions", () => {
  let current: string | null;
  let shown: TerminalSession[];
  let events: string[];
  beforeEach(() => {
    invoke.mockReset();
    current = null;
    shown = [];
    events = [];
    terminals.init({
      sessions: () => [home, named],
      current: () => current,
      show: (session) => shown.push(session),
      left: () => events.push("left"),
      changed: () => events.push("changed"),
      say: (message) => events.push(`said ${message}`),
      pick: async () => named.folder,
    });
  });

  it("end the session whose own shell exits, leaving its view first", async () => {
    current = named.id;
    terminals.closed("workspace:terminal");
    terminals.closed(`${home.id}:terminal-2`);
    terminals.closed("/Users/me:terminal");
    expect(invoke).not.toHaveBeenCalled();

    invoke.mockResolvedValue(undefined);
    terminals.closed(terminals.keyOf(named.id));
    await vi.waitFor(() => expect(events).toEqual(["left", "changed"]));
    expect(invoke.mock.calls).toEqual([["terminal_close", { id: named.id }]]);
    expect(terminals.isLive(named.id)).toBe(false);
  });

  it("start in the home folder or a chosen one", async () => {
    invoke.mockResolvedValueOnce(home).mockResolvedValueOnce(named);
    await terminals.create();
    await terminals.createIn();
    expect(invoke.mock.calls).toEqual([
      ["terminal_create", { folder: null }],
      ["terminal_create", { folder: named.folder }],
    ]);
    expect(shown).toEqual([home, named]);
  });

  it("report a folder that no longer exists without showing a session", async () => {
    invoke.mockRejectedValueOnce('i18n:{"code":"err.dock.noFolder","args":{"path":"/gone"}}');
    await terminals.create("/gone");
    expect(shown).toEqual([]);
    expect(events).toHaveLength(1);
    expect(events[0]).toMatch(/^said .*\/gone/);
  });

  it("ask the backend which sessions run a shell, after a reload or an attachment", async () => {
    let running = new Set([named.id]);
    invoke.mockImplementation(async (_command: string, args: { id: string }) => [{ kind: "terminal", alive: running.has(args.id) }]);
    await terminals.sync();
    expect([home, named].map((session) => terminals.isLive(session.id))).toEqual([false, true]);
    // An attachment that a newer view replaced ends quietly even when its shell failed to start.
    await terminals.check(home.id);
    expect(terminals.isLive(home.id)).toBe(false);
    running = new Set([home.id]);
    await terminals.check(home.id);
    await terminals.check(named.id);
    expect([home, named].map((session) => terminals.isLive(session.id))).toEqual([true, false]);
    invoke.mockRejectedValueOnce("gone");
    await terminals.check(home.id);
    expect(terminals.isLive(home.id)).toBe(false);
    expect(events).toEqual(["changed", "changed", "changed", "changed"]);
  });

  it("close the session on screen with Command-W only while its terminal has focus", async () => {
    invoke.mockResolvedValue(undefined);
    expect(terminals.closeFocused(true)).toBe(false);
    current = home.id;
    expect(terminals.closeFocused(false)).toBe(false);
    expect(terminals.closeFocused(true)).toBe(true);
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("terminal_close", { id: home.id }));
  });
});
