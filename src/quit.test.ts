import { beforeEach, describe, expect, it, vi } from "vitest";
import { use } from "./i18n";
import type { Board, Tab, Workspace } from "./types";

const host = vi.hoisted(() => ({
  invoke: vi.fn(),
  confirm: vi.fn(),
  exit: vi.fn(),
  unminimize: vi.fn(async () => {}),
  setFocus: vi.fn(async () => {}),
  close: null as null | ((event: { preventDefault: () => void }) => Promise<void>),
}));
vi.mock("./ipc", () => ({ invoke: (...args: unknown[]) => host.invoke(...args) }));
vi.mock("./ui", () => ({ confirmDialog: (...args: unknown[]) => host.confirm(...args) }));
vi.mock("@tauri-apps/plugin-process", () => ({ exit: (...args: unknown[]) => host.exit(...args) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    unminimize: host.unminimize,
    setFocus: host.setFocus,
    onCloseRequested: async (handler: NonNullable<typeof host.close>) => {
      host.close = handler;
      return () => {};
    },
  }),
}));

import { allow, init, pending, request, summarize } from "./quit";

// Assertions read the joined English sentence; translations are not under test.
use("en");

const tab = (status: Tab["status"]) => ({ id: status, status }) as Tab;
const workspace = (tabs: Tab[], extra: Partial<Workspace> = {}) => ({ id: "w", tabs, preparing: false, ...extra }) as Workspace;
const board = (...workspaces: Workspace[]): Board => ({ stages: [], projects: [], workspaces });

describe("pending", () => {
  it("counts the conversations a restart would mark interrupted", () => {
    const work = pending(board(workspace([tab("rodando"), tab("querendo"), tab("pronta"), tab("desligada")])), 0);
    expect(work).toEqual({ conversations: 2, preparing: 0, terminals: 0 });
  });

  it("counts workspaces still being prepared and leaves other people's shares out", () => {
    const shared = workspace([tab("rodando")], { remote: { owner: "marcus", online: true } });
    const work = pending(board(workspace([], { preparing: true }), shared), 3);
    expect(work).toEqual({ conversations: 0, preparing: 1, terminals: 3 });
  });
});

describe("summarize", () => {
  it("names nothing when no work would be cut off", () => {
    expect(summarize({ conversations: 0, preparing: 0, terminals: 0 })).toBe("");
  });

  it("joins each kind of work with its count", () => {
    expect(summarize({ conversations: 1, preparing: 0, terminals: 0 })).toBe("1 conversation in progress");
    expect(summarize({ conversations: 0, preparing: 0, terminals: 1 })).toBe("1 terminal running a command");
    expect(summarize({ conversations: 2, preparing: 1, terminals: 3 })).toBe(
      "2 conversations in progress, 1 workspace being prepared and 3 terminals running commands",
    );
  });
});

describe("asking before the app ends", () => {
  let current: Board;
  beforeEach(() => {
    vi.clearAllMocks();
    current = board(workspace([tab("pronta")]));
    host.invoke.mockResolvedValue(0);
    init({ board: () => current });
  });

  it("ends at once when nothing local would be cut off", async () => {
    expect(await allow({ kind: "quit" })).toBe(true);
    expect(host.invoke).toHaveBeenCalledWith("running_terminals");
    expect(host.confirm).not.toHaveBeenCalled();
  });

  it("brings the window forward, names what would stop and follows the answer", async () => {
    current = board(workspace([tab("rodando")]));
    host.invoke.mockResolvedValue(1);
    host.confirm.mockResolvedValueOnce(false).mockResolvedValueOnce(true);

    expect(await allow({ kind: "quit" })).toBe(false);
    expect(host.unminimize).toHaveBeenCalled();
    expect(host.setFocus).toHaveBeenCalled();
    expect(host.confirm).toHaveBeenLastCalledWith(expect.objectContaining({
      title: "Quit Prometeu?",
      message: "Quitting now stops 1 conversation in progress and 1 terminal running a command.",
      accept: "Quit",
    }));
    expect(await allow({ kind: "quit" })).toBe(true);
  });

  it("tells a restart that the installed version also applies at the next start", async () => {
    current = board(workspace([tab("querendo")]));
    host.confirm.mockResolvedValue(true);
    expect(await allow({ kind: "restart", version: "0.37.0" })).toBe(true);
    const [options] = host.confirm.mock.calls[0];
    expect(options).toMatchObject({ title: "Restart to update?", accept: "Restart now" });
    expect(options.message).toContain("1 conversation in progress");
    expect(options.message).toContain("0.37.0 is already installed");
  });

  it("keeps one question at a time and lets it decide", async () => {
    current = board(workspace([tab("rodando")]));
    let answer!: (quit: boolean) => void;
    host.confirm.mockReturnValue(new Promise<boolean>((resolve) => (answer = resolve)));
    const first = allow({ kind: "quit" });
    await vi.waitFor(() => expect(host.confirm).toHaveBeenCalledTimes(1));
    expect(await allow({ kind: "restart", version: "0.37.0" })).toBe(false);
    answer(true);
    expect(await first).toBe(true);
    expect(host.confirm).toHaveBeenCalledTimes(1);
  });

  it("still asks about the board when the terminal count fails", async () => {
    current = board(workspace([tab("querendo")]));
    host.invoke.mockRejectedValue(new Error("unavailable"));
    host.confirm.mockResolvedValue(false);
    expect(await allow({ kind: "quit" })).toBe(false);
    expect(host.confirm.mock.calls[0][0].message).toContain("1 conversation in progress.");
  });

  it("fails open when the question itself breaks", async () => {
    current = board(workspace([tab("rodando")]));
    host.confirm.mockRejectedValue(new Error("no dialog"));
    expect(await allow({ kind: "quit" })).toBe(true);
  });

  it("quits from the menu only once allowed", async () => {
    current = board(workspace([tab("rodando")]));
    host.confirm.mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    await request();
    expect(host.exit).not.toHaveBeenCalled();
    await request();
    expect(host.exit).toHaveBeenCalledWith(0);
  });

  it("holds a window close until the person answers", async () => {
    current = board(workspace([tab("rodando")]));
    host.confirm.mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    const kept = { preventDefault: vi.fn() };
    await host.close!(kept);
    expect(kept.preventDefault).toHaveBeenCalled();
    const closed = { preventDefault: vi.fn() };
    await host.close!(closed);
    expect(closed.preventDefault).not.toHaveBeenCalled();
  });
});
