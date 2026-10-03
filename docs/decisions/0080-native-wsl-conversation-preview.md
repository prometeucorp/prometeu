# ADR 0080 — Native WSL conversation preview

Date: 2026-09-28
Status: Accepted

## Context

The headless conversation runs without Tauri, but the production desktop still
links Unix execution adapters. Compiling that composition for Windows would
reintroduce platform branches throughout the application or require all feature
parity before any native interaction could be tried.

## Decision

Deliver a separate experimental Tauri composition over the existing headless
v1 development protocol. The Windows composition injects a WSL process launcher
into a portable stdio connector; typed session operations hide transport envelopes
from the desktop handlers. Presentation receives an injected session port and
reuses canonical conversation rendering, requests, i18n and shared controls.
A separate browser entry injects the mock implementation.

The preview has its own narrow typed IPC registry and capabilities, rather than
adding commands that the production desktop cannot handle. Its binary build is
feature-gated so portable and headless checks need no GUI dependencies. The
Windows dependency graph excludes the Unix process/profile/tool/runtime crates.
No OS selection enters application rules or the conversation presentation.

## Consequences and compatibility

This gives a reviewable native conversation path with explicit setup and a small
surface for Windows validation. It duplicates shell composition, not protocol or
conversation behavior. It does not establish full Windows product support or
replace the eventual production service bridge. The development envelope and
headless private metadata remain compatible with ADR 0079; desktop IPC and
existing state formats are untouched.

Bootstrap failure publication precedes handshake-channel closure. After stopping
the child, failure reporting waits at most one second for the bounded stderr tail:
this preserves diagnostics despite reader scheduling without trusting descendants
to close inherited pipes. The runtime/bridge startup-failure integration test
checks that native error details reach the caller.

The isolated composition now selects the resident launcher under
[ADR 0082](0082-resident-wsl-attachments.md). Disconnect detaches the client;
the live conversation and supporting terminal remain owned by the host.
Direct stdio execution remains available as a disposable adapter. Operation
receipts, full feature parity and installation are still required.
The production deployment decision in ADR 0050 remains in force.

See the [preview contract and verification](../contracts/wsl-preview.md).

Actual WSL transport and same-thread Codex recall passed through Windows
`wsl.exe` invoked via WSL interop on 2026-09-28. The Windows MSVC executable
passed native WebView2/WSL acceptance on 2026-09-30 with a synthetic provider,
including actual window closure and reattachment. Its CI build remains a separate
check; this evidence does not establish complete application parity or packaging.
