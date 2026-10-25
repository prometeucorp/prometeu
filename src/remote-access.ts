import { invoke, type IpcResult } from "./ipc";
import { fromBack, t } from "./i18n";
import { button, checkbox } from "./ui";
import { h } from "./util";

/** Preferences stay on the Mac. The browser grants each integration a subset of these projects. */
export function remoteAccessRow(projects: { id: string; name: string }[], say: (text: string, error?: boolean) => void): HTMLElement {
  const row = h("div", "setrow");
  row.dataset.trackArea = "settings.remoteAccess";
  const copy = h("div", "txt");
  const status = h("span", "", t("remote.loading"));
  status.setAttribute("role", "status");
  copy.append(h("b", "", t("remote.title")), h("p", "", t("remote.description")), status);
  row.append(copy);
  const controls = h("div", "");
  copy.append(controls);
  function paint(value: IpcResult<"remote_access_status">) {
    status.textContent = t(value.online ? "remote.online" : value.enabled ? "remote.offline" : "remote.disabled");
    controls.replaceChildren();
    const choices = projects.map(project => {
      const choice = checkbox(project.name, value.projects.includes(project.id), "settings.remoteAccess.project");
      controls.append(choice.label);
      return { id: project.id, control: choice.control };
    });
    const save = (enabled: boolean) => {
      controls.querySelectorAll<HTMLButtonElement | HTMLInputElement>("button,input").forEach(control => { control.disabled = true; });
      void invoke("remote_access_save", { enabled, projects: choices.filter(c => c.control.checked).map(c => c.id) })
        .then(paint).catch(() => { say(t("remote.failed"), true); paint(value); });
    };
    const enable = button(t(value.enabled ? "remote.save" : "remote.enable"), () => save(true), "outline", "settings.remoteAccess.save");
    enable.disabled = !value.manage_url || !projects.length;
    controls.append(enable);
    if (value.enabled) controls.append(button(t("remote.disable"), () => save(false), "ghost", "settings.remoteAccess.disable"));
    if (value.manage_url) controls.append(button(t("remote.manage"), () => {
      void invoke("open_external", { url: value.manage_url! }).catch(error => say(fromBack(error), true));
    }, "ghost", "settings.remoteAccess.manage"));
    controls.append(button(t("remote.refresh"), () => {
      void invoke("remote_access_status").then(paint).catch(error => say(fromBack(error), true));
    }, "ghost", "settings.remoteAccess.refresh"));
  }
  void invoke("remote_access_status").then(paint).catch(() => { status.textContent = t("remote.unavailable"); });
  return row;
}
