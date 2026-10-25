import { track } from "./analytics";
import { descriptor, descriptors, type AgentDescriptor, type AuthMethod } from "./agents";
import { invoke, type IpcCall } from "./ipc";
import { brand, icon } from "./icons";
import { fromBack, t } from "./i18n";
import type { ProviderId } from "./types";
import { avatar, badge, button, confirmDialog, disclosure, field, formDialog, menuButton, select } from "./ui";
import { h } from "./util";

export type Account = {
  id: string; provider: ProviderId; email: string | null; plan: string | null;
  connected: boolean; revision: number; authMethod?: string; keySuffix?: string;
};
export type Accounts = {
  accounts: Account[]; active: Partial<Record<ProviderId, string>>;
  login: { id: string; provider: ProviderId } | null;
};

export let accounts: Accounts | null = null;
let busy = false;
let say: (text: string, error?: boolean) => void = () => {};
let quota: (id: string) => string = () => "";
const listeners = new Set<() => void>();
export function onChange(listener: () => void) { listeners.add(listener); return () => listeners.delete(listener); }
function changed() { for (const listener of listeners) listener(); }
export function init(options: { say: typeof say; quota: typeof quota }) { say = options.say; quota = options.quota; }
export function refreshUsage() { changed(); }
export function update(next: Accounts): boolean {
  const selected = (data: Accounts | null) => data && JSON.stringify(data.accounts
    .filter(account => data.active[account.provider] === account.id).map(({ id, revision }) => [id, revision]));
  const selectionChanged = selected(accounts) !== selected(next);
  accounts = next;
  changed();
  return selectionChanged;
}
export function accountName(account: Account) {
  if (account.authMethod === "external") return t("account.external.name", { provider: descriptor(account.provider).label });
  return account.email || t(account.id === account.provider ? "account.terminal" : "account.new");
}

/// The launcher's preflight follows the same global selection as process creation.
export function selected(provider: ProviderId): Account | undefined {
  const id = accounts?.active[provider];
  return accounts?.accounts.find(account => account.provider === provider && id === account.id);
}

export function openPicker(provider: ProviderId) {
  const dialog = formDialog({ track: "account.dialog", area: "accounts",
    title: t("account.use"), save: t("account.continue"), cancel: t("account.cancel"), error: fromBack,
    submit: async () => { if (!selected(provider)) throw new Error(t("launcher.account.required")); },
    closed: () => forget(),
  });
  dialog.root.classList.add("account-dialog");
  dialog.root.querySelector(".sheettop")!.append(h("p", "ui-hint", t("account.selection.help", { provider: descriptor(provider).label })));
  const draw = () => {
    const focus = (document.activeElement as HTMLElement | null)?.dataset.focus;
    dialog.body.replaceChildren(render([descriptor(provider)]));
    dialog.save.disabled = !selected(provider) || busy || !!accounts?.login;
    if (focus) dialog.body.querySelector<HTMLElement>(`[data-focus="${CSS.escape(focus)}"]`)?.focus();
  };
  const forget = onChange(draw);
  draw();
  dialog.open();
}

function focusAccount(id: string) {
  requestAnimationFrame(() => {
    const nodes = document.querySelectorAll<HTMLElement>(`[data-account="${CSS.escape(id)}"] .account-select`);
    const node = [...nodes].find(node => node.getClientRects().length);
    node?.scrollIntoView({ block: "nearest" });
    node?.focus({ preventScroll: true });
  });
}

async function call(...args: IpcCall<"account_select" | "account_login" | "account_remove">) {
  if (busy || accounts?.login) throw new Error(t("err.account.busy"));
  busy = true; changed();
  try {
    const before = new Set(accounts?.accounts.map(a => a.id));
    const provider = args[0] === "account_login" ? args[1].provider : accounts?.accounts.find(account => account.id === args[1].id)?.provider;
    if (provider && args[0] === "account_login") track({ name: "provider_account_action_used", provider, action: "login_start" });
    const next = await invoke(...args);
    if (provider && args[0] !== "account_login") track({ name: "provider_account_action_used", provider, action: args[0] === "account_select" ? "select" : "remove" });
    update(next);
    if (args[0] === "account_login") {
      say(t(args[1].method === "external" ? "account.external.added" : "account.connected", { provider: descriptor(args[1].provider).label }));
      const added = next.accounts.find(a => !before.has(a.id));
      const id = added?.id ?? args[1].id;
      if (id) focusAccount(id);
    }
  } finally { busy = false; changed(); }
}
function run(work: Promise<unknown>) { void work.catch(error => say(fromBack(error), true)); }

async function remove(account: Account) {
  if (accounts?.active[account.provider] === account.id && !await confirmDialog({ track: "account.remove", area: "accounts",
    title: t("account.removeActive"), message: t("account.removeActive.message"),
    accept: t("account.remove"), cancel: t("account.cancel"),
  })) return;
  await call("account_remove", { id: account.id });
  requestAnimationFrame(() => {
    const groups = document.querySelectorAll<HTMLElement>(`[data-provider-accounts="${account.provider}"]`);
    const group = [...groups].find(node => node.getClientRects().length);
    if (group) (group.querySelector<HTMLElement>(".account-select:not(:disabled), .account-add button:not(:disabled)") ?? group).focus();
  });
}

