---
type: Contract
title: "Optional context evaluation"
description: "The TypeSafe adapter with the key of the person, the evaluation port and the key test in Settings."
status: stable
---

# Optional context evaluation

This contract is current.
[ADR 0058](../decisions/0058-optional-context-evaluation.md) records the
decision.

## Responsibilities

| Part | Location | Owns |
| --- | --- | --- |
| evaluation port | `src-tauri/src/evaluation.rs`, `src/evaluation.ts` | closed request and answer types, limits, validation, error codes, the configuration generation |
| TypeSafe adapter and credential | `src-tauri/src/typesafe.rs` | the private key file, the enable flag, the HTTP transport, new tries, the validation of the vendor format and the translation of errors |
| Settings | `src/typesafe-settings.ts` | save, replace and remove the key. **Test key** |

At this time, no product feature uses the port. Settings only controls the key
and gives **Test key**. This action enables the integration if necessary. The
enable flag controls only `context_evaluate`, and the test is its only caller.
The test sends one synthetic request that does not change, with one `task_kind` question. If the
test is correct, it shows the model that the service used. If the test has a failure, it
shows the localized error code. A future feature must import only the port
types. It must not import a vendor payload, the key or the HTTP status. The
timeline, the relay and the conversation protocol do not change. The
integration is not connected to Claude, Codex, Antigravity or the Prometeu Cloud
account.

## Credential and enablement lifecycle

- The integration is **disabled** by default. It is also disabled in these
  conditions:
  - The file is missing.
  - The installation is from an earlier version.
  - The file does not have `enabled: true` and a key.
- When a person saves a key, the app stores it and **does not** enable the
  evaluation. When a person replaces a key, the current selection stays. When a
  person removes the key, the app deletes the file. Thus, a subsequent key
  starts disabled again.
- To enable the integration, a saved key is necessary. If not, the app returns
  `err.evaluation.noKey`.
- Each save, replacement, removal, enable or disable increases a configuration
  generation in memory. The app holds the configuration lock during this change.
- An evaluation reads the key, the enablement and the generation together under
  that lock. If the generation changed after that snapshot, the evaluation
  returns `stale` and does not send the previous key. If the change occurs
  during the call, the evaluation discards the answer and stops the new tries.
- If the app cannot read the file or the file is incorrect, the integration
  stays disabled. The status `problem` reports `err.evaluation.storage`. This
  does not stop usual work. If a person saves a key in this condition, the
  save has a failure with the same code and does not replace the file.
  A missing file is the default configuration.

## Persistence

`<root>/typesafe.json` is private (`0600` in a `0700` folder). The app writes it
with the same atomic writer as other credentials:

```json
{ "version": 1, "enabled": false, "key": "…" }
```

All fields have default values when they are missing. The file adds to the
formats. Older versions of the app ignore it. It does not go into the board,
`team.json`, the relay, Cloud catalogs, transcripts or the repository. After a rollback, the file stays and has no effect.

## IPC

| Command | Arguments | Result |
| --- | --- | --- |
| `typesafe_status` | none | `{ configured, enabled, problem }` |
| `typesafe_save_key` | `{ key }` | the same status |
| `typesafe_remove_key` | none | the same status |
| `typesafe_set_enabled` | `{ enabled }` | the same status |
| `context_evaluate` | `{ request: { context, questions: [{ id, prompt, outcomes }] } }` | `{ answers: [{ id, outcome, confidence }], model: string | null }` |

The status does not contain the key or a part of it. `problem` is `null` or an
error with an i18n code. Failures reject with
`i18n:{"code":"err.evaluation.<code>"}`:

| Code | Meaning |
| --- | --- |
| `disabled` | the integration is off or has no key. The app did not send a call |
| `auth` | the service rejected the key (401/403) |
| `rate_limited` | 429 after the maximum number of new tries |
| `unavailable` | offline, timeout, 5xx or 529 after the maximum number of new tries, or a different status |
| `malformed` | the response is not in the closed questions, has an incorrect confidence, is too large or is not JSON |
| `invalid` | the request is not in the limits of the port, or the service refused it (400/413/422) |
| `stale` | the configuration changed during the call. Callers ignore it silently |
| `key`, `noKey`, `storage` | configuration errors from the `typesafe_*` commands |

The app examines these limits before it sends a network call:

- The context is a string that is not empty or a JSON object. It has a maximum
  of 24 KiB of UTF-8 (the serialized JSON bytes for objects).
- There are one to eight questions.
- Identifiers and outcomes contain only lowercase letters, digits, `_` and `.`.
  They have a maximum of 40 bytes.
- Each question has two to eight unique outcomes.
- Each prompt has a maximum of 600 bytes.

