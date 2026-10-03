import { beforeEach, expect, it, vi } from "vitest";
const ports = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), emit: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: ports.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: ports.open }));
vi.mock("@tauri-apps/api/event", () => ({ emit: ports.emit }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: ports.listen }) }));
import { installWslDrops, pickWslAttachments } from "./file-input";

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("devicePixelRatio", 2);
  ports.emit.mockResolvedValue(undefined);
});

it("converts the picker root and selected files without changing their names or ordering", async () => {
  ports.invoke.mockResolvedValueOnce(["C:\\work"]).mockResolvedValueOnce(["/drive/spec ' ação.txt", "/home/me/image.png"]);
  ports.open.mockResolvedValue(["C:\\spec ' ação.txt", "\\\\wsl.localhost\\Distro\\home\\me\\image.png"]);
  expect(await pickWslAttachments({ title: "Attach", defaultPath: "/work" })).toEqual(["/drive/spec ' ação.txt", "/home/me/image.png"]);
  expect(ports.open).toHaveBeenCalledWith({ title: "Attach", multiple: true, defaultPath: "C:\\work" });
  expect(ports.invoke.mock.calls.map(call => call[1].direction)).toEqual(["windows", "linux"]);
  ports.invoke.mockRejectedValue("wrong distribution");
  await expect(pickWslAttachments({ title: "Attach" })).rejects.toBe("wrong distribution");
});

it("keeps the drop target pending while adapting physical coordinates and execution paths", async () => {
  let finish!: (paths: string[]) => void;
  ports.invoke.mockReturnValue(new Promise<string[]>(resolve => { finish = resolve; }));
  await installWslDrops();
  const receive = ports.listen.mock.calls[0][0];
  receive({ payload: { type: "drop", paths: ["C:\\notes.txt"], position: { x: 160, y: 240 } } });
  await vi.waitFor(() => expect(ports.invoke).toHaveBeenCalled());
  const pending = ports.emit.mock.calls[0][1];
  expect(pending).toMatchObject({ type: "pending", position: { x: 80, y: 120 }, paths: [] });
  finish(["/mnt/c/notes.txt"]);
  await vi.waitFor(() => expect(ports.emit).toHaveBeenCalledTimes(2));
  expect(ports.emit.mock.calls[1][1]).toEqual({ type: "received", id: pending.id, paths: ["/mnt/c/notes.txt"] });
});

it("settles a failed conversion without attaching any partial result", async () => {
  ports.invoke.mockRejectedValue("wrong distribution");
  await installWslDrops();
  ports.listen.mock.calls[0][0]({ payload: { type: "drop", paths: ["bad"], position: { x: 0, y: 0 } } });
  await vi.waitFor(() => expect(ports.emit).toHaveBeenCalledTimes(2));
  expect(ports.emit.mock.calls[1][1]).toMatchObject({ type: "received", paths: [], error: "chat.drop.failed" });
});
