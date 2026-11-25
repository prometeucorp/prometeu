import { open } from "@tauri-apps/plugin-dialog";

export async function pickAttachments(options: { title: string; defaultPath?: string }): Promise<string[]> {
  const selected = await open({ ...options, multiple: true });
  return Array.isArray(selected) ? selected : selected ? [selected] : [];
}
