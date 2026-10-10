# ADR 0089 — Notch island over the main window's state

Date: 2026-10-07
Status: Accepted

## Context

The `notch` notification style showed one notice for eight seconds and then
disappeared. People running several agents want a persistent view at the top of
the screen: which agents are working or waiting, the selected accounts' quota,
and a way to approve, answer and jump back to a conversation without first
bringing the main window forward. The main window already owns the board, the
live conversation stream, the agent catalog, account selection and i18n.

## Decision

With notifications enabled and the `notch` style selected, the existing
`notification` overlay window becomes a persistent island. It stays collapsed
around the camera housing (a pixel-art flame and a waiting/running count) and
expands while the pointer is over it or while a notice is showing.

- **Content source.** `src/island.ts` in the main window reduces live `chat`
  events (last prompt, latest tool, open requests) and the board into a bounded
  snapshot, adds the selected accounts' quota from the status bar and pushes it
  through `island_update`. The overlay renders the snapshot; it never reads the
  board, transcripts or preferences itself.
- **Answers.** The overlay renders compact request panels sized for the notch
  and sends `request.respond` through the existing `chat_control` command for
  the tab. The backend's `request.closed` then updates every view, including
  the main window's cards. Plan edits and "allow and stop asking" stay in the
  chat.
- **Attention.** Finished turns only play the sound; the flame color and a
  green signal mark activity the Dock still counts as pending. Requests expand
  the island because they block the agent.
- **Navigation.** Clicking a row calls `island_open`, which reuses the notice
  path that revalidates the tab and focuses its conversation.
- **Hover.** WKWebView tracks the pointer only in the key window, and the
  overlay must not take focus. The backend samples the cursor ten times per
  second while the island is on and compares it with the overlay frame.
- **Geometry.** On macOS the overlay sits on the display with a notch, using
  `safeAreaInsets` and the auxiliary top areas; without one it sits inside the
  menu bar of the main window's display, and it is placed again on every
  display configuration change and whenever the main window moves, so on a
  display without a notch it follows the window it anchors to. Other desktops
  place it at the top center and let the compositor decide stacking.

## Trade-offs

Requests that opened before the main window started are not actionable from the
island; their rows show the board note and open the conversation. Pointer
sampling costs a few main-thread round trips per second while the island is on;
an `NSTrackingArea` would avoid them at the price of a custom AppKit view.
Resizing is immediate, without AppKit frame animation, so the main thread never
blocks on it. There are no global shortcuts for allow, deny or options. A
second local window can now send `chat_control`; it is app-owned content with
no remote input, and the command keeps its existing session checks.

The notice payload, queue and timing rules from
[ADR 0054](0054-local-notifications.md) are unchanged. The complete contract is
in [local notifications](../contracts/notifications.md).

## Evidence

- [Snapshot reducer and ordering](../../src/island.test.ts).
- [Native commands, bounds and geometry](../../src-tauri/src/island.rs).
- Browser preview through the mock island in `src/mock.ts`.
