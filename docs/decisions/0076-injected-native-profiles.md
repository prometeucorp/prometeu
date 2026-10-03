# ADR 0076 — Native account profiles behind an injected backend

Date: 2026-09-28
Status: Accepted

## Context

Portable registration and login still obtained profile paths, credential
materialization and child environments from desktop account/provider modules.
A headless host needed those native guarantees without linking Tauri, while
startup/login needed a replaceable preparation dependency.

## Decision

Create the Unix `prometeu-profiles` crate. `ProfileBackend` exposes resolution,
preparation and environment application using native `Profile` and `Command`
types. `NativeProfiles` receives explicit roots and registers separate Claude,
Codex and external Antigravity behavior. Provider selection stays in this native
adapter; no platform branches enter application rules.

`ProfileFiles` supplies private directory creation and atomic file writes with
application-encoded errors. Native reads, parsing and resource links stay in the
crate. Desktop composition delegates private writes to the existing `paths.rs`
implementation, avoiding another copy of credential-file persistence policy.

`main.rs` injects one backend into provider preparation and native authentication.
It captures roots after login PATH adoption. Account metadata is captured per
operation; prepared profiles retain their ID, revision and home. The desktop
account facade re-exports the profile model and uses the same native implementation
for secondary feature callers. Changing composition roots requires recomposition;
those secondary facade callers retain call-time home discovery.

## Compatibility and limits

Persisted formats, IPC, CLI flags, credential isolation and shared-history paths
are unchanged. Managed Claude settings exclude alternative credentials and retain
MCP/project trust without copying identity. Managed Codex configuration uses private
file authentication and the OpenAI provider while sharing rollouts/indexes. External
Antigravity keeps its native environment. Existing link conflicts and I/O failures
remain errors and stop preparation. Profile setup adds no multi-file transaction
or rollback guarantee.

This is an execution-side Unix adapter, not a Windows profile implementation or
bridge payload. The core remains independently portable. Account/home discovery,
private-file composition and secondary feature callers retain desktop facades;
tool materialization and provider protocol adapters still need extraction before
a standalone headless runtime. ADR 0050's deployment decision remains in force.

## Evidence

- [Profile backend and model](../../src-tauri/crates/profiles/src/lib.rs).
- [Standalone tests](../../src-tauri/crates/profiles/src/tests.rs), plus moved
  [Claude](../../src-tauri/crates/profiles/src/claude.rs) and
  [Codex](../../src-tauri/crates/profiles/src/codex.rs) materialization fixtures.
- [Private file composition and permission test](../../src-tauri/src/account_profiles.rs).
- Injected failure regressions in [provider startup](../../src-tauri/src/agent_launch.rs)
  and [authentication](../../src-tauri/src/account_login.rs).
- [Application contract](../contracts/application-core.md#native-account-profiles).
