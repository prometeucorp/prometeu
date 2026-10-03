# ADR 0075 — Account login through injected authentication and effects

Date: 2026-09-28
Status: Accepted

## Context

Account storage and selection were portable, but login admission, cancellation,
revision changes and quota/event sequencing lived in a Tauri command. Reusing
those guarantees in a headless host required separating coordination from native
credential handling and desktop execution.

## Decision

`accounts::login::LoginState` owns each host's pending login and cancellation.
`LoginService` composes it with the account registry, `AccountAuthentication` and
`LoginEffects`. Effects supply working-turn observations, quota refresh/forget
and publication. Native authentication prepares the captured profile and runs
the provider's private protocol, returning only identity metadata.

Registration/reconnection and reservation share the admission lock. Quota
invalidation precedes the active-turn check. Refusal clears admission before
refresh/publication. Accepted login publishes pending state before the host runs
the worker. Successful authentication checks cancellation, commits identity and
revision, then forgets quota. Only after the worker stops does the host clear
admission and refresh/publish again, including on worker/executor failure.
Cleanup matches attempt identity and is idempotent; stale cleanup cannot clear a
replacement attempt. Host effects execute outside both core locks.

Selection, removal and external attachment share those admission rules. Native
provider/method validation and ID generation remain at the edge; Antigravity's
external attachment performs no native login. `account_login.rs` registers native
authentication implementations, injected by `main.rs`. `accounts.rs` composes the
desktop effects and retains its blocking executor and singleton facade.

## Compatibility and limits

No persisted format, IPC shape, provider login command or account selection
behavior changes. Failed login retains registration; reconnection increments
revision only after successful authentication and storage. Native authentication
output and credentials stay private. Quota scheduling resumes after failure,
cancellation or active-turn refusal, preserving cached quota on failure.

A host must execute each attempt once and finish only after native work stops,
including join errors. Cancellation does not release admission prematurely and
is best effort after identity commit starts. This does not add automatic cleanup
for a dropped outer request or replace provider subprocess timeout/cleanup rules.

Native profile paths and credential materialization now use
[ADR 0076](0076-injected-native-profiles.md). Provider validation and tool
configuration still require extraction before a standalone headless host. There
is no Windows/WSL transport or native Windows shell yet; ADR 0050 remains in force.

## Evidence

- [Lifecycle and ports](../../src-tauri/crates/core/src/accounts/login.rs).
- [Controlled lifecycle tests](../../src-tauri/crates/core/src/accounts/login/tests.rs).
- [Native authentication adapter](../../src-tauri/src/account_login.rs).
- [Desktop composition](../../src-tauri/src/accounts.rs).
- [Application contract](../contracts/application-core.md#account-login-lifecycle)
  and [account contract](../contracts/accounts.md).
