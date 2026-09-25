import { invoke } from "./ipc";

/// Pasted screenshots and files copied in Finder reach the webview without a path, so the backend
/// materializes them under the private attachments directory. Attaching them here keeps paste and
/// drop equivalent; text keeps the browser's own paste.
export function pasteFiles(event: ClipboardEvent, put: (paths: string[]) => void, fail: (error: unknown) => void) {
  const data = event.clipboardData;
  // WebKitGTK exposes neither type nor file for a pasted image; only the backend can read it there.
  if (!data || (!data.files.length && data.types.length)) return;
  event.preventDefault();
  invoke("paste_files")
    .then((paths) => {
      if (paths.length) put(paths);
    })
    .catch(fail);
}
