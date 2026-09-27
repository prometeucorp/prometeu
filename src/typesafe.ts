import type { EvaluationPort, EvaluationStatus } from "./evaluation";
import { invoke } from "./ipc";
import type { CalibrationPort, CalibrationStatus } from "./review-calibration";

/// Desktop shell of the optional TypeSafe integration: its IPC-backed evaluation port and the
/// configuration status shared by Settings and the launcher. The key is sent once to the backend
/// and never read back.

export const port: EvaluationPort = { async evaluate(request) {
  const started = epoch;
  const result = await invoke("context_evaluate", { request });
  if (started === epoch) {
    observedModel = result.model ?? null;
    notify(false);
  }
  return result;
} };

const DISABLED: EvaluationStatus = { configured: false, enabled: false, problem: null };
let status: EvaluationStatus = DISABLED;
/// Bumped on every configuration change; consumers bind pending work to it.
let epoch = 0;
const listeners = new Set<(configuration: boolean) => void>();
let observedModel: string | null | undefined;
export const lastModel = () => observedModel;
const notify = (configuration: boolean) => { for (const listener of [...listeners]) listener(configuration); };

let calibration: CalibrationStatus = { enabled: false, generation: 0, records: 0, created: 0, actions: { answered: 0, handed_to_agent: 0, dismissed: 0, none: 0 } };
let calibrationProblem: string | null = null;
let calibrationRevision = 0;
let consentRevision = 0;
let consentChanging = false;
export const calibrationStatus = () => calibration;
export const calibrationError = () => calibrationProblem;

async function calibrationChange(work: () => Promise<CalibrationStatus>) {
  const revision = ++calibrationRevision;
  try {
    const next = await work();
    if (revision === calibrationRevision) { calibration = next; calibrationProblem = null; notify(false); }
  } catch (error) {
    if (revision === calibrationRevision) { calibrationProblem = String(error); notify(false); }
    throw error;
  }
}
async function calibrationMutation(work: () => Promise<CalibrationStatus>) {
  const revision = ++consentRevision;
  consentChanging = true;
  try { return await calibrationChange(work); }
  finally { if (revision === consentRevision) consentChanging = false; }
}
export const refreshCalibration = () => calibrationChange(() => invoke("review_calibration_status"));
export const setCalibrationEnabled = (enabled: boolean) => calibrationMutation(() => invoke("review_calibration_set_enabled", { enabled }));
export const clearCalibration = () => calibrationMutation(() => invoke("review_calibration_clear"));
export const calibrationPort: CalibrationPort = {
  consent: () => calibration.enabled && !calibrationProblem && !consentChanging ? calibration.generation : null,
  append(generation, record) {
    const revision = consentRevision;
    const current = () => revision === consentRevision && !consentChanging && generation === calibration.generation;
    void invoke("review_calibration_append", { generation, record }).then(() => {
      if (current()) void refreshCalibration().catch(() => {});
    }, error => {
      if (!current()) return;
      calibrationProblem = String(error);
      notify(false);
    });
  },
};

export const current = () => status;
export const currentEpoch = () => epoch;
export const available = () => status.configured && status.enabled;

export function onChange(listener: (configuration: boolean) => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function publish(next: EvaluationStatus) {
  epoch++;
  status = next;
  notify(true);
  return status;
}

/// A missing or older backend keeps the integration disabled.
export async function refresh() {
  void refreshCalibration().catch(() => {});
  try { return publish(await invoke("typesafe_status")); }
  catch { return publish(DISABLED); }
}

export async function saveKey(key: string) { return publish(await invoke("typesafe_save_key", { key })); }
export async function removeKey() { return publish(await invoke("typesafe_remove_key")); }
export async function setEnabled(enabled: boolean) { return publish(await invoke("typesafe_set_enabled", { enabled })); }
