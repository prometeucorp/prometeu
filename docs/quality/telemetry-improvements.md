---
type: Quality
title: "Usage preview and telemetry improvements"
description: "A local usage prototype that you can review, and the measurement gaps that we propose for each provider and shared limit."
status: draft
---

# Usage preview and telemetry improvements

The reviewed Usage screen is in Settings / Analytics. The
[telemetry contract](../contracts/telemetry.md) specifies its current behavior
and the added conversation coverage. The production presentation also operates
in `/design-system.html?component=usage-history&state=ready&embed=1`.

The initial [standalone HTML](../../telemetry-preview.html) stays a local visual
reference with synthetic data. It is not connected to the backend. The
application uses shared components and its i18n catalogs. It obeys the reviewed
removals, and it keeps the export and the confirmed deletion of the history. The
capture improvements below are proposals. They do not promise that the native
protocols can give these data.

## General improvements

The priorities give a proposed sequence. They are not a schedule.

| Priority | Improvement | Why and acceptance evidence |
| --- | --- | --- |
| P0 | Improve optional measurement diagnostics | The session token coverage and scope are available at this time. If diagnostics need the coverage of the cost, cache and model fields, the queries must add metadata. Keep the fractions of report counts out of the primary screen. |
| P1 | Capture individual tools at the canonical boundary | Record an opaque call identity and a safe tool identity. Record the relation to the session, the turn and the execution. Record the start and end that you saw, the result and the number of output bytes. The native or MCP origin must be explicit. A custom MCP identity must have a local mapping with a privacy review. Do not store arguments, commands, results or raw errors at any time. Remove the duplicates of repeated blocks and terminal events. A start without an end stays incomplete. |
| P1 | Add tool ranking queries | Count the calls, the errors that you saw, the distribution of durations and the returned bytes. If the evidence lets you, make a difference between execution errors and a denial or a cancellation. Do not count calls with a failure as confirmed retries without a reliable relation. Examine background tools, duplicate results, missing starts and turns that you cannot connect to one cause. |
| P1 | Add time and provider breakdowns | Add daily buckets with limits and provider filters in the backend. Model queries must keep the selected identities and the identities that you saw separate. Specify the limits of a local day, the connection of a turn to its start period and the coverage. Add tests for the changes for daylight saving time and for a turn that ends after its start period. |
| P2 | Add evidence-based optimization signals | Show the increase of context, the share of cache reads, the rebuilds and compactions that you saw, large tool results and repeated tool errors. Each signal has a link to the related sessions and to its evidence. The signals tell you where to look. They do not measure waste or claim automatic savings. |
| P2 | Evaluate comparative cost estimates separately | Do this only if it gives more than the token consumption. Give a version to each price source. Record its effective date, its provider and model mapping and its origin. Keep the estimates different from the cost that the CLI reports and from the real invoice. You cannot calculate unknown prices and subscription costs. |

The tool calls do not divide the model tokens into parts that you can add. The
model input can use many tool results again in many subsequent calls. The output
bytes are the accurate volume that we saw. A token count for each tool must be
an estimate with an explicit label. It is not a cost for each tool that a
provider can charge. Do not divide the tokens of a turn by the frequency or the
dimension of each tool.

Origin tags, for example action and delegation, can overlap. They are filters on
the conversation usage. You cannot add them together as sources. The workspace
totals for pull requests can also overlap. These relations do not show that a
task was successful or that a cost belongs only to one item.

## Claude

**Current:** these data are available:

- The input and output of the main turn.
- Cache reads and writes.
- The CLI cost, when the CLI gives it.
- The model rows that we saw.
- The evidence of different model calls.
- The coverage of the full tree, when the CLI gives it.

This adapter does not give reasoning tokens.

1. **P0 — qualify the cost and model values.** The session token coverage is
   available at this time. The evidence for the main agent and for the full
   tree is also available. The cost and model coverage stays a different
   diagnostic improvement. Keep the baselines conservative after a resume, a
   reset or a child that overlaps. Do not show incomplete values as accurate
   values.
2. **P1 — connect tool data.** Remove the duplicates of streamed and completed
   assistant blocks with the native tool identity. Do this before you make local
   opaque identities. Connect the results and the children that have their own
   identity. Do not count the parent totals two times.
3. **P2 — examine reasoning and model data only with native evidence.**
   Thinking text is not a count of reasoning tokens. A selected model does not
   show which model the agent used.

