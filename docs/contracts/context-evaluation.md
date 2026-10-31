---
type: Contract
title: "Optional context evaluation"
description: "Bring-your-own-key TypeSafe adapter, evaluation port and key test in Settings."
status: stable
---

# Optional context evaluation

Current contract. Decision in
[ADR 0058](../decisions/0058-optional-context-evaluation.md).

## Responsibilities

| Part | Location | Owns |
| --- | --- | --- |
| evaluation port | `src-tauri/src/evaluation.rs`, `src/evaluation.ts` | closed request/answer types, bounds, validation, error codes, configuration generation |
| TypeSafe adapter and credential | `src-tauri/src/typesafe.rs` | private key file, enable flag, HTTP transport, retries, vendor validation and error translation |
| Settings | `src/typesafe-settings.ts` | save, replace and remove the key; **Test key** |

No product feature consumes the port today. Settings only manages the key and
offers **Test key**, which enables the integration if needed (the enable flag
only gates `context_evaluate`, and Test is its only caller) and sends one fixed
synthetic request with a single `task_kind` question. Success shows the
effective model; failures show the localized error code. A future feature must
import only the port types, never a vendor payload, the key or the HTTP status. The timeline, the
relay and the conversation protocol are unchanged; the integration is
independent of Claude, Codex and Antigravity and of the Prometeu Cloud account.

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
  change happens during the call.
- An unreadable or invalid file keeps the integration disabled and reports
  `err.evaluation.storage` as the status `problem`; it never blocks normal
  work. Saving a key over such a file fails with the same
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
| `stale` | configuration changed during the call; callers ignore it silently |
| `key`, `noKey`, `storage` | configuration errors from the `typesafe_*` commands |

Bounds checked before any network call: context is a nonempty string or JSON object, up to 24 KiB of UTF-8 (serialized JSON bytes for objects), one to
eight questions, identifiers and outcomes of lowercase letters, digits, `_` and
`.` up to 40 bytes, two to eight unique outcomes per question, prompts up to
600 bytes. Answers must name a requested question and one of its outcomes, at
most once, with a finite confidence between 0 and 1; omitted questions mean
abstention. The optional effective `model` is a nonempty ASCII identifier of at most 80 bytes (`A-Z`, `a-z`, digits, `.`, `_`, `-`); missing/null identity remains unknown and never inherits the requested pin.

Old string contexts remain accepted; old results without `model` deserialize with `None`. The field is additive for older consumers.

`context_evaluate` runs on a blocking worker thread, never on the UI or async
threads. The browser mock implements the evaluation commands without a key or any
network: it stores only whether a key was configured, and answers with a local
deterministic fake. `mock:typesafeFail` simulates a failure code; `mock:typesafeModel` overrides effective identity. No real key or network is needed.

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

Only **Test key** calls `context_evaluate`. It sends the fixed synthetic
request to TypeSafe with the person's key; no draft, issue, project or other
user data leaves the Mac.

## Removed features

Earlier versions offered a missing-context **Review request** in the launcher,
a Settings toggle to enable it and an opt-in local calibration history. All
three were removed; an existing `typesafe.json` keeps working. Current versions
neither read, write nor delete `<root>/review-calibration.json` and
`<root>/review-calibration.jsonl`; the `review_calibration_*` commands no
longer exist.

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
- `src/typesafe-settings.test.ts`: Test enables a saved key, sends only the
  synthetic request, surfaces backend errors and never overlaps a pending
  request after Settings re-renders.
- `src-tauri/tests/mock.rs`: command parity across Rust, the typed map and the
  mock.
