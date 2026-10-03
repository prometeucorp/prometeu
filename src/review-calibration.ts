import type { EvaluationAnswer } from "./evaluation";
import type { Topic } from "./context-review";
import type { ReviewLanguage } from "./review-policy";

export type CalibrationAction = "answered" | "handed_to_agent" | "dismissed" | "none";
export type CalibrationRecord = {
  v: 1;
  at: number;
  model: string | null;
  language: ReviewLanguage;
  answers: EvaluationAnswer[];
  suggested: Topic[];
  /// First explicit action in this review, not evidence of a useful outcome.
  action: CalibrationAction;
  created: boolean;
  latency_ms: number;
};
export type CalibrationStatus = {
  enabled: boolean;
  generation: number;
  records: number;
  created: number;
  actions: Record<CalibrationAction, number>;
};
export type CalibrationPort = {
  consent(): number | null;
  append(generation: number, record: CalibrationRecord): void;
};

/// The supported model family is a closed category. Arbitrary provider-supplied names must not
/// become a text field in local history, even when they happen to end in a version number.
export const recordedModel = (model: string | null | undefined): string | null =>
  model && /^jev-\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(model) ? model : null;

const presence = ["present", "ambiguous", "absent", "uninspected", "not_applicable"];
const resolver = ["person", "agent", "unclear"];
const outcomes: Record<string, readonly string[]> = {
  task_kind: ["bug_fix", "feature", "investigation", "other"],
  business_rule: presence, expected_behavior: presence, reproduction: presence,
  business_rule_resolver: resolver, expected_behavior_resolver: resolver, reproduction_resolver: resolver,
  business_rule_kind: ["existing_records", "permissions", "failure_handling", "scope", "other"],
};
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const fields = (value: Record<string, unknown>, names: string[]) => Object.keys(value).length === names.length && Object.keys(value).every(key => names.includes(key));
const natural = (value: unknown) => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

/// Browser-backend validation mirrors the native content-free persistence contract.
export function isCalibrationRecord(value: unknown): value is CalibrationRecord {
  if (!object(value) || !fields(value, ["v", "at", "model", "language", "answers", "suggested", "action", "created", "latency_ms"])) return false;
  if (value.v !== 1 || !natural(value.at) || !natural(value.latency_ms) || typeof value.created !== "boolean") return false;
  if (!(value.model === null || (typeof value.model === "string" && recordedModel(value.model) === value.model))) return false;
  if (typeof value.language !== "string" || !["en", "pt", "other"].includes(value.language)) return false;
  if (typeof value.action !== "string" || !["answered", "handed_to_agent", "dismissed", "none"].includes(value.action)) return false;
  if (!Array.isArray(value.suggested) || value.suggested.length > 2 || new Set(value.suggested).size !== value.suggested.length
    || !value.suggested.every(topic => ["business_rule", "expected_behavior", "reproduction"].includes(topic))) return false;
  if (!Array.isArray(value.answers) || value.answers.length > 8) return false;
  const ids = new Set<string>();
  return value.answers.every(answer => {
    if (!object(answer) || !fields(answer, ["id", "outcome", "confidence"]) || typeof answer.id !== "string" || typeof answer.outcome !== "string") return false;
    if (!Object.prototype.hasOwnProperty.call(outcomes, answer.id) || !outcomes[answer.id].includes(answer.outcome) || ids.has(answer.id)) return false;
    ids.add(answer.id);
    return typeof answer.confidence === "number" && Number.isFinite(answer.confidence) && answer.confidence >= 0 && answer.confidence <= 1;
  });
}
