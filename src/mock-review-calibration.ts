import { isCalibrationRecord, type CalibrationRecord, type CalibrationStatus } from "./review-calibration";

type Saved = { enabled: boolean; generation: number; records: CalibrationRecord[] };
const storageError = () => 'i18n:{"code":"err.calibration.storage"}';
const checkHistory = (records: unknown): records is CalibrationRecord[] => Array.isArray(records) && records.every(isCalibrationRecord);
function load(history = true): Saved {
  try {
    const value: Saved = JSON.parse(localStorage.getItem("mock:reviewCalibration") ?? "null") ?? { enabled: false, generation: 0, records: [] };
    if (typeof value.enabled !== "boolean" || !Number.isSafeInteger(value.generation) || value.generation < 0 || (history && !checkHistory(value.records))) throw storageError();
    return value;
  } catch { throw storageError(); }
}
const save = (value: Saved) => { localStorage.setItem("mock:reviewCalibration", JSON.stringify(value)); };
export function status(): CalibrationStatus {
  const state = load();
  const actions = { answered: 0, handed_to_agent: 0, dismissed: 0, none: 0 };
  for (const record of state.records) actions[record.action]++;
  return { enabled: state.enabled, generation: state.generation, records: state.records.length, created: state.records.filter(record => record.created).length, actions };
}
export function setEnabled(enabled: boolean) { const state = load(false); save({ ...state, enabled, generation: state.generation + 1 }); return status(); }
export function clear() { const state = load(false); save({ ...state, generation: state.generation + 1, records: [] }); return status(); }
export function append(generation: number, record: CalibrationRecord) {
  const state = load(false);
  if (state.enabled && state.generation === generation) {
    if (!isCalibrationRecord(record)) throw 'i18n:{"code":"err.calibration.invalid"}';
    if (!checkHistory(state.records)) throw storageError();
    // Project the same content-free fields as the native store; never retain caller extras.
    const { v, at, model, language, answers, suggested, action, created, latency_ms } = record;
    state.records.push({ v, at, model, language, answers: answers.map(({ id, outcome, confidence }) => ({ id, outcome, confidence })), suggested, action, created, latency_ms });
    save(state);
  }
}
export function exportCsv() {
  const fields: (keyof CalibrationRecord)[] = ["v", "at", "model", "language", "answers", "suggested", "action", "created", "latency_ms"];
  const rows = load().records.map(record => fields.map(field => {
    const value = record[field];
    const text = value === null ? "" : typeof value === "string" ? value : JSON.stringify(value);
    return `"${text.replace(/"/g, '""')}"`;
  }).join(","));
  return [fields.join(","), ...rows, ""].join("\r\n");
}
