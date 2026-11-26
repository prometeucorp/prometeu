---
type: Decision
title: "ADR 0018: receiving macOS file promises"
description: "The desktop receives macOS file promises in the backend."
status: stable
---

# ADR 0018: receiving macOS file promises


## Context

The thumbnail of a screenshot gives a file promise, not the path of a file that
is on the disk. The app uses wry 0.55.1, which reads `NSFilenamesPboardType`.
The frontend knows only paths. When the app starts again, it does not add the
missing drag type, and it does not write the screenshot to a file.

## Options considered

- Tell the person to save the screenshot in Finder first. The problem stays with
  the usual gesture of the person.
- Disable the native drag and copy each browser `File`. The chat and the
  terminal then do not get the initial paths.
- Adapt `NSFilePromiseReceiver` in the backend. The usual paths stay the same,
  and the native operation for screenshots that are not saved receives the
  screenshots.

## Decision

Add the types that `NSFilePromiseReceiver` accepts to the primary webview. Keep
the drag mechanism of Tauri for files that are on the disk.

Replace the implementation of `performDragOperation:` on the class that declares
it, the webview class of Wry. Process file promises only when the view that
receives the drop is the primary webview. All other calls go to the initial
implementation of Wry.

Do not change the class of the live view at any time. KVO changes it before.
On macOS 27.2.1, a subclass that the app puts in causes an AppKit assertion in
`NSDP_getComputedPropertyValue`. This occurs each time that the window zooms or
changes dimensions with animation.

Read the pasteboard of the gesture and request the reception synchronously in
that callback. Each gesture has a private folder. Do not send a processed
promise to Wry, because Wry then puts a second drop in its queue. Other drag
methods and usual file drops keep the behavior of Wry. The Objective-C bindings
are part of the transitive dependencies of Tauri before this change.

Tauri keeps `on_webview_event` in a queue until the native callback returns. If
the reception starts there, it does not obey an AppKit requirement. AppKit must
receive the files during `prepareForDragOperation:`, `performDragOperation:` or
`concludeDragOperation:`. On macOS 27.2.1, this causes the assertion at
`NSFilePromiseReceiver.m:300`. The Objective-C exception goes to the Rust panic
boundary of Tao, and the process stops. A receiver that stays in memory does not
make the permitted scope of the native callback larger.

The internal `file-drag` event uses one format for the paths of files on the
disk and for promises. `pending` records the destination in the UI. `received`
sends the paths and does not calculate the target again. AppKit receives the
files in a queue. A wait with a limit collects the results on a thread that is
not the primary thread. Transcripts, providers and the relay continue to use the
current path contract.

## Consequences

At that edge, the app uses the public AppKit API. Received files are not in the
worktree. The app keeps them so that the history can continue to refer to them.
The app does not delete them automatically. A drop on an area that is not a
destination in the UI can also write a file. The cause is that Tauri accepts the
native gesture before the DOM hit test. A failure or a timeout shows a warning.
The app keeps the files that it received correctly.

## Evidence

- [Apple
  documentation](https://developer.apple.com/documentation/appkit/supporting-table-view-drag-and-drop-through-file-promises).
- `src-tauri/src/file_drop.rs` has the native interception, the reception, the
  validation of the destination and the tests. The interception test makes sure
  that the patched implementation operates. It also makes sure that a second
  install keeps the initial implementation. It does not simulate an AppKit drag
  session.
- Five local crash logs from 2026-11-17 (v0.41.1) stop in
  `NSDP_getComputedPropertyValue` from
  `-[WKWebView _viewDidChangeEffectiveCornerRadii]`.
  The crashes occurred during a window zoom from a double-click on the title
  bar and from the green button. Before the crashes, the app replaced the class of the view at runtime.
- Two local crash logs from 2026-11-16 contain `NSFilePromiseReceiver.m:300`.
  The disassembly of the installed AppKit connects that assertion to the
  callback scope requirement above. A native smoke validation must drag a
  screenshot thumbnail into the chat. It must also do a check of a
  usual file drop from Finder.
- `e2e/file-drop.spec.ts` has tests for asynchronous phases, tab changes,
  failure, retry, Finder, the launcher and the terminal through the mock. Some
  WebKit scenarios are also in the test.
- The thumbnail gesture on a Mac operated correctly in Prometeu Dev on
  2026-09-06, after the registration of `on_webview_event`. With `unstable`,
  `on_window_event` does not receive the drag of the primary webview. The native
  diagnosis showed the sequence of enter, move and drop, with an
  `NSFilePromiseReceiver`.
