---
type: Decision
title: "ADR 0089 — Notch island over the main window's state"
description: "A notch island that stays on the screen, follows agents, answers requests and opens conversations again."
status: stable
---

# ADR 0089 — Notch island over the main window's state

Date: 2026-10-07

## Context

The `notch` notification style showed one notice for eight seconds, and then
the notice went away. Persons who operate many agents want a view at the top of
the screen that stays there. This view must show these items:

- which agents work and which agents wait
- the quota of the selected accounts
- a method to give approval, answer and go back to a conversation before the main
  window comes to the front

The main window owns the board, the live conversation stream, the agent
catalog, the account selection and i18n at this time.

## Decision

When notifications are on and the person selects the `notch` style, the
current `notification` overlay window becomes an island that stays on the
screen. It stays small around the camera area. In that state, it shows the
app mark, an animation while an agent operates, and a count of agents that
wait or operate. It expands while the pointer is above it or while it shows a
notice.

- **Content source.** `src/island.ts` in the main window changes live `chat`
  events (last prompt, latest tool, open requests) and the board into a
  small snapshot with a limit. It adds the quota of the selected accounts from
  the status bar. It sends the snapshot through `island_update`. The overlay
  renders the snapshot. It does not read the board, transcripts or preferences
  itself.
- **Answers.** The overlay renders small request panels that fit the notch. It
  sends `request.respond` through the current `chat_control` command for the
  tab. Then the `request.closed` event of the backend updates all views. This
  includes the cards of the main window. Plan edits and "allow and stop asking"
  stay in the chat.
- **Signals.** Completed turns only play the sound. The color of the mark and
  a green signal show activity that the Dock continues to count as not seen.
  Requests expand the island, because they stop the agent until the person
  answers.
- **Navigation.** When the person clicks a row, the overlay calls
  `island_open`. This command uses the notice path again. That path validates
  the tab again and puts the focus on its conversation.
- **Hover.** WKWebView monitors the pointer only in the key window, and the
  overlay must not get the focus. Thus the backend reads the cursor position
  10 times each second while the island is on. It compares the position with
  the overlay frame. When the pointer goes into the island, Prometeu records
  that the person looked at it. The main window records the listed conversations
  as seen. It also records the GitHub thread of the notice on the screen as
  read.
- **Geometry.** On macOS, the overlay is on the display with a notch. It uses
  `safeAreaInsets` and the auxiliary top areas. Without a notch, the overlay
  is in the menu bar of the display of the main window. Prometeu puts it in
  position again after each change of the display configuration and each time
  the main window moves. Thus, on a display without a notch, it follows its
  window. Other desktops put it at the top center and let the compositor
  select the stack sequence.

## Trade-offs

The island cannot answer requests that opened before the main window started.
Their rows show the board note and open the conversation. While the island is
on, the pointer reads add some calls on the main thread each second. An
`NSTrackingArea` can prevent them, but a custom AppKit view is then necessary. The
island changes its dimensions immediately, without AppKit frame animation. Thus the
main thread does not wait on it. There are no global shortcuts for allow, deny
or options. With this decision, a second local window can send `chat_control`. The app owns
its content, and it has no remote input. The command keeps its current session
checks.

The hidden overlay can keep the process active after the main window closes,
and the person has no method to go back to it. Thus, when the person closes the
main window, the app stops explicitly. If local work will stop, the app gets
the approval of the person first ([ADR 0097](0097-ask-before-ending-the-app.md)). An app that
continues to operate and opens the main window again from the Dock is a
different window policy. It is not part of this decision.

The notice payload, the queue and the time rules of
[ADR 0054](0054-local-notifications.md) do not change. The full contract is in
[local notifications](../contracts/notifications.md).

## Evidence

- [Snapshot reducer and ordering](../../src/island.test.ts).
- [Native commands, bounds and geometry](../../src-tauri/src/island.rs).
- Browser preview through the mock island in `src/mock.ts`.