function start(provider: ProviderId, method: AuthMethod, id: string | null) {
  run(call("account_login", { provider, id, method: method.id }));
}
export function add(provider: ProviderId, id: string | null = null) {
  const methods = descriptor(provider).authMethods ?? [];
  const account = accounts?.accounts.find(a => a.id === id);
  const previous = methods.find(m => m.id === account?.authMethod);
  if (previous) return start(provider, previous, id);
  if (methods.length === 1) return start(provider, methods[0], id);
  if (!methods.length) return;
  const method = select(methods[0].id, methods.map(m => [m.id, m.label]), { track: "account.method" });
  let selected: AuthMethod | undefined;
  const dialog = formDialog({ track: "account.dialog", area: "accounts",
    title: t("account.add"), save: t("account.continue"), cancel: t("account.cancel"), error: fromBack,
    submit: async () => { selected = methods.find(m => m.id === method.value); },
    closed: () => { if (selected) start(provider, selected, id); },
  });
  dialog.body.append(field(t("account.method"), method.root));
  dialog.open();
}

function card(account: Account, compact: boolean): HTMLElement {
  const active = accounts?.active[account.provider] === account.id;
  const external = account.id === account.provider;
  const loggingIn = accounts?.login?.id === account.id;
  const root = h("section", `uaccount${active ? " active" : ""}`);
  root.dataset.account = account.id;
  const choose = button("", () => run(call("account_select", { id: account.id })), undefined, "account.select");
  choose.classList.add("account-select");
  choose.dataset.focus = `select-${account.id}`;
  choose.setAttribute("aria-label", accountName(account));
  choose.setAttribute("aria-pressed", String(active));
  choose.disabled = busy || !!accounts?.login || (!account.connected && !external);
  const content = h("div", "account-content");
  const heading = h("div", "account-heading");
  const name = h("strong", "", accountName(account)); name.title = accountName(account);
  const identity = h("div", "account-identity");
  identity.append(name);
  const meta = h("div", "account-meta");
  if (account.plan) meta.append(h("span", "", account.plan));
  if (active || account.connected || external) {
    const state = badge(t(active ? "account.active" : "account.use"), active);
    state.classList.add("account-state");
    meta.append(state);
  } else if (!loggingIn) meta.append(h("span", "", t("account.disconnected")));
  identity.append(meta);
  const actions = menuButton("…", () => [
    ...(!external ? [{ track: "account.reconnect", label: t("account.reconnect"), run: () => add(account.provider, account.id) }] : []),
    { track: "account.remove", label: t("account.remove"), danger: true, run: () => run(remove(account)) },
  ]);
  actions.setAttribute("aria-label", t("account.actions"));
  actions.dataset.track = "account.actions";
  actions.dataset.focus = `actions-${account.id}`;
  actions.disabled = busy || !!accounts?.login;
  heading.append(avatar(), identity, actions); content.append(heading);
  if (loggingIn) {
    const wait = h("div", "account-wait", t("account.browser")); wait.setAttribute("role", "status");
    wait.append(button(t("account.cancel"), () => run(invoke("account_login_cancel", { id: account.id }).then(() => track({ name: "provider_account_action_used", provider: account.provider, action: "login_cancel" }))), undefined, "account.cancel"));
    content.append(wait);
  }
  const limits = h("div", "account-quotas"); limits.innerHTML = quota(account.id);
  if (compact) {
    const usage = disclosure(t("status.usage"), limits);
    usage.classList.add("account-usage");
    usage.dataset.settingsDisclosure = `account-usage-${account.id}`;
    usage.querySelector("summary")!.dataset.track = "account.usage";
    usage.querySelector("summary")!.dataset.focus = `usage-${account.id}`;
    content.append(usage);
  } else content.append(limits);
  root.append(choose, content);
  return root;
}

export function render(providers: readonly AgentDescriptor[] = descriptors(), manage?: () => void, compact = false): HTMLElement {
  const root = h("div", `accounts-view${compact ? " compact" : ""}`);
  root.dataset.trackArea = "accounts";
  for (const provider of providers) {
    const group = h("section", "accounts-provider"); group.dataset.providerAccounts = provider.id; group.tabIndex = -1; group.setAttribute("aria-label", provider.label);
    const heading = h("div", "uhead");
    const mark = h("span", ""); mark.innerHTML = brand(provider.id);
    heading.append(mark, h("span", "uname", provider.label)); group.append(heading);
    if (!provider.installed) group.append(h("p", "ui-hint", provider.unavailableReason ? fromBack(provider.unavailableReason) : t("account.install", { provider: provider.label })));
    const list = accounts?.accounts.filter(a => a.provider === provider.id) ?? [];
    if (!list.length) group.append(h("div", "uempty", t("account.empty")));
    group.append(...list.map(account => card(account, compact)));
    if (provider.accountNotice) group.append(h("p", "account-notice ui-hint", fromBack(provider.accountNotice)));
    const external = provider.authMethods.some(method => method.kind === "external");
    const footer = h("div", "account-add");
    if (external && list.length) {
      const attached = h("span", "account-attached");
      attached.innerHTML = icon("check", 14);
      attached.append(document.createTextNode(t("account.external.attached", { provider: provider.label })));
      footer.append(attached);
    } else {
      const addButton = button(t(external ? "account.external.attach" : "account.add", { provider: provider.label }), () => add(provider.id));
      addButton.insertAdjacentHTML("afterbegin", icon("plus", 14));
      addButton.dataset.track = "account.add";
      addButton.dataset.focus = `add-${provider.id}`;
      addButton.disabled = !provider.installed || busy || !!accounts?.login || !provider.authMethods.length;
      footer.append(addButton);
    }
    if (manage) {
      const manageButton = button(t("account.manage"), manage, "ghost");
      manageButton.dataset.track = "account.manage";
      manageButton.dataset.focus = `manage-${provider.id}`;
      footer.append(manageButton);
    }
    group.append(footer); root.append(group);
  }
  return root;
}
