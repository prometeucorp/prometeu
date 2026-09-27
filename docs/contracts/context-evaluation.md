# Optional context evaluation and missing-context review

Status: current contract. Decision in
[ADR 0058](../decisions/0058-optional-context-evaluation.md).

## Responsibilities

| Part | Location | Owns |
| --- | --- | --- |
| evaluation port | `src-tauri/src/evaluation.rs`, `src/evaluation.ts` | closed request/answer types, bounds, validation, error codes, configuration generation |
| TypeSafe adapter and credential | `src-tauri/src/typesafe.rs` | private key file, enable flag, HTTP transport, retries, vendor validation and error translation |
| desktop shell | `src/typesafe.ts`, `src/typesafe-settings.ts` | IPC port, status store, Settings controls |
| local calibration | `src-tauri/src/review_calibration.rs`, `src/review-calibration.ts` | independent consent, private content-free records, summary, CSV export and Clear |
| missing-context review | `src/context-review.ts` (rules), `src/context-review-view.ts` (launcher panel) | context building, closed questions, selection, localized question catalog, stale-result binding |

The review imports only the port types. It never sees a vendor payload, the key
or the HTTP status. The timeline, the relay and the conversation protocol are
unchanged; the integration is independent of Claude, Codex and Antigravity and
of the Prometeu Cloud account.

## Credential and enablement lifecycle

- The integration is **disabled** by default. A missing file, an old
  installation, or a file without `enabled: true` and a key is disabled.
- Saving a key stores it and **does not** enable evaluation. Replacing a key
  keeps the current choice. Removing the key deletes the file, so a later key
  starts disabled again.
- Enabling requires a saved key (`err.evaluation.noKey` otherwise).
- Every save, replacement, removal, enable or disable bumps an in-memory
  configuration generation while holding the configuration lock. An evaluation
  reads the key, the enablement and the generation together under that lock;
  if the generation changed after that snapshot it returns `stale` without
  sending the old key, or discards the answer and stops retrying when the
  change happens during the call. The frontend also bumps its
  own epoch and discards pending and shown results.
- An unreadable or invalid file keeps the integration disabled and reports
  `err.evaluation.storage` as a visible configuration problem in Settings; it
  never blocks normal work. Saving a key over such a file fails with the same
  code instead of overwriting it; a missing file is the default configuration.

## Persistence

`<root>/typesafe.json`, private (`0600` in a `0700` directory) with the atomic
writer used by other credentials:

```json
{ "version": 1, "enabled": false, "key": "…" }
```

All fields default when absent. The file is additive: older app versions ignore
it, and it never enters the board, `team.json`, the relay, Cloud catalogs,
transcripts or the repository. Rolling back leaves it inert.

## IPC

| Command | Arguments | Result |
| --- | --- | --- |
| `typesafe_status` | none | `{ configured, enabled, problem }` |
| `typesafe_save_key` | `{ key }` | the same status |
| `typesafe_remove_key` | none | the same status |
| `typesafe_set_enabled` | `{ enabled }` | the same status |
| `context_evaluate` | `{ request: { context, questions: [{ id, prompt, outcomes }] } }` | `{ answers: [{ id, outcome, confidence }], model: string | null }` |

The status never contains the key or any part of it. `problem` is `null` or an
i18n-coded error. Failures reject with `i18n:{"code":"err.evaluation.<code>"}`:

| Code | Meaning |
| --- | --- |
| `disabled` | integration off or without a key; no call was made |
| `auth` | the service rejected the key (401/403) |
| `rate_limited` | 429 after bounded retries |
| `unavailable` | offline, timeout, 5xx or 529 after bounded retries, or any other status |
| `malformed` | response outside the closed questions, invalid confidence, oversized or not JSON |
| `invalid` | request outside the port bounds, or refused by the service (400/413/422) |
| `stale` | configuration changed during the call; the frontend ignores it silently |
| `key`, `noKey`, `storage` | configuration errors from the Settings commands |

