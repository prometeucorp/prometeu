# Review calibration implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task by task.

**Goal:** Implement [issue #145](https://github.com/prometeucorp/prometeu/issues/145) on updated `origin/main`, making review decisions attributable and local calibration explicitly opt-in.

**Architecture:** Keep model transport in the TypeSafe adapter, language and selection policy in the pure review core, and calibration persistence in a separate Rust module. The launcher reports actual workspace creation through its existing composition callback. Calibration is independent of the API key and general telemetry.

**Tech stack:** Rust/Tauri, TypeScript, Vitest, existing shared DOM controls.

**Spec:** Issue #145 and [ADR 0058](../../decisions/0058-optional-context-evaluation.md).

## Global constraints

- Pin `jev-1.13.0`; missing or different effective models abstain.
- Preserve English thresholds: kind 0.7, presence 0.8, resolver 0.7, rule kind 0.6.
- Portuguese: 0.75, 0.85, 0.75, 0.65. Unknown/mixed/empty drafts: 0.8, 0.9, 0.8, 0.7. These remain hypotheses, not claims of measured quality.
- Detect only the draft language deterministically; ambiguous text uses the conservative default, independent of UI language and issue text.
- Structured JSON context stays within 22 KiB including JSON escaping; the port remains compatible with old string contexts.
- Opt-in is separate, defaults off, and never stores task content, names, paths, credentials or probabilities. Disabling or clearing invalidates unfinished records.
- Keep eight closed questions, explicit review, stale-result cancellation, localized copy, and shared controls. No new browser scenario.

## Review focus

- JSON escaping and Unicode must not exceed either context bound.
- Missing model identity must never silently inherit the requested model.
- Clear/disable during a review must not let a late completion recreate history.
- Failed workspace creation must never be counted as created.
- Arbitrary answer/model strings must not become a content-storage channel.

## Task 1: Model identity, language policy and structured state

Files: `src-tauri/src/evaluation.rs`, `src-tauri/src/typesafe.rs`, `src/evaluation.ts`, `src/context-review.ts`, `src/context-review.test.ts`, new `src/review-policy.ts`.

Interface: `EvaluationResult { answers, model? }`; context accepts a string or JSON object. `select(result, context)` abstains on unknown model. Result views retain model identity.

- [x] Add regressions for model mismatch/missing identity, language thresholds, escaped Unicode budgets and quoted issue instructions; run focused tests and observe failures.
- [x] Pin and parse the model, preserve the additive result field, implement deterministic policy and bounded structured context.
- [x] Run `npm run test:web -- src/context-review.test.ts` and focused Rust evaluation/adapter tests.

## Task 2: Private calibration persistence and lifecycle

Files: new `src-tauri/src/review_calibration.rs`, `src/review-calibration.ts`; update Rust registration, typed IPC, browser mock and review-session tests.

Interface: status/enable/append/export/clear commands; consent generation travels with final records and is checked under the storage lock. The review accumulates only completed current evaluations, records the first explicit action, and finalizes exactly once after launcher cancellation or creation outcome.

- [x] Write tests for default-off, private permissions, closed content-free schema, CSV, clear/disable races, first action, deduplication and failed creation.
- [x] Implement private config and append-only JSONL, summary, CSV export and generation invalidation. Wire the same contract into the mock.
- [x] Run focused Rust calibration tests, review-session tests and IPC parity tests.

## Task 3: Settings, launcher composition and documentation

Files: `src/typesafe.ts`, `src/typesafe-settings.ts`, `src/context-review-view.ts`, `src/launcher.ts`, `src/main.ts`, both i18n catalogs, evaluation contract, ADR 0058 and provider matrix.

Interface: model diagnostics do not advance the configuration epoch; Settings shows effective model and calibration-pending notice. `go(draft)` resolves the actual workspace creation result. Calibration errors stay visible in Settings and never block creation.

- [x] Wire model diagnostics and localized opt-in, summary, CSV export and Clear controls using existing primitives.
- [x] Bind final calibration records to actual creation success and preserve existing stale-result behavior.
- [x] Write the predeclared measurement plan: separate language/model cohorts, minimum sample, objective clarification outcomes, effort/latency guardrails and a decision rule; record limitations of content-free records.
- [x] Update contracts, compatibility details, provider matrix and documentation index.
- [x] Run `npm run check`; inspect failures, fix task regressions, and report environmental limits precisely.
- [x] Obtain an independent whole-branch review and address material findings; preserve the requested implementation branch.

## Execution evidence

- Branch: `feat/145-review-calibration`, based on fetched `origin/main` at `115df1e`.
- Baseline: all 27 existing context-review tests passed.
- Vendor docs checked on 2026-09-27: pinned version, effective model identity and JSON state are supported.

- Focused evidence: 36 review tests, 4 shell tests, 3 browser calibration contract tests, port/adapter compatibility and 5 calibration storage tests cover the new boundaries.

- Independent review found an arbitrary-model-name storage channel and browser schema divergence. Regressions reproduced both before the fix; recorded model IDs now accept only bounded Jev versions, and native/browser persistence enforce the same closed schema. Live quality measurement remains explicitly pending under ADR 0058.

- Validation passed: documentation links and index, architecture rules, Rust formatting, desktop/mobile builds, type checks, release checks, 570 web tests, 485 Rust tests (7 ignored), and Clippy with warnings denied.
- Full `npm run check` attempts stopped at intermittent browser failures. Three existing Settings scenarios opened the screen before startup navigation finished; they now wait for the initial desk, matching an existing readiness guard. An unrelated WebKit draft-restoration failure did not recur across three isolated repetitions, and a relay WebSocket shutdown timeout from an earlier parallel test run did not recur in subsequent complete web suites. These observations do not establish the cause of either intermittent failure.
- Final browser validation: all 179 existing scenarios passed with `npm run test:e2e -- --workers=2`, followed by a successful `npm run lint:rust`. The default-concurrency `npm run check` is not claimed as passing.
