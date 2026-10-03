# ADR 0058 — Optional context evaluation behind an application-owned port

Date: 2026-09-24
Status: Accepted

## Context

A task can start with an unresolved product decision or missing information
that changes the implementation. Discovering that after the agent has started
costs clarification turns and rework. TypeSafe can classify a text context and
answer several narrow, closed questions in one call with a confidence per
answer. Prometeu runs local CLIs and has no model of its own; any external
evaluation is a new trust boundary: it needs the person's own credential,
sends their draft off the machine and can fail independently of the app.

The first consumer is a missing-context review in the launcher for new local
workspaces. Other bounded classification features may follow, so the boundary
must not be shaped around one vendor payload or one screen.

## Options considered

1. **Call TypeSafe from the launcher.** Smallest change, but the key would
   reach the webview, vendor payloads would reach the presentation, and a second
   feature would copy the transport and error handling.
2. **A new conversational provider in the agent catalog.** Reuses accounts and
   the runtime port, but evaluation is not a conversation, has no process and
   must stay independent of the chosen coding agent.
3. **An application-owned evaluation port with one HTTP adapter at the edge.**
   The feature asks closed questions and applies its own rules; the adapter owns
   the credential, transport, retries, response validation and error
   translation.
4. **A general provider marketplace or orchestration layer.** Premature: one
   adapter and one consumer exist.

## Decision

Adopt option 3.

- `src-tauri/src/evaluation.rs` owns the port: `EvaluationRequest` (bounded
  text or structured JSON context plus at most eight closed questions), answers with outcome,
  confidence and optional effective model identity, request and response validation against the closed sets, the
  configuration generation, read together with the credential, that
  invalidates results started under an older configuration, and the
  application error codes `disabled`, `auth`, `rate_limited`, `unavailable`,
  `malformed`, `invalid` and `stale`. It has a fake for tests and a disabled
  path that makes no call.
- `src-tauri/src/typesafe.rs` is the only place that knows TypeSafe: the private
  credential file, the explicit enable flag, the HTTP adapter with timeouts and
  bounded retries, and the translation of vendor statuses into the codes above.
- `src/evaluation.ts` mirrors the port for the frontend without I/O;
  `src/typesafe.ts` is the desktop shell (IPC port and configuration status).
- The missing-context review (`src/context-review.ts`) is a pure consumer: it
  builds the context, asks its own closed questions, and selects a question from
  an app-owned localized catalog with conservative thresholds and an explicit
  no-suggestion result. It never generates or rewrites text.
- The integration starts disabled. Saving a key never enables it; removing the
  key disables it. Only an explicit **Review request** action sends context.
  Creating workspaces, sending prompts and running agents never depend on it,
  and a failure never falls back to another service or provider.