Bounds checked before any network call: context is a nonempty string or JSON object, up to 24 KiB of UTF-8 (serialized JSON bytes for objects), one to
eight questions, identifiers and outcomes of lowercase letters, digits, `_` and
`.` up to 40 bytes, two to eight unique outcomes per question, prompts up to
600 bytes. Answers must name a requested question and one of its outcomes, at
most once, with a finite confidence between 0 and 1; omitted questions mean
abstention. The optional effective `model` is a nonempty ASCII identifier of at most 80 bytes (`A-Z`, `a-z`, digits, `.`, `_`, `-`); missing/null identity remains unknown and never inherits the requested pin.

Old string contexts remain accepted; old results without `model` deserialize with `None` and the review abstains. The new field is additive for older consumers. Updated frontend and backend ship together. A mismatched older backend rejects the new object context as an invalid IPC argument; ordinary creation remains available.

`context_evaluate` runs on a blocking worker thread, never on the UI or async
threads. The browser mock implements the evaluation and calibration commands without a key or any
network: it stores only whether a key was configured, and answers with a local
deterministic fake. `mock:typesafeFail` simulates a failure code; `mock:typesafeModel` overrides effective identity. Calibration preferences/records use `mock:reviewCalibration`; export writes CSV to `mock:reviewCalibrationExport`. No real key or network is needed.

## Transport

- Origin `https://api.typesafe.ai`, overridable with `PROMETEU_TYPESAFE_URL`
  (HTTPS, or HTTP on loopback). The override must be an origin only: userinfo,
  a path other than `/`, a query or a fragment disable the adapter. The
  endpoint is built from the parsed origin, and redirects are not followed.
- 5-second connect and 15-second request timeouts; responses above 256 KiB are
  malformed.
- At most three attempts for network errors, 429, 529 and 500/502/503/504, with
  400 ms and 1.2 s backoff or a `Retry-After` of at most three seconds.
- The key travels only as `Authorization: Bearer`. Response bodies, URLs and
  library errors are discarded; errors carry only the application code.

### TypeSafe System One wire shape

