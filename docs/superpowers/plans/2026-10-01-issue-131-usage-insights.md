# Local usage and context insights implementation plan

> **For agentic workers:** Use test-driven implementation and verification before completion. Independent adapter, query and presentation tasks have exclusive file ownership; integrate their shared contracts before the final checks.

**Goal:** Implement the required local usage, context and workspace/PR consumers from [issue 131](https://github.com/prometeucorp/prometeu/issues/131).

**Architecture:** Extend the existing private SQLite telemetry boundary from ADR 0059 instead of creating the proposed second JSONL ledger. Adapters provide canonical nullable measurements; the conversation carries optional turn usage, and explicit local queries restore native-transcript footers and aggregate workspace consumption. Presentation consumes capabilities and application-owned types only.

**Tech stack:** Rust/Tauri, SQLite, TypeScript, shared DOM Design System, Vitest and existing Rust/Playwright suites.

**Spec:** The required phases 1 and 2a–2c of issue 131, reconciled with the current [telemetry contract](../../contracts/telemetry.md) and [ADR 0059](../../decisions/0059-local-telemetry-foundation.md). Phase 2d (full history page) and phase 3 (OpenTelemetry export) remain explicitly later work in that issue.

## Global constraints

- Input includes cache subsets; output includes reasoning. Never add subsets to totals again.
- Unknown remains null. Show a CLI estimate only with a verified per-turn baseline; never sum legacy `costUsd`.
- Keep existing main-agent coverage explicit. Whole-tree/model attribution requires provider evidence; resets, restored spend and overlapping work must not be guessed.
- All local records remain content-free and privately stored. Existing history deletion, retention, capture ordering and generation suppression remain effective.
- Old transcripts/boards/events remain readable. Additive telemetry fields use defaults for old records.
- User-facing text is translated in English and Portuguese. Controls reuse `src/ui.ts`, `src/menu.ts` and shared tokens.
- Context actions are explicit and capability-driven. They neither change workspace stage nor invent agent status.
- A workspace total is displayed once. Cross-repository PR associations must not be summed as independent consumption. PR tenure needs observed branch and close-time evidence.
- Reuse the existing isolated worktree and preserve unrelated changes. No release or remote publication is part of this task.

## Review focus

1. Resumed/reset providers and duplicate terminals: never count restored spend or a cumulative sample twice.
2. Native Claude transcript replay: match the final assistant identity, never approximate by timestamp or ordinal.
3. Empty/partial history and measured zero: do not turn missing measurements into free work or hide measured zero.
4. Navigation, desk frames and in-flight reads: do not place a stale summary or new conversation into the wrong workspace.
5. Deletion during asynchronous calls and multiple repositories: do not restore erased records or double-count a workspace through PR joins.

## Shared interfaces

```ts
// Canonical completion extension; legacy costUsd remains readable but is not used for insights.
type CompletionExtension = {
  usage?: TelemetryMeasurement;
  messageId?: string;
};
type UsageGroup = {
  id: string;
  provider: string | null;
  turns: number;
  usage: TelemetryUsage;
};
// telemetry_insights({filter}) returns summary, usage, conversations, models,
// sources, overlapping origins and observed pullRequests. Context occupancy is not summed.
// telemetry_turns({conversation, messageIds}) returns matched rows only:
type TurnMeasurement = {
  messageId: string;
  durationMs: number | null;
  usage: TelemetryMeasurement;
};
```

### Task 1: Canonical measurements and provider evidence

**Files:** `src/conversation.ts`, `src/conversation.test.ts`, `src/agents.ts`, `src-tauri/src/agents.rs`, provider adapters and their fixtures/tests, `src-tauri/src/chat.rs`.

**Interfaces:** Produce the optional completion fields above and capabilities `usageTokens`, `usageCost`, `contextWindow`. Reuse `TelemetryMeasurement` rather than introduce another token taxonomy.

- [x] Write compatibility tests: old completions remain valid; valid optional usage survives; invalid optional measurements cannot break turn settlement; unsupported capabilities remain false.

```ts
expect(parseConversationEvent(oldCompletion)).not.toBeNull();
expect(parseConversationEvent({ ...oldCompletion, usage: measurement })?.type).toBe("turn.completed");
expect(measurement.usage.inputTokens! + measurement.usage.outputTokens!).toBe(12000);
```

- [x] Run `npx vitest run src/conversation.test.ts src/agents.test.ts` and the closest provider tests. Expected: new assertions fail until implemented.
- [x] Preserve normalized usage and final assistant identity at completion. Observe Claude main-call occupancy/window, Codex input occupancy/window, Antigravity recorded usage and deduplicated rebuild signals; test consecutive turns, resume, reset and duplicate terminals.
- [x] Run the same focused suites. Expected: all new and existing cases pass.

### Task 2: Local summaries, reply lookup and app-call attribution

**Files:** `src-tauri/src/telemetry.rs`, `src-tauri/src/telemetry/{capture,query,tests}.rs`, `src-tauri/src/naming.rs`, `src-tauri/src/plugins.rs`, GitHub relation capture where required.

**Interfaces:** Consume adapter completion identity/measurement. Produce `telemetry_insights` and bounded `telemetry_turns` queries. Persist a hashed reply association; native identifiers are not exported as arbitrary text. App calls have a closed `naming | plugin-maker` source, with no invented workspace for global plugin creation.

- [x] Add regression cases for grouped sums, observed models only, missing totals, replay lookup, old event defaults, app-call source, erasure generations and PR relations/tenure.

```rust
assert_eq!(insights.usage.input_tokens, Some(100));
assert_eq!(insights.usage.cache_read_tokens, Some(80));
assert_eq!(insights.usage.context_used, None);
assert_eq!(insights.summary.turns, 1);
```

- [x] Run `cd src-tauri && cargo test telemetry`. Expected: new behavior fails before implementation.
- [x] Implement snapshot queries using final/latest measurements without loading the event history in the frontend. Preserve health, missing measurements, independent app calls and existing deletion behavior.
- [x] Run focused telemetry, naming and plugin tests. Expected: pass, with exported records still excluding content and paths.

### Task 3: Turn footer, context controls and workspace summary

**Files:** `src/timeline.ts`, timeline/presentation tests, `src/chat.ts`, focused usage/context modules, `src/desk.ts`, `src/workspace.ts`, archive/finish consumers and relevant component styles/catalog.

**Interfaces:** Consume completion usage/capabilities and the two local queries. New-conversation callbacks capture the owning workspace ID. Shared components accept translated data and callbacks, without IPC ownership.

- [x] Test final-reply-only attachment, provider duration preference, inclusive token formatting, null/zero, stale reply lookups, gauge thresholds and unknown windows.

```ts
expect(contextBand(120000, 200000)).toBe("warn");
expect(contextBand(160000, 200000)).toBe("hot");
expect(contextBand(220000, 1000000)).toBe("warn");
```

- [x] Run focused Vitest suites. Expected: fail until the new rules exist.
- [x] Add the footer tooltip and a compact context meter with explicit Compact, Context report and New conversation actions. Preserve the draft, support desk frames, and hide unsupported values.
- [x] Add the workspace chip and grouped summary, including coverage, known CLI cost, agent/wait time, conversations, observed models, app sources and PR associations. Include the total in the existing finish/archive flow without adding an approval step.
- [x] Run focused tests. Add one browser interaction only if needed to prove popover keyboard/focus behavior and draft preservation; parsing and arithmetic stay in unit tests.

### Task 4: Typed integration, mock and documentation

**Files:** `src/telemetry.ts`, `src/ipc.ts`, `src/mock.ts`, `src/mock-telemetry.ts`, `src/i18n.{en,pt}.ts`, backend command registration, serialization fixtures, `ARCHITECTURE.md`, `README.md`, affected contracts/ADR/provider matrix and documentation index.

**Interfaces:** Keep native command registration, typed IPC and browser handlers in parity. Mock realistic measurements without invoking a provider.

- [x] Test mock summaries and lookup using the same snapshot semantics, inclusive totals, unknowns and legacy data as native queries.
- [x] Implement registry, catalog and i18n integration and update serialization fixtures intentionally with `npm run contracts:update`.
- [x] Update ADR 0059 and the affected contracts with the implemented decisions, sharing scope, replay lookup, compatibility and measurement limits. Link the new tests and this plan.
- [x] Run `npm run docs:check`, `npm run architecture:check`, `npm run typecheck`, and `npm run test:contracts`. Expected: pass.

### Task 5: Integrated verification and review

**Files:** Final diff and this plan.

- [x] Run `npm run check`; inspect the result of every stage and address failures caused by the change.
- [x] Review replay, reset, unknowns, concurrency, deletion and PR attribution against the review-focus list. Review the actual diff, not agent summaries alone.
- [x] Record completed checks and any remaining provider evidence limits below. Do not claim live CLI or native webview conformance from browser mocks.

## Execution record

Initial analysis: existing SQLite telemetry supersedes the issue's proposed JSONL ledger. The workspace was clean and already isolated. Implementation is authorized by the user's request to plan and apply.


Implementation decisions:

- Reused ADR 0059's private SQLite ledger, retention, export and deletion. Added
  exact conversation-scoped reply hashes, source/origin attribution and snapshot
  queries; no second ledger, raw reply identifiers or transcript backfill.
- Resumed or reset cumulative counters without a verified baseline stay partial.
  A persisted previous terminal cannot prove what happened during external resumed
  work, so it is not used to manufacture a delta. Fresh and same-process baselines
  permit complete deltas. Legacy top-level cost is never summed or displayed.
- PR tenure requires observed branch identity, valid lifecycle dates and a fully
  captured history snapshot queried after the turn. Insufficient evidence remains
  a related association. Multiple repository associations are not added together.
- Canonical optional usage remains part of the conversation's existing encrypted
  sharing scope. The private ledger, source groups and summary queries stay local.
- Added English/Portuguese controls, reply tooltips, draft-preserving context
  actions, workspace/PR summaries, source breakdowns in Settings, and totals in
  the existing archive/finish flow. Full history UI and OpenTelemetry remain the
  later phases identified by the issue.

Independent review found and drove regressions for Antigravity's zeroed error
result, reopened context after compaction, and late app/PR capture invalidation.
The browser check also fixed WebKit keyboard focus and constrained the header
chip at the native minimum width. New browser coverage is limited to the shared
popover's focus/draft/owning-workspace behavior; arithmetic and lifecycle rules
remain pure or native tests.

First integrated `npm run check` passed: documentation, architecture, formatting,
Desktop/mobile builds, contracts, release checks, 628 web tests, 520 native tests
(8 existing ignored integrations), 181 browser scenarios and warning-free Clippy.
Final `npm run check` passed after all review fixes: 635 web tests across 78 files,
523 native tests (8 existing ignored integrations), 181 browser scenarios in
Chromium/WebKit, and every documentation, architecture, format, build, contract,
release and Clippy stage. The event producer regression confirms commit visibility,
lock release and a null payload; consumer regressions cover refresh without a
board change, coalescing and stale reads. The independent final review reported
no remaining actionable findings. `git diff --check` is clean.

Evidence limits: Claude's new deterministic fixture is explicitly synthetic and
based on the documented CLI contract. Existing Antigravity recordings and Codex
protocol fixtures remain covered. No live installed-CLI or native webview
conformance is claimed. Unknown cost/model/call measurements stay null rather
than being extrapolated from another provider.
