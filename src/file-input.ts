import { open } from "@tauri-apps/plugin-dialog";

export type AttachmentPicker = (options: { title: string; defaultPath?: string }) => Promise<string[]>;

let picker: AttachmentPicker = async options => {
  const selected = await open({ ...options, multiple: true });
  return Array.isArray(selected) ? selected : selected ? [selected] : [];
};

/** Hosts adapt file selection to the execution filesystem at composition. */
export function useAttachmentPicker(implementation: AttachmentPicker) { picker = implementation; }
export const pickAttachments: AttachmentPicker = options => picker(options);