The adapter uses TypeSafe's documented System One HTTP API pinned to `jev-1.13.0`.
Its request and response shape stays isolated in `typesafe.rs::wire`, described
in the [contract](../contracts/context-evaluation.md#typesafe-system-one-wire-shape).
The endpoint origin can be overridden with `PROMETEU_TYPESAFE_URL` (an origin
only: HTTPS, or loopback HTTP for local testing). A wire correction changes
only that module and its fixtures.

### Version, language and source boundaries

The review policy separately names its baseline model in `src/review-policy.ts`.
The response's effective identity crosses the port and appears with the result.
Missing identity or a different version produces `uncertain`, without suggestions;
Settings displays the observed version and a calibration-pending notice until a
known version answers. This diagnostic is session-local and does not change the
configuration epoch. Pinning trades automatic vendor improvements for reviewable
changes. Updating only the adapter pin cannot silently approve new thresholds.

A small deterministic vocabulary detects English or Portuguese from the person's
draft. It does not read the issue or the interface language. Ambiguous, mixed,
empty and unsupported text uses the conservative default. This avoids a UI
language preference lowering thresholds for a draft in another language; a full
language-detection dependency is unnecessary for two supported cohorts. False
negatives select stricter thresholds. The table is:

| Draft language | Task kind | Gap presence | Person resolver | Rule subtype |
| --- | --- | --- | --- | --- |
| English | 0.70 | 0.80 | 0.70 | 0.60 |
| Portuguese | 0.75 | 0.85 | 0.75 | 0.65 |
| Other/uncertain | 0.80 | 0.90 | 0.80 | 0.70 |

English preserves the original policy. Portuguese and the default start more
conservatively; these values are hypotheses, not empirical claims of calibration.
The existing English/Portuguese fixtures preserve their expected selections.

The context is now JSON: `requester.draft`, `third_party.issue`, project metadata
and uninspected attachment metadata. Every question identifies issue text as
quoted third-party data, never instructions. Clipping counts serialized UTF-8
bytes, including JSON escaping. This separation reduces accidental instruction
mixing; it is not a guarantee against adversarial influence on a classifier.

### Local evidence and predeclared measurement plan

Calibration has its own opt-in, off by default, independent of the integration
and the API key. `review_calibration.rs` stores only closed answer categories,
confidence, version, detected language, shown topics, the first explicit action,
request latency and whether workspace creation succeeded. It has no transcript,
workspace identifier, draft, issue content, names, keys or paths. Full vendor
probabilities are deliberately excluded: the current decision uses the chosen
outcome/confidence, and storing the vendor distribution would widen the contract
without a predeclared analysis that needs it. A future calibration-curve study
can propose that addition explicitly.

Completed current reviews are buffered until the launcher closes or its creation
call settles. Each produces one record, including `none` for no action. Clicking
Answer records insertion of the editable clarification block, not proof it was
filled or useful. Multiple actions use the first action; shown topics include
at most two. Cancellation/creation failure records `created: false`. Stale,
failed and unfinished evaluations are excluded. Closing the process before
finalization loses these in-memory records; no draft recovery journal is added.
Disable and Clear advance a persisted consent generation, rejecting older
records even after re-enabling. Storage failure never blocks creation. Settings
shows counts, CSV export, Clear and failures; no records leave the Mac except by
the person's explicit export.

Clear uses a persisted in-progress marker across its consent and history files.
It blocks collection and export until both writes complete, preserving the
deletion boundary after a partial failure or process restart. Retrying Clear is
the explicit recovery action; changing consent cannot bypass it. Older consent
files default the additive marker to false. Late frontend append replies are
bound to the consent revision, so they cannot undo a later Clear's recovery.

The native process caches validated summary counters against history metadata
(identity, size and modification/change timestamps), updating them after each
successful append. This avoids quadratic parsing as opted-in history grows
without trusting a changed or damaged file. Cache loss on restart causes one
fresh validation; CSV export remains a complete scan. No second persisted index
or additional migration is needed.

Before inspecting outcomes or tuning thresholds, use this protocol:

1. Study English and Portuguese separately, with the exact model version fixed.
   Recruit consenting local participants and preassign eligible bug-fix/feature
   tasks to ordinary submission or an invitation to explicitly review, balanced
   by task kind. Never invoke review automatically. Keep assignment even if the
   invitation is declined; record that adherence separately in the study sheet.
2. Collect at least **100 created workspaces per arm per language**. An explicit,
   local manual audit records only two yes/no signals: whether the agent asks
   the person a task-clarification question in its first three assistant turns,
   and whether the person adds a clarification block within 30 minutes of
   creation. Administrative messages do not count. Their union is the primary
   endpoint. Also time the person's extra pre-creation effort and count aborted
   creations. Review latency is available in the calibration records.
3. Before changing a language's thresholds, require at least a **10 percentage
   point reduction** in the primary endpoint and a 95% confidence interval for
   the review-minus-ordinary risk difference entirely below zero. Guardrails:
   median extra effort at most 15 seconds, 90th-percentile review latency at most
   10 seconds, and no increase above 5 percentage points in abandoned creations.
   If sample size or any criterion fails, keep the current row. Do not repeatedly
   inspect partial samples and stop on the first favorable result.
4. Develop a candidate on that completed cohort and repeat the same fixed-size
   protocol on a fresh holdout before adopting it. Never mix model versions or
   languages to meet the sample minimum. A version bump repeats the protocol;
   it does not inherit evidence from an alias or a previous version.

The CSV alone cannot establish these outcomes: it intentionally cannot join to
transcripts or identify ordinary submissions. The manual study sheet contains
only assignment, language/model, task kind, boolean outcomes and timings; it
does not copy text or identifiers and is outside automatic collection. Clicks,
answers and creation rates in Settings are descriptive evidence, not proof of
avoided rework. Unknown-language rows remain conservative pending a separate
predeclared study.

## Consequences

- The feature and the foundation are tested separately: the port with a fake,
  the adapter against a local HTTP server with synthetic responses, and the
  review rules with a fake port in TypeScript.
- The key stays in a private backend file and never crosses IPC, the board, the
  relay, transcripts, logs or error messages.
- A second bounded classification feature can reuse the port and the adapter by
  supplying its own closed questions and selection rules.
- Thresholds are hypotheses. Suggestion usefulness, avoided rework, extra user
  effort and latency must be measured against ordinary submission; a clicked
  suggestion is not evidence of better outcomes.
- The wire shape was checked against the public API and live Choice requests,
  including one with eight questions, on 2026-09-24. Suggestion quality still
  needs validation with real tasks.
- Automatic review while typing, ongoing-chat review, mobile and remote
  sessions, repository indexing, image interpretation and free-form question
  generation are deferred.

## Evidence

- [Port validation, disabled path and stale generations](../../src-tauri/src/evaluation.rs).
- [Credential lifecycle, secret redaction and adapter failures](../../src-tauri/src/typesafe.rs).
- [Selection rules, English and Portuguese examples and stale results](../../src/context-review.test.ts).
- [Private calibration schema, opt-in, export and consent generations](../../src-tauri/src/review_calibration.rs).
- TypeSafe [models](https://docs.typesafe.ai/models) and [HTTP API](https://docs.typesafe.ai/api), checked 2026-09-27: pinned version, effective identity and JSON state are supported. The new structured requests are validated with synthetic fixtures, not a live quality study.
- [Contract](../contracts/context-evaluation.md).
