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
preserving Tauri's drag mechanism for existing files. Replace the implementation
of `performDragOperation:` on the class that declares it (Wry's webview class) and
handle file promises only when the receiving view is the main webview; every
other call goes to Wry's original implementation. Never change the live view's
class: KVO already swaps it, and on macOS 27.2.1 an injected subclass makes AppKit
assert in `NSDP_getComputedPropertyValue` whenever the window zooms or resizes
with animation. Read the gesture's
pasteboard and request reception synchronously inside that callback, in a private
folder per gesture. Do not forward a handled promise to Wry, which would queue a
second drop. Other drag methods and ordinary file drops retain Wry's behavior.
The Objective-C bindings are already part of Tauri's transitive dependencies.

Tauri queues `on_webview_event` until after the native callback returns. Starting
reception there violates AppKit's requirement that it happen during
`prepareForDragOperation:`, `performDragOperation:` or `concludeDragOperation:`.
On macOS 27.2.1 this raises the assertion at `NSFilePromiseReceiver.m:300`; the
foreign exception reaches Tao's Rust panic boundary and aborts the process.
Keeping receivers alive does not extend the allowed native callback scope.

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
- `src-tauri/src/file_drop.rs`: native interception, reception, destination
  validation and tests. The interception test verifies the patched implementation
  and that a second install keeps the original; it does not simulate an AppKit
  drag session.
- Five local crash logs on 2026-11-17 (v0.41.1) abort in
  `NSDP_getComputedPropertyValue` from `-[WKWebView _viewDidChangeEffectiveCornerRadii]`
  during window zoom, from both the title-bar double-click and the green button,
  after the view's class was replaced at runtime.
- Two local crash logs on 2026-11-16 contain `NSFilePromiseReceiver.m:300`.
  Disassembly of the installed AppKit maps that assertion to the callback-scope
  requirement above. Native smoke validation must drag a floating screenshot
  thumbnail into chat and also check an ordinary Finder file drop.
- `e2e/file-drop.spec.ts`: asynchronous phases, tab switching, failure, retry,
  Finder, launcher and terminal over the mock, with selected WebKit scenarios.
- The real thumbnail gesture was confirmed in Prometeu Dev on 2026-09-06, after
  registering `on_webview_event`: with `unstable`, `on_window_event` does not
  receive the main webview's drag. The native diagnosis confirmed the sequence
  of enter, move and drop, with an `NSFilePromiseReceiver`.