Each answer must identify a requested question and one of its outcomes, and it
can occur only one time. It must have a finite confidence between 0 and 1. A
question without an answer is an abstention. The optional `model` identifies the model that the service used. It is
an ASCII identifier that is not empty. It has a maximum of 80 bytes (`A-Z`,
`a-z`, digits, `.`, `_`, `-`). When the identity is missing or null, it stays
unknown. It does not get the value of the requested model.

The app continues to accept previous string contexts. Previous results without
`model` deserialize with `None`. The field adds to the format for older
consumers.

`context_evaluate` operates on a worker thread that can wait. It does not
operate on the UI threads or the async threads. The browser mock has the
evaluation commands without a key and without a network. It stores only if a
person configured a key, and it answers with a local deterministic simulation.
`mock:typesafeFail` simulates a failure code. `mock:typesafeModel` replaces the
identity of the model. An actual key or a network is not necessary.

## Transport

- The origin is `https://api.typesafe.ai`. `PROMETEU_TYPESAFE_URL` can replace
  it (HTTPS, or HTTP on loopback). The replacement must be only an origin. If
  it has userinfo, a path other than `/`, a query or a fragment, the app
  disables the adapter. The adapter makes the endpoint from the parsed origin, and it does
  not follow redirects.
- The connect timeout is 5 seconds, and the request timeout is 15 seconds.
  Responses larger than 256 KiB are malformed.
- The adapter makes a maximum of three tries for network errors, 429, 529 and
  500/502/503/504. It waits 400 ms and 1.2 s between the tries, or a
  `Retry-After` of a maximum of three seconds.
- The key goes only in `Authorization: Bearer`. The adapter discards response
  bodies, URLs and library errors. Errors contain only the application code.

### TypeSafe System One wire shape

The adapter follows the [TypeSafe HTTP API](https://docs.typesafe.ai/api). It
keeps the shape of the request and of the response isolated in
`typesafe.rs::wire`. On 2026-09-24, the initial string requests returned live
Choice answers, also with eight questions in one call. On 2026-09-27, we
compared the current structured request below with the documentation of the
vendor and with synthetic fixtures. No live quality study examined it at this
time:

```http
POST /v1/systemone
Authorization: Bearer <key>

{ "state": { "requester": { "draft": "…" }, "third_party": { "issue": null }, "project": { "name": "…", "repositories": "", "base": "main" }, "attachments": { "count": 0, "inspection": "uninspected", "instruction": "…" } }, "model": "jev-1.13.0", "questions": { "task_kind": { "type": "choice", "instructions": "…", "criteria": { "bug_fix": null, "feature": null } } } }
```

```json
{ "model": "jev-1.13.0", "answers": { "task_kind": { "type": "choice", "choice": "bug_fix", "confidence": 0.93, "probabilities": { "bug_fix": 0.93, "feature": 0.07 } } } }
```

The Choice result becomes the `{ id, outcome, confidence }` value that the
application owns. The `model` of the response goes together with the
answers. The values for each outcome and the usage stay out of the port. A missing model is
unknown, and a model that is not a string is malformed. An answer that is
missing is an abstention. All other formats are `malformed`.

## External data flow

Only **Test key** calls `context_evaluate`. It sends the synthetic
request to TypeSafe with the key of the person. No draft, issue, project or
other user data goes out of the Mac.

## Removed features

Earlier versions had these features:

- A **Review request** in the launcher for missing context.
- A toggle in Settings to enable it.
- A local calibration history that a person had to enable.

We removed these three features. A current `typesafe.json` continues to
operate. Current versions do not read, write or delete
`<root>/review-calibration.json` and `<root>/review-calibration.jsonl`. The
`review_calibration_*` commands are not available.

## Tests

- `src-tauri/src/evaluation.rs`: the disabled path sends no call. It also
  examines the limits, the validation of the closed set, and an expired
  generation during the call and after the credential snapshot.
- `src-tauri/src/typesafe.rs`: these tests examine these items:
  - The disabled default values and files from earlier versions.
  - A save does not enable the integration, and a removal disables it.
  - The `0600` file, and that the status and the errors do not contain the key.
  - The System One wire shape.
  - The translation of 401, 429, 529, 503, offline, timeout and responses that
    are not JSON.
  - The maximum number of new tries, the recovery after a temporary failure and the
    cancellation between new tries.
  - A key that a person replaced, disabled or removed after the credential
    snapshot.
  - The app does not replace an incorrect file.
  - Replacements of the origin that contain only an origin.
- `src/typesafe-settings.test.ts`: the test enables a saved key and sends only
  the synthetic request. It shows backend errors. A new request does not start
  while a request waits, also after Settings renders again.
- `src-tauri/tests/mock.rs`: the same commands in Rust, in the typed map and in
  the mock.
