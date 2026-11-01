---
type: Quality
title: "Usage preview and telemetry improvements"
description: "Reviewable local usage prototype and proposed measurement gaps by provider and shared boundary."
status: draft
---

# Usage preview and telemetry improvements

The reviewed Usage screen is implemented in Settings / Analytics.
Its current behavior and additive conversation coverage are specified
in the [telemetry contract](../contracts/telemetry.md). The production presentation
also runs in `/design-system.html?component=usage-history&state=ready&embed=1`.

The original [standalone HTML](../../telemetry-preview.html) remains a local visual
reference with synthetic data. It is not connected to the backend. The application
uses shared components and its i18n catalogs, respects the reviewed removals, and
retains export and confirmed history deletion. Remaining capture improvements
below are proposals; they do not promise native protocol support.

## General improvements

Priorities indicate proposed sequence, not a scheduled commitment.

| Priority | Improvement | Why and acceptance evidence |
| --- | --- | --- |
| P0 | Improve optional measurement diagnostics | Session token coverage and scope now exist. Field-specific cost/cache/model coverage still needs query metadata if later required for diagnostics. Keep reporting-count fractions out of the main screen. |
| P1 | Capture individual tools at the canonical boundary | Record opaque call identity, a safe tool identity, session/turn/execution relation, observed start/end, result outcome and output byte count. Native or MCP origin must be explicit; custom MCP identity needs a privacy-reviewed local mapping. Never persist arguments, commands, results or raw errors. Deduplicate repeated blocks and terminal events. Starts without ends remain incomplete. |
| P1 | Add tool ranking queries | Count calls, observed errors, duration distributions and returned bytes. Distinguish execution errors from denial/cancellation when evidence allows. Do not count failed calls as confirmed retries without a reliable relation. Verify background tools, duplicate results, missing starts and ambiguous turn attribution. |
| P1 | Add time and provider breakdowns | Add bounded daily buckets and provider filters in the backend. Model queries must keep selected and observed identities separate. Specify local-day boundaries, start-cohort attribution and coverage; test daylight-saving boundaries and a turn that ends outside its start period. |
| P2 | Add evidence-based optimization signals | Show context growth, cache read share, observed rebuilds/compactions, large tool results and repeated tool errors. Each signal links to underlying sessions and its evidence. Signals are investigation prompts, not measured waste or automatic savings claims. |
| P2 | Evaluate comparative cost estimates separately | Only if useful beyond token consumption. Version any price source and record its effective date, provider/model mapping and provenance. Keep estimates distinct from CLI-reported cost and actual billing; unknown prices and subscription costs cannot be synthesized. |

Tool calls do not provide an additive partition of model tokens. Model input can
reuse several tool results on multiple subsequent calls. Output bytes are exact
observed volume; tokenization would be an explicitly labeled estimate, not a
billable per-tool cost. Do not apportion a turn's tokens by tool frequency or size.

Origin tags such as action and delegation may overlap. They are filters over
conversation usage, not sources that can be added together. PR-related workspace
totals can overlap too. Neither relation proves task success or exclusive cost.

## Claude

**Current:** main-turn input/output, cache reads/writes, conditional CLI cost,
observed model rows, distinct model-call evidence and conditional whole-tree
coverage. Reasoning tokens are not available in this adapter.

1. **P0 — qualify cost/model measurements.** Session token coverage and
   main-agent versus whole-tree evidence are now exposed; cost/model coverage
   remains a separate diagnostic improvement. Keep resumed/reset/overlapping-child
   baselines conservative; do not make incomplete values look exact.
2. **P1 — connect tool facts.** Deduplicate streaming and completed assistant blocks
   by native tool identity before creating local opaque identities. Correlate
   results and independently identified children without counting parent totals twice.
3. **P2 — investigate reasoning and finer model attribution only where the native
   protocol supplies evidence.** Thinking text is not a reported reasoning-token
   count. A selected model is not proof of actual model use.

