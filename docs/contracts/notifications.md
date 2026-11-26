---
type: Contract
title: "Local notifications"
description: "The selection of events, the native delivery and the silent default values."
status: stable
---

# Local notifications

Notifications operate on the Mac of the owner. They do not change the relay,
the mobile shell, the transcripts, the board or the provider protocols. This
version does not give a push subscription or a selection of devices.

## Eligibility

`src/alert.ts` stays the only tracker of live events for the Dock and for the
optional notifications. These rules apply:

- A new `request.opened` that the person did not see causes a notice for an
  approval or an input. More events for the same open request do not cause a
  second notice.
- An accepted `session.state: busy` starts a possible completion. Assistant
  activity makes the completion sure.
- After a `turn.completed` with or without a failure, the tracker waits one
  second without background work. Then the completion is complete.
- An error does not make assistant activity necessary. New activity cancels the timer.
- Interruptions, snapshots, replay, read operations, request answers and the
  end of background tasks do not start a new completion at any time.
- If the person can see a conversation and it has the focus, its activity
  causes no notification.
- Comments go only to the Dock.

Native terminal observations use the additive `execution.observed` event from
the [conversation contract](conversation-events-v1.md). Explicit root activity
can make the execution sure, and the app does not make incorrect assistant
output.
`blocked` can cause one notification for each blocked period that the app
sees. But it
does not give the notch a request that the person can answer. Child activity
keeps a primary completion that waits for those children.

`idle` does not start a completion at any time. `unknown` cancels a candidate
before an uncertain removal of a child can release it. A status report that
comes after an interval cannot start again a completed activity identifier. Native replay
and baseline reads stay out of this tracker. This observation path does not make
native approvals or remote controls equal. These use different channels for
each provider, as the [agent runtime](agent-runtime.md) tells.

GitHub activity from the Cloud stream of events is the other source
([GitHub notifications](github-notifications.md)). Only rows that the app gets after
the first load since startup cause a notification, under the `github`
preference. Its title is the headline ("reviewer approved"). Its body is
`owner/repo#number`, plus the title when the app knows the title at that time.
Its `tab` is null.

Each delivery reads the current preferences. If a person disables an event, the
Dock and the unread state do not change. When a person enables notifications,
the app does not show previous activity again at any time. Payloads that
are special to a provider stay out of this path.

## Preferences

The localStorage record `prometeu:notifications` contains:

```json
{
  "version": 1,
  "enabled": false,
  "approval": true,
  "done": true,
  "error": true,
  "github": true,
  "style": "banner",
  "sound": false,
  "tone": "soft"
}
```

The styles are `banner`, `notch` and `none`. The tones are `soft`, `digital` and
`bell`. Notifications and sound start disabled. If a record is missing, has an
incorrect format or has an unknown version, the app uses these default values.
If a field has an incorrect type, the app uses the default value of that field.
The app does not read or write again the retired `prometeu:som` key.
Each Mac keeps its own preferences. If a save has a failure, the app shows the
failure and the controls go back to the saved values.

## Native delivery and IPC

`src/ipc.ts`, `src/mock.ts` and `src-tauri/src/notifications.rs` own the same
commands:

- `notification_permission({ request })`: returns `granted`, `denied`,
  `default` or `unavailable`. Only an explicit action of the user sends a
  request for authorization. When the person opens Settings, the app reads the status
  without a prompt. The app sends a request for authorization again in these
  two conditions. The person enables notifications while Banner is the
  selected style. Or the person explicitly selects Banner. This occurs after the initial status read is
  complete. A previous read cannot replace the result of the authorization. If
  banners are unavailable or denied, the app shows this directly below the
  master control. This message is not one of the saved preferences.
- `notification_show({ notice })`: supplies `{ title, body, style, sound, tab,
  openLabel, closeLabel }`. `sound` is a tone or null. `tab` is a local tab ID,
  or null for a test. The command refuses unknown enum values, unknown fields
  and text above the limits. The limits are 512 bytes for titles, 2048 for
  bodies, 128 for tab IDs and 256 for the text of action buttons. The command ignores
  a target that is not there, or that the app archived or cleaned.
