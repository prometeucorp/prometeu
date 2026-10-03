import { invoke } from "@tauri-apps/api/core";
import { useIpc } from "../ipc";
import { fromBack, t } from "../i18n";
import { listen } from "@tauri-apps/api/event";
import { useConnectionRecovery } from "../connection";
import { useTerminalSnapshots, type TerminalOutput } from "../terminal-output";
import { Recovery } from "./recovery";
import { wslCommands } from "./transport";
import type { Commands } from "../wsl/ipc";
import type { Target } from "../wsl/session";
import { useApplicationMenu, windowsApplicationMenu } from "../appmenu";
import { useAttachmentPicker } from "../file-input";
import { installWslDrops, pickWslAttachments } from "./file-input";
import "../style.css";

function host<K extends keyof Commands>(command: K, ...args: Commands[K]["args"] extends undefined ? [] : [Commands[K]["args"]]): Promise<Commands[K]["result"]> {
  return invoke(command, args[0]);
}

const key = "prometeu:windows-target";
let previous: Target | null = null;
try {
  const value: unknown = JSON.parse(localStorage.getItem(key) ?? "null");
  if (value && typeof value === "object" && ["distribution", "executable", "root", "workdir", "codex"].every(key => typeof (value as Record<string, unknown>)[key] === "string")) previous = value as Target;
} catch { /* An invalid cache does not override the system default. */ }
try {
  const target = await host("application_open", { previous });
  localStorage.setItem(key, JSON.stringify(target));
  useIpc(wslCommands);
  useApplicationMenu(windowsApplicationMenu);
  useAttachmentPicker(pickWslAttachments);
  useTerminalSnapshots({ read: async session => {
    const snapshot = await invoke<TerminalOutput | number[] | null>("application_request", { command: "pty_buffer", args: { session, snapshot: true } });
    return Array.isArray(snapshot) ? { data: snapshot } : snapshot ?? { data: [], seq: 0, running: false };
  },
  });
  const recovery = new Recovery(() => host("application_reconnect"), recovering => {
    const status = document.getElementById("msg")!;
    const message = t("windows.reconnecting");
    if (recovering) {
      status.textContent = message;
      status.classList.add("err");
    } else if (status.textContent === message) {
      status.textContent = "";
      status.classList.remove("err");
    }
  });
  useConnectionRecovery(recovery);
  await import("../main");
  await listen<{ frame: { lifecycle?: string } }>("wsl-runtime", ({ payload }) => {
    if (payload.frame.lifecycle === "disconnected") void recovery.recover();
  });
  // Also detect a lost attachment during initial listener registration.
  void recovery.recover();
  await installWslDrops();
} catch (error) {
  // Startup failures use the existing application status area, without a setup flow.
  const status = document.getElementById("msg")!;
  status.textContent = fromBack(error);
  status.classList.add("err");
  status.setAttribute("role", "alert");
}
