---
type: Decision
title: "ADR 0037 — browser with visual context for the conversation"
description: "A browser in the workspace lets a person select page elements and capture them into the draft of the conversation."
status: stable
---

# ADR 0037 — browser with visual context for the conversation

Date: 2026-09-10

## Context

The native preview replaced the conversation. It also stopped drags that the app
ignored because they came from a different webview. The person did not have a
function to select HTML or styles, or to capture the page. Designers must point at the
interface while it operates and attach that context to the draft of the
conversation.

## Options considered

- Change the engine of the application. This makes the supply and the
  maintenance larger before we know of a limit that stops this flow.
- Use an iframe in the product. This makes the DOM composition easier. But it
  cannot show pages that do not let other pages frame them, and it gives no
  inspection access across origins.
- Keep WKWebView, control its lifecycle and capture context through a script
  that does not change, with a validated return value. This uses the engine again and keeps
  origins isolated.

## Decision

Keep one webview for each workspace, and show the conversation at the same time.
The presentation does the visibility changes one at a time and cancels previous
continuations. The child view sends uploads to WebKit. The primary view keeps the
native reception of attachments that
[ADR 0018](0018-native-file-promises.md) specifies.

Inspection starts only when the person selects it. A script in the bundle
highlights elements and gives data about them. Rust reads a result with a
limit and does not open
general IPC to the page. Capture uses the public WKSnapshotConfiguration API.
HTML, styles, the URL and the PNG go into the draft only when the person does an
action. They use the current contract for messages and attachments.

## Consequences

The native layer continues to make suspension necessary during overlays and its own tests
on macOS. The capture shows the viewport, with an optional crop, and not the full
page. The feature has no live CSS editor, no tree of framework components and no
automatic link to the source code. The interactive mock is a UI fixture. It does
not replace the native verification.

There is no change to the stored state, the transcripts, the providers or the
relay. The new IPC only adds commands, and a test examines its format. There is
no data migration.

## Evidence

- [Browser contract](../contracts/browser.md), with tests and limits.
- [Cursor Design Mode](https://cursor.com/docs/agent/design-mode): a reference
  for the interaction that selects elements and adds context to the conversation.
- [Tauri Webview](https://docs.rs/tauri/latest/tauri/webview/struct.Webview.html):
  a script in the webview that returns a value.
