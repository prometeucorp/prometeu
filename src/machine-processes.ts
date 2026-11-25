import { itemRow, sectionHeader } from "./components/compositions";
import { fromBack, t } from "./i18n";
import { invoke } from "./ipc";
import { tabLabel, type Board } from "./types";
import { button, confirmDialog, formDialog } from "./ui";
import { h } from "./util";

/** Existing terminal_list wire fields; running includes idle agents and sign-in terminals. */
export type MachineTerminal = {
  id: string; workspace: string | null; pid: number; unit: string | null;
  started: number; running: boolean; code: number | null; seq: number;
};

export function runningProcesses(terminals: MachineTerminal[], board: Board, machine: string) {
  return terminals.filter(terminal => terminal.running).sort((a, b) => a.started - b.started || a.id.localeCompare(b.id)).map(terminal => {
    const workspace = board.workspaces.find(workspace => workspace.host === machine && workspace.id === terminal.workspace);
    const tab = workspace?.tabs.find(tab => tab.id === terminal.id);
    return { ...terminal,
      title: tab && workspace ? tabLabel(workspace, tab) : terminal.id.startsWith("claude-login-") ? t("machines.processes.login") : terminal.id,
      workspaceName: workspace?.title || terminal.workspace || t("machines.processes.noWorkspace"),
    };
  });
}

/** Query the host, including processes no longer represented by an open desktop tab. */
export function showMachineProcesses(id: string, name: string) {
  let closed = false, loading = false, stopping = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const dialog = formDialog({ title: t("machines.processes.title", { name }), save: t("review.close"), cancel: t("review.close"),
    submit: async () => {}, error: fromBack, track: "machines.processes", area: "settings",
    closed: () => { closed = true; clearTimeout(timer); },
  });
  dialog.save.remove();
  const rows = h("div", "");
  const status = h("p", "ui-hint", t("machines.processes.loading")); status.setAttribute("role", "status");
  const error = h("p", "ui-hint ui-error"); error.setAttribute("role", "alert");
  const refresh = button(t("machines.processes.refresh"), () => void load(), "outline", "machines.processes.refresh");
  dialog.body.append(sectionHeader({ description: t("machines.processes.hint"), actions: [refresh] }), status, error, rows);

  async function stop(terminal: ReturnType<typeof runningProcesses>[number], control: HTMLButtonElement) {
    if (stopping || closed) return;
    stopping = true; clearTimeout(timer); control.disabled = refresh.disabled = true;
    try {
      if (!await confirmDialog({ title: t("machines.processes.stop"),
        message: t("machines.processes.stopConfirm", { name: terminal.title, pid: terminal.pid }),
        accept: t("machines.processes.stop"), cancel: t("machines.cancel"), track: "machines.processes.stop", area: "settings" }) || closed) return;
      await invoke("machine_terminal_stop", { id, terminal: terminal.id });
      error.textContent = "";
    } catch (cause) { if (!closed) error.textContent = fromBack(cause); }
    finally {
      stopping = false; control.disabled = refresh.disabled = false;
      if (!closed) void load();
    }
  }

  async function load() {
    if (closed || loading || stopping) return;
    clearTimeout(timer); loading = true; refresh.disabled = true;
    try {
      const [terminals, board] = await Promise.all([invoke("machine_terminals", { id }), invoke("load_board")]);
      if (closed || stopping) return;
      const processes = runningProcesses(terminals, board, id);
      const focused = rows.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.focus : undefined;
      rows.replaceChildren(...processes.map(terminal => {
        const control = button(t("machines.processes.stop"), () => void stop(terminal, control), "outline", "machines.processes.stop");
        control.dataset.focus = terminal.id;
        return itemRow({ title: terminal.title, glyph: "terminal",
          description: t("machines.processes.detail", { workspace: terminal.workspaceName, pid: terminal.pid,
            started: new Date(terminal.started * 1000).toLocaleString() }), actions: [control],
        }).root;
      }));
      status.textContent = processes.length ? t("machines.processes.count", { count: processes.length }) : t("machines.processes.empty");
      status.setAttribute("role", "status");
      if (focused) (Array.from(rows.querySelectorAll<HTMLElement>("[data-focus]")).find(control => control.dataset.focus === focused) ?? refresh).focus();
    } catch (cause) {
      if (!closed && !stopping) { rows.replaceChildren(); status.textContent = fromBack(cause); status.setAttribute("role", "alert"); }
    } finally {
      loading = false; refresh.disabled = stopping;
      if (!closed && !stopping) timer = setTimeout(() => void load(), 3000);
    }
  }
  dialog.open(); void load();
}