Evidence: existing adapter `telemetry_*` tests in
[`claude.rs`](../../src-tauri/src/claude.rs), plus fixtures for tool block/result
duplication, child completion after the main turn and resumed cost baselines.

## Codex

**Current:** input/output consumption from verified cumulative thread deltas,
cache reads and reasoning subsets, session/workspace aggregation. CLI cost,
observed model breakdown and reliable model-call counts remain unavailable.

1. **P0 — expose baseline gaps.** Distinguish unsupported fields from unknown
   consumption after resume/reset. Investigate an earlier native baseline read
   only if it can precede work reliably; never charge old thread totals to a new turn.
2. **P1 — connect tool facts.** Normalize native tool lifecycle signals at the
   adapter and capture through the shared boundary. Repeated status updates must
   not create extra calls; interrupted calls retain their observed status.
3. **P2 — investigate actual model identity and distinct model calls.** Implement
   only if native evidence identifies them at the measured scope. Do not use tool
   count or selected-model configuration as a proxy.
4. **P2 — consider optional cost estimates under the shared pricing proposal.**
   Missing CLI cost remains null until a separately identified source exists.

Evidence: existing counter-delta, reset, resume and duplicate-terminal tests in
[`protocols/src/codex.rs`](../../src-tauri/crates/protocols/src/codex.rs); extend
with lifecycle fixtures and boundary ordering when new capture is introduced.

## Antigravity

**Current:** deduplicated step consumption, including steps without text, and
verified fresh/same-process cumulative completion. Cache/thinking subsets depend
on native evidence. Cost, model breakdown and context capacity are unavailable;
interactive waits and independent child visibility are also limited.

1. **P0 — make partial coverage legible.** Preserve useful step totals while
   distinguishing unknown resumed baselines. Test tool-only and interrupted steps.
2. **P1 — connect tool facts.** Verify stable step/tool identity before deriving
   counts and durations. Do not infer timing from text order or fabricate child calls.
3. **P2 — investigate native context capacity, model identity, request/child
   signals and cost.** Each addition requires a provider signal and fixture;
   unsupported fields remain explicitly unavailable.

Evidence: existing telemetry normalization tests in
[`antigravity.rs`](../../src-tauri/src/antigravity.rs). Any claimed improvement
also needs a controlled native conformance run with the tested CLI version recorded.

## Cursor

**Current:** accepted by the local telemetry provider vocabulary and the common
conversation lifecycle. The ACP adapter does not normalize token consumption;
`usageTokens` is disabled. Account quota percentages are not historical tokens.

1. **P0 — keep unsupported usage explicit.** The UI must not display zero usage
   for Cursor or imply that its account quota measures session consumption.
2. **P1 — verify whether the installed ACP implementation exposes usage.** If it
   does, normalize scope, baseline, inclusive token categories and completion
   semantics before enabling usage capability. If not, retain unavailable status
   and document the protocol limitation; do not estimate from transcript length.
3. **P1 — connect observable tool lifecycle facts independently of tokens.** Tool
   counts can become useful even if consumption remains unavailable.

Evidence: [`cursor.rs`](../../src-tauri/crates/protocols/src/cursor.rs) and
[`agents.rs`](../../src-tauri/src/agents.rs). Add adapter fixtures and a native
conformance check before changing the support matrix or enabling the capability.

## Delivery and verification

Capture shared tool facts and adapt providers before adding optimization signals.
Protocol feasibility must precede promises of new provider fields.

New events or IPC fields require updates to the telemetry/conversation contracts,
the browser mock and typed registry, compatibility tests and an ADR where a
trade-off is introduced. The implemented screen adds only query metadata;
persisted events and provider capabilities remain unchanged.

Production mapping, controller, mock and native coverage tests are listed in the
[provider matrix](provider-matrix.md#local-telemetry-foundation). Browser evidence
covers session details during live updates, Escape/focus restoration, deletion
and narrow table navigation. Run browser scenarios with the English UI.
