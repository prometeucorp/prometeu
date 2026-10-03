# ADR 0074 — Portable account registry with injected storage

Date: 2026-09-28
Status: Accepted

## Context

Provider preparation has an injected boundary, but account registration and
selection still mixed compatibility rules, filesystem writes, native profiles
and desktop login effects. Reusing those rules in a headless execution host
required separating account state from native credential handling.

## Decision

Move account identity, registration, selection and compatibility models into
`prometeu-core::accounts`. `AccountRegistry` owns per-instance state and receives
an `AccountStore`. The store distinguishes missing storage from an empty registry.
Only missing storage imports the default external Claude/Codex accounts; load or
validation failures remain errors and cannot silently reset or overwrite state.

Updates serialize under one registry lock: clone, apply, validate, save if changed,
then replace memory. Callback, validation and save failures leave memory intact;
unchanged updates do not write. Reads return captured copies, without exposing
the registry lock. Revision checks retain account handoff and late-quota guards.
Store implementations and update callbacks must not re-enter the registry.

`account_store.rs::FileAccountStore` owns the existing private atomic JSON writes
at an explicit root. `accounts.rs` re-exports shared models and composes the store
behind the existing desktop singleton facade. Native profile paths, credentials,
native login protocols and desktop effects remain at that edge. Login
coordination now follows [ADR 0075](0075-injected-account-login.md).

Using an injected store keeps native paths and persistence effects out of the
portable rules. Keeping the current desktop facade avoids changing login and
quota ownership before their own extraction; the core itself has no singleton.

## Compatibility and limits

The persisted format, IPC snapshots, external defaults, removal semantics and
structured errors are unchanged. Unknown and retired provider entries round-trip
opaquely through persistence but are not exposed, queried or selected. Recognized
accounts retain validation. Explicit empty selections survive restarting.

One instance serializes its own changes; this adds no coordination between
processes or separate instances sharing storage. Native save errors after file
replacement can leave disk ahead of memory, retaining the existing writer's
semantics. There is no automatic reload or recovery of a failed initial load.

This extraction does not make profiles, login, tools or provider preparation a
standalone native crate. The headless runtime and Windows/WSL bridge still need
those dependencies and feature composition. ADR 0050 remains in force.

## Evidence

- [Registry and storage port](../../src-tauri/crates/core/src/accounts.rs).
- [Portable compatibility and failure tests](../../src-tauri/crates/core/src/accounts/tests.rs).
- [Native persistence and file tests](../../src-tauri/src/account_store.rs).
- [Desktop facade and existing compatibility fixtures](../../src-tauri/src/accounts.rs).
- [Account contract](../contracts/accounts.md) and
  [application contract](../contracts/application-core.md#account-registry).
