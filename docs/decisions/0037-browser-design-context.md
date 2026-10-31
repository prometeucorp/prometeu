---
type: Decision
title: "ADR 0037 — browser with visual context for the conversation"
description: "Browser with visual context for the conversation."
status: stable
---

# ADR 0037 — browser with visual context for the conversation

Date: 2026-09-10

## Context

The native preview replaced the conversation and intercepted drags that the app
ignored because they came from another webview. Selecting HTML, styles and
capturing did not exist. Designers need to point at the running interface and
attach that context to the conversation's draft.

## Options considered

- Change the application's engine: it widens distribution and maintenance before
  demonstrating a limitation that blocks this flow.
- Use an iframe in the product: it makes DOM composition easier, but does not
  serve pages that forbid framing and gives no cross-origin inspection access.
- Keep WKWebView, coordinate its lifecycle and capture context through a fixed
  script with a validated return: it reuses the engine and preserves origin
  isolation.

## Decision

Keep one webview per workspace and show the conversation at the same time. The
presentation serializes visibility effects and invalidates old continuations.
The child view delivers uploads to WebKit; the main view keeps the native
attachment reception defined in
[ADR 0018](0018-native-file-promises.md).

Inspection is explicit. A script belonging to the bundle highlights and
describes elements; Rust queries a bounded result, without opening general IPC
to the page. Capture uses the public WKSnapshotConfiguration API. HTML, styles,
URL and PNG enter the draft through an action by the person, reusing the message
and attachment contract.

## Consequences

The native layer still requires suspension during overlays and its own tests on
macOS. Capture is of the viewport, with an optional crop, and not of the whole
page. It does not implement a live CSS editor, a framework component tree or
automatic source code resolution. The interactive mock is a UI fixture; it does
not replace that native verification.

There is no change in persisted state, transcripts, providers or the relay. The
new IPC is additive and its format is tested; there is no data migration.

## Evidence

- [Browser contract](../contracts/browser.md), with tests and limits.
- [Cursor Design Mode](https://cursor.com/docs/agent/design-mode): an interaction
  reference for selecting elements and adding context to the conversation.
- [Tauri Webview](https://docs.rs/tauri/latest/tauri/webview/struct.Webview.html):
  evaluation with a return value in the webview.
