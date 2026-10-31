---
type: Decision
title: "ADR 0018: receiving macOS file promises"
description: "Receiving macOS file promises."
status: stable
---

# ADR 0018: receiving macOS file promises


## Context

A screenshot's thumbnail announces a file promise, not an already-existing path.
The wry 0.55.1 used by the app reads `NSFilenamesPboardType`; the frontend only
knows paths. Restarting does not add the missing drag type and does not
materialize the capture.

## Options considered

- Require saving in Finder: it keeps the problem with the gesture the person
  expects.
- Disable the native drag and copy every browser `File`: it loses the original
  paths used by the chat and the terminal.
- Adapt `NSFilePromiseReceiver` in the backend: it preserves the normal paths
  and uses the specific native operation for captures not yet saved.

## Decision

Register the types accepted by `NSFilePromiseReceiver` in the main webview,
preserving Tauri's drag mechanism. Retain the receivers at the start of the
gesture and materialize them on drop, in a private folder per gesture. The
Objective-C bindings are already part of Tauri's transitive dependencies.

The internal `file-drag` event unifies immediate paths and promises. `pending`
captures the destination in the UI; `received` delivers the paths without
recomputing the target. AppKit receives the files in a queue; a bounded wait
gathers results outside the main thread. Transcripts, providers and the relay
keep consuming the existing path contract.

## Consequences

The app depends on the public AppKit API at that edge. Received files stay
outside the worktree and persist so that references in the history are not
broken; there is no automatic collection. Even a drop outside a destination
accepted by the UI may be materialized, since Tauri already accepts the native
gesture before the DOM hit test. Failures and timeouts produce a warning;
successfully received files are preserved.

## Evidence

- [Apple documentation](https://developer.apple.com/documentation/appkit/supporting-table-view-drag-and-drop-through-file-promises).
- `src-tauri/src/file_drop.rs`: registration, reception, destination validation
  and tests.
- `e2e/file-drop.spec.ts`: asynchronous phases, tab switching, failure, retry,
  Finder, launcher and terminal over the mock, with selected WebKit scenarios.
- The real thumbnail gesture was confirmed in Prometeu Dev on 2026-09-06, after
  registering `on_webview_event`: with `unstable`, `on_window_event` does not
  receive the main webview's drag. The native diagnosis confirmed the sequence
  of enter, move and drop, with an `NSFilePromiseReceiver`.