Evidence: the current `telemetry_*` adapter tests in
[`claude.rs`](../../src-tauri/src/claude.rs). Also add fixtures for duplicate
tool blocks and results, a child that completes after the main turn and the
cost baselines after a resume.

## Codex

**Current:** the input and output consumption from verified cumulative thread
deltas, the cache reads and reasoning subsets, and the totals for each session
and workspace. The CLI cost, the breakdown of the models that we saw and reliable
counts of model calls continue to be not available.

1. **P0 — show baseline gaps.** Show which fields the provider does not give.
   Show separately the unknown consumption after a resume or a reset.
   Investigate an earlier read of the native baseline only if it can occur
   reliably before the work. Do not charge old thread totals to a new turn at
   any time.
2. **P1 — connect tool data.** Normalize the native signals of the tool
   lifecycle in the adapter. Capture them through the shared limit. Repeated
   status updates must not make more calls. Calls that stop before the end keep
   the status that we saw.
3. **P2 — examine the model identity and the model calls.** Use only native
   evidence at the measured scope. Do not use the
   tool count or the configuration of the selected model as an alternative.
4. **P2 — examine optional cost estimates.** Use the shared price proposal.
   The missing CLI cost stays null until there is a different, identified
   source.

Evidence: the current tests for counter deltas, reset, resume and duplicate
terminal events in
[`protocols/src/codex.rs`](../../src-tauri/crates/protocols/src/codex.rs). When
you add a new capture, add lifecycle fixtures and tests of the order at the
limit.

## Antigravity

**Current:** the consumption of each step without duplicates, also for steps
without text. The cumulative completion is verified in a new process or in the
same process. The cache and thinking subsets are available only when native
evidence is available. The cost, the model breakdown and the context capacity
are not available. The interactive waits and the visibility of independent
child processes are also limited.

1. **P0 — show partial coverage clearly.** Keep the useful step totals. Show
   when the baseline of a resume is unknown. Add tests for steps with only tools and steps
   that stop before the end.
2. **P1 — connect tool data.** Make sure that the step and tool identities are
   stable. Then calculate counts and durations. Do not calculate the timing
   from the order of the text. Do not make child calls that the provider did not
   report.
3. **P2 — examine native context, model, request, child and cost signals.**
   Add only provider signals, with a fixture.
   Fields that the provider does not give stay explicitly not available.

Evidence: the current telemetry normalization tests in
[`antigravity.rs`](../../src-tauri/src/antigravity.rs). Each claimed
improvement must also have a controlled native conformance run. Record the CLI
version of that run.

## Cursor

**Current:** the local telemetry provider vocabulary and the common
conversation lifecycle accept Cursor. The ACP adapter does not normalize the
token consumption. `usageTokens` is disabled. The account quota percentages are
not historical tokens.

1. **P0 — show clearly that usage is not available.** The interface must not show zero
   usage for Cursor. It must not tell that the account quota measures the
   consumption of a session.
2. **P1 — find if the installed ACP gives usage.** If yes, normalize the scope,
   baseline, token categories and completion. Do this before you enable the usage
   capability. If it does not give usage, keep the status not available and
   record the protocol limit. Do not calculate usage from the length of the
   transcript.
3. **P1 — connect tool lifecycle data without tokens.** Tool counts are useful
   without consumption data.

Evidence: [`cursor.rs`](../../src-tauri/crates/protocols/src/cursor.rs) and
[`agents.rs`](../../src-tauri/src/agents.rs). Before you change the support
matrix or enable the capability, add adapter fixtures and a native conformance
check.

## Delivery and verification

Capture the shared tool facts and change the providers before you add
optimization signals. Make sure that a protocol can give new provider fields
before you promise these fields.

New events or IPC fields make these changes necessary:

- Updates to the telemetry and conversation contracts.
- Updates to the browser mock and the typed registry.
- Compatibility tests.
- An ADR, if the change has a trade-off.

The implemented screen adds only query metadata. The stored events and the
provider capabilities do not change.

The [provider matrix](provider-matrix.md#local-telemetry-foundation) lists the
tests for the production mapping, the controller, the mock and the native
coverage. The browser evidence includes the session details during live
updates, Escape and focus restoration, deletion and table navigation in a narrow
window. Run the browser scenarios with the English interface.