- `notification_sound({ tone })`: plays a sample of a system sound from the
  allowlist.
- `notification_current()`: gives the initial content for the local overlay.
- `notification_dismiss()`: removes the notice and its target.
- `notification_open()`: closes the notice and opens its target, if the target
  is there.
- `island_enable({ enabled })`: only for the main window. It shows or hides the
  island. The main window enables it while notifications are on with the
  `notch` style.
- `island_update({ snapshot })`: only for the main window. It stores the
  snapshot that follows and sends it to the overlay. The command refuses
  snapshots above 512 KiB.
- `island_current()`: gives the initial `{ snapshot, layout, notice }` for the
  overlay.
- `island_resize({ height })`: gives the rendered height of the expanded
  overlay. The maximum is 640 points.
- `island_open({ tab })`: opens a tab that is there, as `notification_open`
  does. The island stays closed until the pointer goes out of it.

The commands for permission, delivery and sound accept only the main window. A
banner uses macOS UserNotifications. The app keeps a delegate for delivery in
the foreground and for clicks. A denied permission and failures of the native
delivery go to the UI. A click sends `notification-open` with the tab ID to the
main window. The main window validates the workspace again before it opens it.
Agent notices contain only the workspace title, not the output of the
conversation or the tools. GitHub notices contain only the metadata above, and
not comment text at any time.

The notch style is a continuous island in a different local Tauri window
([ADR 0089](../decisions/0089-notch-island.md)). This window has its own built
entry and a capability to receive events. It renders text nodes. When it shows,
it does not activate the application at any time, and it accepts the first
click. On macOS, it is on the display with a notch. When it is closed, it has
the height and width of the camera area, plus a wing on each side. The wings
show the app mark and the number of conversations that wait. If no conversation
waits, they show the number of conversations that operate. Without a notch, the
island is in the menu bar of the display of the main window.

The app moves the island again each time the display configuration changes,
because AppKit can move it or the display with the notch can change. The app
also moves it each time the main window moves. Thus, on a display without a
notch, the island follows its window. AppKit and CSS cut the lower corners of
the island. Without this cut, a rectangular window that is not transparent
is on top of the rounded web content.

The island expands to 560 points while the pointer is on it or while a notice
shows. If not, it goes back to the dimensions of the camera area. With the notch
style, done notices and error notices only play the selected sound. The island
shows the same information before, with these signals:

- It changes the color of the app mark.
- It moves a light across the mark while an agent operates.
- It shows a green signal for activity that the person did not see. This is the
  unread state of the Dock.

Approval notices and GitHub notices expand the island. A notice about a
conversation highlights the row of that conversation. Only notices without a
conversation get a line.

While the island is on, the backend reads the pointer position each 100 ms.
The webview does not receive pointer movement out of the key window.
When the pointer goes into the island, the backend sends `island-hover` to the
main window, with the current notice or `null`. Then the main window does these
operations:

- It removes the unread state of the conversations that the island shows, as
  it does when the person looks at them.
- If the latest GitHub notice is on the screen, it sets all events of the
  thread of that notice to read.

Other notices, for example the preview in Settings, do not set items to read.
When the person opens a conversation, the island stays closed until the
pointer goes out. A new notice replaces the previous notice. A notice goes
away after eight seconds. A generation check prevents an older timer from
the removal of a replacement. Only the action that opens a conversation gives the focus to the
main window. The local page is not a new remote IPC surface.

The app makes the overlay window one time. After that, the app only hides it.
Tauri stops the app only when its last window closes. The island cannot operate
without the main window and cannot open it again. Thus, when the main window closes, the app
stops explicitly, as it does without the island. The app saves the board on exit,
as usual. If local work will stop, the interface shows a question to the
person before the window closes ([ADR 0097](../decisions/0097-ask-before-ending-the-app.md)).

Only the main window builds the snapshot, from the board and from live `chat`
events. Replay does not go into the snapshot at any time:

