import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import type { AttachmentPicker } from "../file-input";
import type { Commands } from "../wsl/ipc";

function paths(paths: string[], direction: "linux" | "windows") {
  return invoke<Commands["application_paths"]["result"]>("application_paths", { paths, direction });
}

export const pickWslAttachments: AttachmentPicker = async options => {
  const defaultPath = options.defaultPath ? (await paths([options.defaultPath], "windows"))[0] : undefined;
  const selected = await open({ ...options, defaultPath, multiple: true });
  return paths(Array.isArray(selected) ? selected : selected ? [selected] : [], "linux");
};

/** Adapt native physical coordinates and paths to the existing file-drag contract. */
export function installWslDrops() {
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    const position = "position" in payload
      ? { x: payload.position.x / devicePixelRatio, y: payload.position.y / devicePixelRatio }
      : undefined;
    if (payload.type !== "drop") {
      void emit("file-drag", { type: payload.type, position, paths: [] });
      return;
    }
    const id = crypto.randomUUID();
    // Keep the original target while path conversion is pending, including launcher drafts.
    void emit("file-drag", { type: "pending", id, position, paths: [] }).then(async () => {
      try {
        await emit("file-drag", { type: "received", id, paths: await paths(payload.paths, "linux") });
      } catch {
        await emit("file-drag", { type: "received", id, paths: [], error: "chat.drop.failed" });
      }
    });
  });
}
