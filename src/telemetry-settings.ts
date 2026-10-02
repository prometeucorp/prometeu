import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "./ipc";
import { t, type Key } from "./i18n";
import { button, confirmDialog, field, input, select } from "./ui";
import { h } from "./util";
import { telemetryPeriod, telemetryCleared, type TelemetryFilter } from "./telemetry";
import { costLabel, usageLabel } from "./usage-presentation";

let chosenFrom = "";
let chosenThrough = "";
let chosenWorkspace = "";

export function telemetryRows(): HTMLElement[] {
  const root = h("div", "telemetry-settings");
  const controls = h("div", "telemetry-controls");
  const from = input(chosenFrom); from.type = "date";
  const through = input(chosenThrough); through.type = "date";
  const workspace = select(chosenWorkspace, [["", t("telemetry.all")], ...(chosenWorkspace ? [[chosenWorkspace, t("telemetry.missingWorkspace", { id: chosenWorkspace.slice(0, 8) })] as [string, string]] : [])]);
  const summary = h("dl", "telemetry-summary");
  const details = h("p", "ui-hint"); details.hidden = true;
  const status = h("p", "ui-hint"); status.setAttribute("role", "status");
  let revision = 0;
  let names = new Map<string, string>();
  const filter = (): TelemetryFilter => ({ ...telemetryPeriod(from.value, through.value), ...(workspace.value ? { workspaceId: workspace.value } : {}) });
  const failure = (error: unknown) => {
    const code = String(error).includes("err.telemetry.filter") ? "err.telemetry.filter"
      : String(error).includes("err.telemetry.export") ? "err.telemetry.export" : "err.telemetry.storage";
    status.textContent = t(code);
  };
  const refresh = async () => {
    const mine = ++revision;
    try {
      const [insights, board] = await Promise.all([
        invoke("telemetry_insights", { filter: filter() }),
        invoke("load_board"),
      ]);
      if (mine !== revision) return;
      const result = insights.summary;
      names = new Map(board.workspaces.map(w => [board.telemetry_ids?.[`workspace:${w.id}`] ?? w.id, w.title]));
      const selected = workspace.value;
      const ids = new Set([...result.workspaceIds, ...(selected ? [selected] : [])]);
      workspace.setOptions([["", t("telemetry.all")], ...[...ids].map(id => [id, names.get(id) ?? t("telemetry.missingWorkspace", { id: id.slice(0, 8) })] as [string, string])], selected);
      summary.replaceChildren(); details.hidden = true;
      if (result.health.unavailable) { status.textContent = t("err.telemetry.storage"); return; }
      const show = (key: Key, value: string | number | null) => summary.append(h("dt", "", t(key)), h("dd", "", value === null ? t("telemetry.unknown") : String(value)));
      const date = (at: number | null) => at === null ? null : new Date(at).toLocaleString();
      show("telemetry.first", date(result.firstRecordedAt)); show("telemetry.last", date(result.lastRecordedAt));
      show("telemetry.turns", result.turns); show("telemetry.completed", result.completedTurns);
      show("telemetry.input", insights.usage.inputTokens); show("telemetry.output", insights.usage.outputTokens);
      if (insights.usage.costUsd !== null) show("usage.cost", costLabel(insights.usage.costUsd));
      show("telemetry.coverage", `${result.measuredTurns} / ${result.turns}`);
      show("telemetry.partial", result.partialTurns); show("telemetry.incomplete", result.incompleteExecutions);
      show("telemetry.execution", result.executionSumMs === null ? null : Math.round(result.executionSumMs / 1000));
      show("telemetry.active", result.activeAgentMs === null ? null : Math.round(result.activeAgentMs / 1000));
      show("telemetry.wait", result.humanWaitMs === null ? null : Math.round(result.humanWaitMs / 1000));
      show("telemetry.clock", result.clockAnomalies);
      const sourceLabels: Record<string, Key> = { naming: "usage.source.naming", "plugin-maker": "usage.source.pluginMaker",
        action: "usage.source.action", delegation: "usage.source.delegation" };
      for (const row of [...insights.sources, ...insights.origins]) {
        const label = sourceLabels[row.id];
        if (label) show(label, usageLabel(row.usage) || t("telemetry.unknown"));
      }
      const hints = [insights.usage.costUsd === null ? "" : t("usage.costHint"), insights.origins.length ? t("usage.originsHint") : ""].filter(Boolean);
      details.hidden = hints.length === 0; details.textContent = hints.join(" ");
      status.textContent = t(result.health.unavailable ? "err.telemetry.storage" : result.health.failures ? "telemetry.incompleteHistory" : "telemetry.local");
    } catch (error) { if (mine === revision) { summary.replaceChildren(); details.hidden = true; failure(error); } }
  };
  const update = button(t("telemetry.refresh"), () => void refresh(), "ghost");
  const exportButton = button(t("telemetry.export"), async () => {
    exportButton.disabled = true;
    try {
      const selected = filter();
      const path = await save({ defaultPath: "prometeu-telemetry.jsonl", filters: [{ name: "JSONL", extensions: ["jsonl"] }] });
      if (path) { await invoke("telemetry_export", { filter: selected, path }); status.textContent = t("telemetry.exported"); }
    } catch (error) { failure(error); }
    finally { exportButton.disabled = false; }
  }, "ghost");
  const erase = button(t("telemetry.clear"), async () => {
    if (!await confirmDialog({ title: t("telemetry.clear"), message: t("telemetry.clearConfirm"), accept: t("telemetry.clear"), cancel: t("telemetry.cancel") })) return;
    erase.disabled = true; ++revision;
    try { await invoke("telemetry_clear"); telemetryCleared(); workspace.value = ""; chosenWorkspace = ""; await refresh(); }
    catch (error) { failure(error); }
    finally { erase.disabled = false; }
  }, "ghost");
  from.onchange = () => { chosenFrom = from.value; void refresh(); };
  through.onchange = () => { chosenThrough = through.value; void refresh(); };
  workspace.onchange = () => { chosenWorkspace = workspace.value; void refresh(); };
  controls.append(field(t("telemetry.from"), from), field(t("telemetry.through"), through), field(t("telemetry.workspace"), workspace.control));
  const actions = h("div", "telemetry-controls"); actions.append(update, exportButton, erase);
  root.append(h("p", "ui-hint", t("telemetry.description")), controls, summary, details, status, actions);
  void refresh();
  return [root];
}