```ts
type IslandSnapshot = {
  tabs: {
    id: string; title: string; agent: ProviderId;
    model: string; status: "rodando" | "querendo" | "pronta";
    unseen: boolean; note: string | null;
    prompt: string | null; since: number | null;
    activity: { tool: string; target: string } | null;
    request: { id: string; requestKind: "approval" | "question" | "plan";
      tool: string; input: object; toolUseId: string | null } | null;
  }[];
  usage: { agent: ProviderId; windows: { label: string; pct: number; resets: number }[] }[];
};
```

The tabs are local tabs in active workspaces that are not stopped. The most
urgent tabs are first, then the most recent. The maximum is twenty tabs. The app
knows `prompt`, `activity` and `request` only from the events that the main
window received after it started. Strings in tool inputs have a maximum of 2000
characters. Question text is the key of the answers, so the app keeps all of
it. The backend answers from its own copy of the request.

The app keeps request panels in a cache for each tab and request, because
provider request IDs are unique only in one conversation. If an answer has a failure,
the panel shows the error, and the person can try again. The quota comes from
the account that the person selected globally for each installed provider.

The overlay answers through `chat_control` with small request panels:

- Let a tool operate or deny it, with a short preview of its edit or command.
- Give approval to a plan, or open the conversation to change it.
- Answer questions one at a time. One selection answers a question with a single
  selection. The last answer sends all the answers.

"Allow and stop asking" stays in the chat. The `request.closed` event from the
backend removes the request from all views.

Sound operates independently of the visual delivery. It uses `/usr/bin/afplay` with
three specified system files (Pop, Glass and Ping). It does not accept a file
name or a shell command at any time. System banners obey the macOS notification
settings. The custom overlay and the sound files that the app plays independently
do not obey the macOS Focus filters. Sound tests and the delivery report playback
errors.

On Linux, the same commands and payloads apply
([ADR 0055](../decisions/0055-linux-desktop.md)). The permission is `granted`
when `notify-send` is on PATH, and `unavailable` when it is not. Freedesktop
does not have an authorization for each app. A banner runs
`notify-send --wait`. It has a default action only when the notice has a tab.
The selected action goes to the same `open_tab` path as a macOS click. If there
is no notification daemon, `notify-send` stops immediately with a failure and the
delivery reports the unavailable error.

The tones go to the freedesktop sound theme events `message-new-instant`,
`complete` and `bell`. `canberra-gtk-play` plays them. If it has a failure,
`pw-play` or `paplay` plays the standard theme files. The notch window shows as a usual
window, and the compositor selects its position in the stack of windows.

## Verification boundary

`src/notifications.test.ts` examines the compatibility, the persistence, the
selection of events and channels, and delivery failures. `src/alert.test.ts`
examines deduplication, visibility and eligibility. `e2e/notifications.spec.ts`
examines keyboard activation and the geometry of the controls in Chromium and
WebKit. It also examines navigation from a live notice in Chromium.

For native input styles and for navigation with the focus, a browser is
necessary.
The compatibility of the preferences and the selection of events and channels
stay in unit tests. The tests of the silent default values stay in
`src/alert.test.ts`. Rust tests validate the payload limits. The IPC parity test
examines the registration.

The browser mock renders notices with the same view as the native overlay. It
does not send a request for the permission of the operating system and does not play sound.
With the `notch` style, it also shows the island at the center of the top. The island
expands when the pointer is on the DOM element.
`src/island.test.ts` examines the snapshot reducer, the lifecycle of requests,
the input limits and the sequence of tabs.

These items operate only in a bundled or development macOS app:

- The notch geometry.
- The pointer samples.
- The delivery of the first click.
- The answers from the overlay.

Automated browser tests do not prove these items:

- The presentation in the macOS Notification Center.
- The sound output.
- The geometry on more than one monitor.
- The interaction with full screen and Focus.

For these items, a bundled macOS app and a native manual check are necessary.
The Test button in Settings operates without live models. Under `tauri dev`, system banners and
their authorization prompt are unavailable. Use `npm run app:bundle` for that
native check. The notch and the sound operate without a macOS notification
authorization, and they stay available during development.

The development bundle must have one signature for the full bundle. Thus, its
signing identifier agrees with its bundle identifier. The signature that only the
linker gives to the executable cannot authorize notifications for the bundle.
The development configuration selects ad-hoc signing. The bundle launcher makes
sure that this signature is correct before it opens the app.
