---
type: Decision
title: "ADR 0024 — Command-owned IPC argument and result types"
description: "Each IPC command owns the types of its arguments and its result."
status: stable
---

# ADR 0024 — Command-owned IPC argument and result types

Date: 2026-09-07

## Context

The previous TypeScript wrapper did a check of the command names. But a caller
was able to select a free return type and send arguments without limits. Thus
the browser mock was able to become different from these assumptions, and the
tests for the command names continued to show no errors.

## Options considered

1. Keep the list of names. Use the type notes of the callers and runtime tests.
2. Share a TypeScript command map between the callers and the browser handlers.
3. Make Rust bindings for commands and events immediately.

## Decision

Use `src/ipc.ts` as the TypeScript command map. Each command owns its arguments
and its result. `invoke` gets the two types from the command name. The browser
mock does the same `IpcHandlers` map. Wrappers use the exported types that
come from the commands. They do not select a free generic type for the result.

Keep the current Tauri transport, the Rust handlers, the error format and the
test that compares the command names. Make bindings only after a different
proof includes these items:

- The serde enums and the default values.
- The optional values.
- The errors.
- The events.

For an incremental verification between the two languages, serialize typical
Rust values from the code into a fixture that is part of the repository. Then
compile the JSON literals against the TypeScript contracts in the code. Rust
tests reject fixtures that are not current. Consumer tests validate canonical
events and give them to the reducer. This method does not add a binding
generator or a schema library. The examples accept added fields, and they prove
only the structures that they use. They do not replace validation. They do not
show that the tests cover all arguments and results.

## Consequences

Incorrect arguments and incorrect mock results stop the compilation. This occurs
without a new dependency. All application commands in the registry have explicit
browser handlers. The dynamic plugin commands are not in the application map.

The structures of the Rust arguments and results must continue to agree through
manual work and boundary tests. This map does not validate runtime data that is
not trusted or the payloads of Tauri events. No wire format or stored format
changes. A rollback changes only the source code back.

## Evidence

- [IPC contract](../contracts/ipc.md).
- [Invocation and compile-time regressions](../../src/ipc.test.ts).
- [Command-name parity](../../src-tauri/tests/mock.rs).
- [Rust serialization producer](../../src-tauri/src/boundary_contract.rs),
  [literal type check](../../scripts/check-contracts.mjs) and
  [consumer regressions](../../src/backend-contract.test.ts).
- `npm run typecheck` validates the callers and the mock handlers.
