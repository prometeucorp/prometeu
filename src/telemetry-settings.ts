import { track } from "./analytics";
import { save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "./ipc";
import { t } from "./i18n";
import { button, confirmDialog, field, input, select } from "./ui";
import { h } from "./util";
import { descriptors } from "./agents";
import { onTelemetryCleared, telemetryPeriod, telemetryCleared, type TelemetryFilter } from "./telemetry";
import { usageHistoryView } from "./components/usage-history";
import { sectionHeader } from "./components/compositions";
import { usageHistoryData } from "./usage-history";

let chosenFrom = "";
let chosenThrough = "";
let chosenWorkspace = "";
let dispose: (() => void) | undefined;
let activeRoot: HTMLElement | undefined;
let refreshActive: (() => void) | undefined;

/** Settings retires the controller before replacing its section or entering search results. */
export function disposeTelemetry() { dispose?.(); dispose = undefined; activeRoot = undefined; refreshActive = undefined; }

export function telemetryRows(): HTMLElement[] {
  // Settings redraws on board/catalog updates; retain controls and an open detail dialog.
  if (activeRoot) { refreshActive?.(); return [activeRoot]; }
  const root = h("div", "telemetry-settings"); root.dataset.trackArea = "usage_history";
  activeRoot = root;
  const controls = h("div", "telemetry-controls");
  const from = input(chosenFrom, false, "usage_history.from"); from.type = "date"; from.dataset.focus = "history-from";
  const through = input(chosenThrough, false, "usage_history.through"); through.type = "date"; through.dataset.focus = "history-through";
  const workspace = select(chosenWorkspace, [["", t("telemetry.all")], ...(chosenWorkspace ? [[chosenWorkspace, t("telemetry.missingWorkspace", { id: chosenWorkspace.slice(0, 8) })] as [string, string]] : [])]);
  workspace.control.dataset.track = "usage_history.workspace"; workspace.control.dataset.focus = "history-workspace";
  const view = usageHistoryView(); view.root.hidden = true;
  const status = h("p", "telemetry-status ui-hint"); status.setAttribute("role", "status");
  let revision = 0, generation = 0, disposed = false, erasing = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let unlisten: (() => void) | undefined;
  const filter = (): TelemetryFilter => ({ ...telemetryPeriod(from.value, through.value), ...(workspace.value ? { workspaceId: workspace.value } : {}) });
  const failure = (error: unknown) => {
    const code = String(error).includes("err.telemetry.filter") ? "err.telemetry.filter"
      : String(error).includes("err.telemetry.export") ? "err.telemetry.export" : "err.telemetry.storage";
    status.textContent = t(code);
  };
  const refresh = async () => {
    clearTimeout(timer);
    if (disposed || erasing) return false;
    const mine = ++revision;
    root.setAttribute("aria-busy", "true");
    if (view.root.hidden) status.textContent = t("usage.loading");
    try {
      const selectedFilter = filter();
      from.removeAttribute("aria-invalid"); through.removeAttribute("aria-invalid");
      const [insights, board] = await Promise.all([
        invoke("telemetry_insights", { filter: selectedFilter }), invoke("load_board"),
      ]);
      if (disposed || mine !== revision) return false;
      const names = new Map(board.workspaces.map(w => [board.telemetry_ids?.[`workspace:${w.id}`] ?? w.id, w.title]));
      const selected = workspace.value;
      const ids = new Set([...insights.summary.workspaceIds, ...(selected ? [selected] : [])]);
      workspace.setOptions([["", t("telemetry.all")], ...[...ids].map(id => [id, names.get(id) ?? t("telemetry.missingWorkspace", { id: id.slice(0, 8) })] as [string, string])], selected);
      if (insights.summary.health.unavailable) { view.clear(); status.textContent = t("err.telemetry.storage"); return false; }
      view.update(usageHistoryData(insights, board, new Map(descriptors().map(provider => [provider.id, provider.label]))));
      status.textContent = insights.summary.health.failures ? t("telemetry.incompleteHistory") : "";
      return true;
    } catch (error) {
      if (disposed || mine !== revision) return false;
      view.clear(); failure(error);
      if (String(error).includes("err.telemetry.filter")) { from.setAttribute("aria-invalid", "true"); through.setAttribute("aria-invalid", "true"); }
      return false;
    } finally { if (!disposed && mine === revision) root.setAttribute("aria-busy", "false"); }
  };
  const schedule = () => {
    if (disposed || erasing || root.closest("[hidden]")) return;
    ++revision; clearTimeout(timer); timer = setTimeout(() => void refresh(), 150);
  };
  refreshActive = schedule;
  void listen("telemetry-changed", schedule).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => { /* Manual refresh remains available. */ });
  const stopCleared = onTelemetryCleared(() => {
    ++generation; ++revision; clearTimeout(timer); view.clear(); status.textContent = "";
    if (!erasing) void refresh();
  });
  dispose = () => { disposed = true; ++generation; ++revision; clearTimeout(timer); unlisten?.(); stopCleared(); view.destroy(); };

  const update = button(t("telemetry.refresh"), () => { void refresh().then(shown => { if (shown) track({ name: "usage_history_action_used", action: "refresh" }); }); }, "ghost", "usage_history.refresh");
  const exportButton = button(t("telemetry.export"), async () => {
    exportButton.disabled = true; const started = generation;
    try {
      const selected = filter();
      const path = await save({ defaultPath: "prometeu-telemetry.jsonl", filters: [{ name: "JSONL", extensions: ["jsonl"] }] });
      if (disposed || generation !== started) return;
      if (path) {
        await invoke("telemetry_export", { filter: selected, path });
        if (disposed || generation !== started) return;
        track({ name: "usage_history_action_used", action: "export" }); status.textContent = t("telemetry.exported");
      }
    } catch (error) { if (!disposed && generation === started) failure(error); }
    finally { exportButton.disabled = false; }
  }, "ghost", "usage_history.export");
  const erase = button(t("telemetry.clear"), async () => {
    if (!await confirmDialog({ track: "usage_history.clear", area: "usage_history", title: t("telemetry.clear"), message: t("telemetry.clearConfirm"), accept: t("telemetry.clear"), cancel: t("telemetry.cancel") }) || disposed) return;
    erase.disabled = true; erasing = true; ++revision; clearTimeout(timer); view.clear();
    try {
      await invoke("telemetry_clear"); telemetryCleared();
      if (disposed) return;
      track({ name: "usage_history_action_used", action: "clear" }); workspace.value = ""; chosenWorkspace = "";
      erasing = false; await refresh();
    } catch (error) { if (!disposed) failure(error); }
    finally { erasing = false; erase.disabled = false; }
  }, "ghost", "usage_history.clear");
  const changed = () => {
    chosenFrom = from.value; chosenThrough = through.value; chosenWorkspace = workspace.value;
    view.clear(); track({ name: "usage_history_action_used", action: "filter" }); void refresh();
  };
  from.onchange = through.onchange = workspace.onchange = changed;
  const reset = button(t("history.reset"), () => { from.value = ""; through.value = ""; workspace.value = ""; changed(); }, "ghost", "usage_history.reset");
  controls.append(field(t("telemetry.from"), from), field(t("telemetry.through"), through), field(t("telemetry.workspace"), workspace.control), reset);
  const actions = h("div", "telemetry-controls telemetry-actions"); actions.append(exportButton, erase);
  root.append(sectionHeader({ title: t("history.title"), description: t("history.intro"), actions: [update] }), controls, status, view.root, actions);
  void refresh();
  return [root];
}
