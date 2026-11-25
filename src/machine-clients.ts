import { itemRow, sectionHeader } from "./components/compositions";
import { fromBack, t } from "./i18n";
import { invoke } from "./ipc";
import { button, confirmDialog, formDialog } from "./ui";
import { h } from "./util";

/** clients_list wire fields (ADR 0104); `current` is this computer's own attachment. */
export type MachineClient = { device: string; label: string; connected: number; current: boolean };

/** This computer first, then the others by when they connected. */
export function connectedComputers(clients: MachineClient[]) {
  return [...clients].sort((a, b) => Number(b.current) - Number(a.current) || a.connected - b.connected || a.device.localeCompare(b.device))
    .map(client => ({ ...client, name: client.label || (client.device ? client.device.slice(0, 8) : t("machines.clients.unnamed")) }));
}

/** Show the computers attached to the machine's host, and disconnect another one. */
export function showMachineClients(id: string, name: string) {
  let closed = false, loading = false, detaching = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const dialog = formDialog({ title: t("machines.clients.title", { name }), save: t("review.close"), cancel: t("review.close"),
    submit: async () => {}, error: fromBack, track: "machines.clients", area: "settings",
    closed: () => { closed = true; clearTimeout(timer); },
  });
  dialog.save.remove();
  const rows = h("div", "");
  const status = h("p", "ui-hint", t("machines.clients.loading")); status.setAttribute("role", "status");
  const error = h("p", "ui-hint ui-error"); error.setAttribute("role", "alert");
  const refresh = button(t("machines.processes.refresh"), () => void load(), "outline", "machines.clients.refresh");
  dialog.body.append(sectionHeader({ description: t("machines.clients.hint"), actions: [refresh] }), status, error, rows);

  async function detach(client: ReturnType<typeof connectedComputers>[number], control: HTMLButtonElement) {
    if (detaching || closed) return;
    detaching = true; clearTimeout(timer); control.disabled = refresh.disabled = true;
    try {
      if (!await confirmDialog({ title: t("machines.clients.detach"), message: t("machines.clients.detachConfirm", { name: client.name }),
        accept: t("machines.clients.detach"), cancel: t("machines.cancel"), track: "machines.clients.detach", area: "settings" }) || closed) return;
      await invoke("machine_client_detach", { id, device: client.device });
      error.textContent = "";
    } catch (cause) { if (!closed) error.textContent = fromBack(cause); }
    finally {
      detaching = false; control.disabled = refresh.disabled = false;
      if (!closed) void load();
    }
  }

  async function load() {
    if (closed || loading || detaching) return;
    clearTimeout(timer); loading = true; refresh.disabled = true;
    try {
      const clients = connectedComputers(await invoke("machine_clients", { id }));
      if (closed || detaching) return;
      rows.replaceChildren(...clients.map(client => {
        const actions: HTMLButtonElement[] = [];
        if (!client.current) {
          const control = button(t("machines.clients.detach"), () => void detach(client, control), "outline", "machines.clients.detach");
          actions.push(control);
        }
        return itemRow({ title: client.name, glyph: "signal",
          status: client.current ? t("machines.clients.current") : undefined,
          description: t("machines.clients.detail", { connected: new Date(client.connected * 1000).toLocaleString() }), actions,
        }).root;
      }));
      status.textContent = t("machines.clients.count", { count: clients.length });
      status.setAttribute("role", "status");
    } catch (cause) {
      if (!closed && !detaching) { rows.replaceChildren(); status.textContent = fromBack(cause); status.setAttribute("role", "alert"); }
    } finally {
      loading = false; refresh.disabled = detaching;
      if (!closed && !detaching) timer = setTimeout(() => void load(), 5000);
    }
  }
  dialog.open(); void load();
}
