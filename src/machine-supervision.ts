import { connect } from "./cloud";
import { fromBack, t } from "./i18n";
import { invoke } from "./ipc";
import type { AgentsSetup } from "./machines";
import { button, checkbox, field, formDialog, input, select } from "./ui";
import { h } from "./util";

export type SupervisorDestination = { name: string; url: string; webhook_protocol: "openbot" | "prometeu"; signing_secret: string };
export type SupervisionSetup = { account: string; supervisor_id?: string; supervisor_revision?: string; destination?: SupervisorDestination };
export type SupervisionOptions = { account: string | null; supervisor: { id: string; revision: string; name: string; url: string; enabled: boolean; webhook_protocol: "openbot" | "prometeu" } | null };
export type SupervisionTest = { status: "pending" | "sending" | "delivered" | "cancelled"; failed: boolean };

/** A delivery receipt proves receipt by the receiver, never completion of an agent task. */
export function connectionTestMessage(result: SupervisionTest): string {
  return t(result.status === "delivered" && !result.failed ? "machines.events.delivered"
    : result.failed || result.status === "cancelled" ? "machines.events.failed" : "machines.events.pending");
}

export async function setupAgentConnection(id: string, existing: boolean, supervision?: SupervisionSetup): Promise<AgentsSetup | null> {
  if (existing) {
    if (!supervision) throw new Error(t("err.supervision.invalid"));
    await invoke("machine_supervision_enable", { id, setup: supervision });
    return null;
  }
  return invoke("machine_agents_enable", { id, rotate: false, supervision });
}

/** The same form serves first MCP setup and adding events to an existing connection. */
export function showAgentConnection(id: string, name: string, existing: boolean,
  saved: (setup: AgentsSetup | null, events: boolean) => void) {
  let closed = false, submitting = false;
  let options: SupervisionOptions | undefined;
  const events = checkbox(t("machines.events.optIn"), existing, "machines.events.optIn");
  const receiver = select("openbot", [["openbot", "OpenBot"], ["prometeu", "Prometeu"]], { track: "machines.events.receiver" });
  const url = input("", false, "machines.events.url"); url.type = "url"; url.maxLength = 2048;
  const secret = input("", false, "machines.events.secret"); secret.type = "password"; secret.autocomplete = "off"; secret.minLength = 16; secret.maxLength = 1024;
  const details = h("div", "ui-form-body");
  const state = h("p", "ui-hint", t("machines.events.loading")); state.setAttribute("role", "status");
  const dialog = formDialog({ title: t("machines.events.title", { name }), save: t(existing ? "machines.events.enable" : "machines.agents.enable"),
    cancel: t("machines.cancel"), track: "machines.events.setup", area: "settings", error: fromBack,
    closed: () => { closed = true; secret.value = ""; },
    submit: async () => {
      let supervision: SupervisionSetup | undefined;
      if (events.control.checked) {
        if (!options?.account) throw new Error(t("machines.events.signInHint"));
        if (options.supervisor && !options.supervisor.enabled) throw new Error(t("err.supervision.disabled"));
        supervision = { account: options.account, ...(options.supervisor ? { supervisor_id: options.supervisor.id, supervisor_revision: options.supervisor.revision } : {
          destination: { name: receiver.value === "openbot" ? "OpenBot" : "Prometeu", url: url.value.trim(),
            webhook_protocol: receiver.value as SupervisorDestination["webhook_protocol"], signing_secret: secret.value },
        }) };
      }
      submitting = true; render();
      try {
        const setup = await setupAgentConnection(id, existing, supervision);
        saved(setup, !!supervision);
      } finally { submitting = false; render(); }
    },
  });
  const signIn = button(t("machines.events.signIn"), () => { dialog.close(); connect(); }, "outline", "machines.events.signIn");
  signIn.hidden = true;
  function render() {
    if (closed) return;
    const selected = events.control.checked;
    details.hidden = !selected;
    const editable = selected && !!options?.account && !options.supervisor;
    for (const control of [receiver.control, receiver.native, url, secret]) control.disabled = submitting || !editable;
    url.required = secret.required = editable;
    events.control.disabled = submitting || existing;
    dialog.save.disabled = submitting || selected && (!options?.account || options.supervisor?.enabled === false);
    if (options) {
      state.textContent = !options.account ? t("machines.events.signInHint")
        : options.supervisor ? t(options.supervisor.enabled ? "machines.events.reuse" : "err.supervision.disabled", { name: options.supervisor.name })
        : t("machines.events.destinationHint");
      signIn.hidden = !!options.account;
    }
    fields.hidden = !editable;
  }
  const fields = h("div", "ui-form-body");
  fields.append(field(t("machines.events.receiver"), receiver.root), field(t("machines.events.url"), url), field(t("machines.events.secret"), secret));
  details.append(state, fields, signIn);
  events.control.onchange = render;
  dialog.body.append(events.label, h("p", "ui-hint", t("machines.events.hint")), details);
  render(); dialog.open();
  void invoke("machine_supervision_options").then(value => { options = value; render(); }).catch(error => {
    if (!closed) { state.textContent = fromBack(error); state.setAttribute("role", "alert"); }
  });
}