The adapter follows the [TypeSafe HTTP API](https://docs.typesafe.ai/api) and
keeps its request and response shape isolated in `typesafe.rs::wire`. Original
string requests returned live Choice answers on 2026-09-24, including eight
questions in one call. The current structured request below is checked against
the vendor documentation and synthetic fixtures on 2026-09-27; it has not yet
been evaluated in a live quality study:

```http
POST /v1/systemone
Authorization: Bearer <key>

{ "state": { "requester": { "draft": "…" }, "third_party": { "issue": null }, "project": { "name": "…", "repositories": "", "base": "main" }, "attachments": { "count": 0, "inspection": "uninspected", "instruction": "…" } }, "model": "jev-1.13.0", "questions": { "task_kind": { "type": "choice", "instructions": "…", "criteria": { "bug_fix": null, "feature": null } } } }
```

```json
{ "model": "jev-1.13.0", "answers": { "task_kind": { "type": "choice", "choice": "bug_fix", "confidence": 0.93, "probabilities": { "bug_fix": 0.93, "feature": 0.07 } } } }
```

The Choice result becomes the application-owned `{ id, outcome, confidence }`; the effective response `model` travels alongside the answers. Probabilities and usage remain outside the port. A missing model is unknown, and a non-string model is malformed.
An omitted answer is an abstention. Any other shape is `malformed`.

## External data flow

Only after the person chooses **Review request** with the integration enabled,
the local backend sends the context directly to TypeSafe with the person's key.
Typing never sends anything. Settings and the action's tooltip state this. The
context contains:

- `requester.draft`, in the person's own language;
- `third_party.issue`, the complete originating Linear issue when attached: identifier, title,
  description, state, team, project and labels;
- the project name, additional repository names and the base branch;
- the number of attachments, marked as **uninspected**. Attachment paths and
  contents are never sent.

Issue and project metadata are single-line fields capped per field (256 serialized UTF-8 bytes; 1 KiB for the title and for each joined list of labels or repositories) and clipped with `…`. The draft and issue description share the remaining budget; the draft receives at least half when both are long. Clipped text ends with `[…]`. Every field, JSON key and escape counts toward the 22 KiB context budget, below the port's 24 KiB bound. All eight questions explicitly treat issue text as quoted third-party data, never instructions to follow; that mitigation cannot guarantee adversarial robustness.

## Missing-context review

Scope: new local workspaces, bug fixes and features. The request asks one call
with eight closed questions: the task kind; for expected behavior, reproduction
information and unresolved business rules, a state among `present`,
`ambiguous`, `absent`, `uninspected` and `not_applicable` and who can resolve a
gap (`person`, `agent`, `unclear`); and what an unresolved rule concerns.

Selection rules, in `src/context-review.ts`:

- an absent or different model from the `jev-1.13.0` policy baseline: no suggestion (`uncertain`);
- task kind below the language threshold: no suggestion (`uncertain`); `investigation`:
  no suggestion; anything other than a bug fix or feature: no suggestion;
- a topic becomes a suggestion only when `absent` or `ambiguous` with at least
  the language's presence threshold **and** resolvable only by the person at its resolver threshold;
- `present`, `not_applicable` and `uninspected` never produce a question;
  reproduction is never requested while attachments exist;
- order of consequence: business rule, reproduction (bug fixes), expected
  behavior;
- the question text comes from the localized `review.q.*` catalog; the rule
  question uses its subtype only at the language's subtype threshold, otherwise a generic one.

`src/review-policy.ts` detects only the draft language through a deterministic vocabulary, without reading UI language or issue text. English uses 0.70/0.80/0.70/0.60 (kind/presence/resolver/subtype), Portuguese 0.75/0.85/0.75/0.65, and unknown/mixed/empty drafts 0.80/0.90/0.80/0.70. These are hypotheses; the predeclared measurement protocol is in ADR 0058. Each finished view includes the effective model; Settings shows a calibration-pending notice after a missing or mismatched model, without invalidating the configuration epoch.

Interaction:

- one suggestion at a time, at most two per unchanged request; reviewing an
  unchanged request again reuses the result without another call;
- **Answer** appends a localized, editable `Clarification: … / Answer:` block to
  the draft; **Let the agent investigate** appends an explicit instruction;
  **Dismiss** suppresses that topic for the same draft and context and shows the
  next one;
- every result is bound to the draft/context revision — every field the request
  sends: the draft; the issue identifier, title, description, state, team,
  project and labels; the project name, additional repositories and base
  branch; and the attachment count — and to the configuration epoch. Edits,
  issue or project changes, submission, closing the launcher, disabling, and
  key replacement or removal discard pending and shown results;
- **Create workspace** stays available during evaluation, after dismissal and
  on failure; drafts and attachments are preserved.

## Calibration persistence and IPC

The independent opt-in defaults off and does not need a TypeSafe credential.
`<root>/review-calibration.json` stores `{ "enabled": false, "generation": 0, "clearing": false }`.
The additive `clearing` field defaults to false for older consent files.
`<root>/review-calibration.jsonl` contains one final record per completed current
review. Both files are `0600` in the private `0700` application directory:

```json
{"v":1,"at":1790000000000,"model":"jev-1.13.0","language":"pt","answers":[{"id":"task_kind","outcome":"feature","confidence":0.91}],"suggested":["business_rule"],"action":"answered","created":true,"latency_ms":125}
```

- `at` is the request start time in epoch milliseconds; `latency_ms` measures
  evaluator completion. `language` is `en`, `pt` or `other`; unknown model is null.
- `answers` admits only the eight review question IDs and their closed outcomes,
  unique IDs and finite confidence in `[0,1]`. Model identities in records use
  the supported `jev-major.minor.patch` family, with one to three digits per component; other identities become null. The native and browser stores reject arbitrary model names, text and unknown fields.
- `suggested` contains the topics actually shown (at most two, unique).
  `action` is the first explicit `answered`, `handed_to_agent`, `dismissed`, or
  `none`. Answer means the clarification block was inserted, not completed.
- `created` reflects the actual `create_workspace` result, not the Create click.
  The launcher finalizes records exactly once after cancellation or that result;
  failed/stale/in-flight evaluations are excluded. Closing the app first loses
  buffered records. Multiple completed reviews in one launcher can share the
  same creation outcome, so record count is not unique workspace count.
- Consent is captured before evaluation and checked again under the backend's
  storage lock. Disable, re-enable and Clear increment the persisted generation;
  late results from earlier consent cannot append. Clear preserves enablement.
- Clear persists `clearing: true` with the new generation before replacing the
  records file, then resets the marker only after replacement succeeds. An
  interrupted or failed Clear blocks new records, summaries and export until
  Clear succeeds, including after restart or toggling consent. The browser
  mock stores consent and history in one atomic localStorage value and does not
  need this native two-file recovery marker.
- Unknown record/answer fields are rejected, and all categorical values are
  validated before persistence. There is no text, project/repository name,
  credential, path, attachment content, workspace ID or vendor probabilities.
  Records never enter board state, transcripts, telemetry, Cloud or relay.
- Corrupt storage reports a localized error and never overwrites history while
  appending. Clear explicitly recovers damaged records. Collection failure is
  independent of review and workspace creation.
- The native store validates existing history once per process or detected file
  change, caching only summary counters and the file's identity, size and
  modification/change timestamps. Successful appends update those counters;
  failed I/O invalidates the cache. Settings summaries and subsequent appends
  avoid reparsing unchanged history. CSV export still reads the complete file.

| Command | Arguments | Result |
| --- | --- | --- |
| `review_calibration_status` | none | `{ enabled, generation, records, created, actions: { answered, handed_to_agent, dismissed, none } }` |
| `review_calibration_set_enabled` | `{ enabled }` | same status |
| `review_calibration_append` | `{ generation, record }` | null; a disabled/stale consent is ignored |
| `review_calibration_clear` | none | same status with zero counts and a new generation |
| `review_calibration_export` | `{ path }` | null; writes the selected CSV file |

Failures use `err.calibration.storage`, `err.calibration.invalid` or
`err.calibration.export`. The Settings summary and error are refreshed after
collection, preference changes and Clear. Frontend consent revisions suppress
late append errors and summary refreshes from before a consent change or Clear;
collection pauses while either mutation is pending. CSV contains the record fields in the
order shown above; arrays are JSON in escaped CSV cells. Export is private and
atomic, accepts regular `.csv` destinations, rejects symlinks and leaves the
parent directory's permissions alone. Exports are user-owned copies: Clear only
removes the application history, not exported files. Nothing is uploaded.

## Tests

- `src-tauri/src/evaluation.rs`: disabled path makes no call, bounds, closed-set
  validation, stale generation during the call and after the credential
  snapshot.
- `src-tauri/src/typesafe.rs`: disabled defaults and old files, saving does not
  enable, removal disables, `0600` file, key absent from status and errors, the
  System One wire shape, 401/429/529/503/offline/timeout/non-JSON translation, bounded
  retries, recovery after a transient failure, cancellation between retries, a
  key replaced, disabled or removed after the credential snapshot, no overwrite
  of an invalid file, origin-only overrides.
- `src/context-review.test.ts`: context building with the complete issue and
  uninspected attachments, byte budget, English and Portuguese examples including
  short, investigative, attachment and low-confidence negatives, the CSV example,
  version mismatch/missing identity, per-language thresholds, escaped JSON budgets, content-free finalization and failed creation, one-at-a-time and two-per-request limits, dismissal, stale responses after
  edits, closing, disabling and key changes, failures and explicit-only calls.
- `src-tauri/src/review_calibration.rs`: default-off/independent consent, private permissions, closed content-free schema, CSV, damaged history, summary cache invalidation, interrupted Clear/restart recovery, legacy consent compatibility, Clear/disable generations and export destination protection.
- `src/typesafe.test.ts` and `src/review-calibration.test.ts`: model diagnostics/epoch isolation, collection failures, stale append/summary rejection around Clear and browser/native calibration contract parity.
- `src-tauri/tests/mock.rs`: command parity across Rust, the typed map and the
  mock.

No browser scenario is added: the behavior is covered by unit tests, and the
panel reuses shared controls without a new keyboard or focus model (see the
[E2E scope policy](../operations/development.md#e2e-scope)).
