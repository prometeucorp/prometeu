/// Application-owned evaluation port shared by features and shells (ADR 0058). Features depend on
/// these closed types, never on a vendor payload; the TypeSafe adapter and the API key stay in the
/// Rust backend. This module performs no I/O.

export type EvaluationQuestion = { id: string; prompt: string; outcomes: string[] };
export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };
export type EvaluationRequest = { context: string | { [key: string]: JsonValue }; questions: EvaluationQuestion[] };
export type EvaluationAnswer = { id: string; outcome: string; confidence: number };
export type EvaluationResult = { answers: EvaluationAnswer[]; model?: string | null };

/// Only configuration and availability cross IPC; `problem` is an i18n-coded configuration error.
export type EvaluationStatus = { configured: boolean; enabled: boolean; problem: string | null };

export type EvaluationErrorCode = "disabled" | "auth" | "rate_limited" | "unavailable" | "malformed" | "invalid" | "stale";
const CODES: readonly EvaluationErrorCode[] = ["disabled", "auth", "rate_limited", "unavailable", "malformed", "invalid", "stale"];

export type EvaluationPort = { evaluate(request: EvaluationRequest): Promise<EvaluationResult> };

/// Recover the application code from a rejected evaluation. Unknown failures count as unavailable so
/// the caller never treats them as a result.
export function errorCode(error: unknown): EvaluationErrorCode {
  const text = typeof error === "string" ? error : String(error);
  const match = /"code":"err\.evaluation\.([a-z_]+)"/.exec(text);
  const code = match?.[1] as EvaluationErrorCode | undefined;
  return code && CODES.includes(code) ? code : "unavailable";
}
