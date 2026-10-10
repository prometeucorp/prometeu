# Local notifications

Notifications run on the owner's Mac. The relay, mobile shell, transcripts,
board and provider protocols are unchanged. No push subscription or device
selection is offered in this version.

## Eligibility

`src/alert.ts` remains the single live-event tracker for the Dock and optional
notifications. A new unseen `request.opened` produces an approval/input notice;
repeated events for the same outstanding request do not repeat it. Accepted
`session.state: busy` arms a completion. Assistant activity confirms it; a
successful or failed `turn.completed` settles for one second without background
work. Errors need no assistant activity. New activity cancels the timer.
Interruptions, snapshots, replay, reading, request answers and background-task
drainage never arm a new completion. Visible, focused conversations consume
their activity without notification. Comments remain Dock-only.

GitHub activity from the Cloud feed is the other source
([GitHub notifications](github-notifications.md)): only rows that arrive after the
first load since startup notify, under the `github` preference. Its title is the
headline ("reviewer approved") and its body `owner/repo#number`, plus the title when
already known; its `tab` is null.

Each delivery reads current preferences. Disabling an event does not change
the Dock or unread state. Turning notifications on never replays old activity.
Provider-specific payloads stay outside this path.

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

Styles are `banner`, `notch`, `none`; tones are `soft`, `digital`, `bell`.
Both notifications and sound start disabled. Missing, malformed or unknown-version
records use these defaults; wrong field types use that field's default. The
retired `prometeu:som` key is neither read nor rewritten. Preferences do not sync
between Macs. A failed save is reported and the controls restore saved values.

## Native delivery and IPC

`src/ipc.ts`, `src/mock.ts` and `src-tauri/src/notifications.rs` own matching
commands:

- `notification_permission({ request })`: returns `granted`, `denied`, `default`
  or `unavailable`. Only explicit user interaction requests authorization;
  opening Settings reads status without prompting.
  Enabling notifications with Banner selected or selecting Banner explicitly
  requests authorization again, after the initial status read finishes; a stale
  read cannot overwrite the authorization result. Unavailable or denied banners
  are shown directly below the master switch, separate from saved preferences.
- `notification_show({ notice })`: delivers `{ title, body, style, sound, tab,
  openLabel, closeLabel }`. `sound` is a tone or null; `tab` is a local tab ID
  or null for a test. Unknown enum values/fields and excessive text are rejected.
  Titles are bounded to 512 bytes, bodies to 2048, tab IDs to 128, and action
  labels to 256. Nonexistent or archived/cleaned targets are ignored.
- `notification_sound({ tone })`: previews an allowlisted system sound.
- `notification_current()`: initial content for the local overlay.
- `notification_dismiss()`: clears the notice and its target.
- `notification_open()`: closes the notice and opens its still-existing target.
- `island_enable({ enabled })`: main window only; shows or hides the island.
  The main window enables it while notifications are on with the `notch` style.
- `island_update({ snapshot })`: main window only; stores and relays the
  snapshot below. Snapshots above 512 KiB are rejected.
- `island_current()`: initial `{ snapshot, layout, notice }` for the overlay.
- `island_resize({ height })`: the overlay's rendered height while expanded,
  capped at 640 points.
- `island_open({ tab })`: opens a still-existing tab like `notification_open`
  and keeps the island closed until the pointer leaves it.

Permission, delivery and sound commands accept only the main window. A banner
uses macOS UserNotifications, with a retained delegate for foreground delivery
and clicks. Denied permission and native delivery failures reach the UI.
Clicking sends `notification-open` with the tab ID to the main window, which
revalidates the workspace before opening it. Agent notices carry only the
workspace title, not conversation or tool output; GitHub notices carry only the
metadata above, never comment text.

The notch style is a persistent island in a separate local Tauri window with a
dedicated built entry and event-listening capability
([ADR 0089](../decisions/0089-notch-island.md)). It renders text nodes, never
activates the application when shown, and accepts the first click. On macOS it
sits on the display with a notch, collapsed to the camera housing's height and
width plus a wing on each side for the flame and the count of waiting (or else
running) conversations; without a notch it sits inside the menu bar of the main
window's display. It is placed again whenever the display configuration
changes, because AppKit may move it or the notched display may change, and
whenever the main window moves, so on a display without a notch it follows the
window it anchors to. Its lower corners are clipped in AppKit as well as CSS; an
opaque rectangular window would otherwise cover the rounded web content.

The island expands to 560 points while the pointer is over it or while a notice
is showing, and returns to the housing otherwise. With the notch style, done and
error notices only play the selected sound: the island already recolors the
flame and shows a green signal for activity not yet looked at (the Dock's
pending state). Approval notices and GitHub notices expand it; a notice about a
conversation highlights its row, and only notices without one get a line. The backend samples the
pointer every 100 ms while the island is on because the webview receives no
pointer movement outside the key window. Opening a conversation keeps it
collapsed until the pointer leaves. One notice replaces the previous notice; it
clears after eight seconds. A generation check prevents an older timer from
clearing a replacement. Opening an actual conversation is the only action that
focuses the main window. The local page is not a new remote IPC surface.

The snapshot is built only in the main window from the board and live `chat`
events; replay never reaches it:

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

Tabs are local, active workspaces that are not stopped, most urgent first and
then most recent, at most twenty. `prompt`, `activity` and `request` are known
only from events received since the main window started; strings in tool
inputs are capped at 2000 characters (question text keys the answers and is
kept whole), and the backend answers from its own copy of the request. Request
panels are cached per tab and request, since provider request IDs are only
unique within one conversation. A failed answer shows its error in the panel
and leaves it ready to retry. Quota comes from the globally selected account of each
installed provider. The overlay answers through `chat_control` with compact
request panels: allow or deny a tool with a short preview of its edit or
command; approve a plan, or open the conversation to change it; and answer
questions one at a time, where one choice answers a single-choice question and
the last answer sends all of them. "Allow and stop asking" stays in the chat.
The backend's `request.closed` removes the request from every view.

Sound is independent of visual delivery and uses `/usr/bin/afplay` with three
fixed system files: Pop, Glass and Ping. It never accepts a filename or shell
command. System banners obey macOS notification settings. The custom overlay
and independently played sounds do not participate in macOS Focus filtering.
Sound tests and delivery report playback errors.

On Linux the same commands and payloads apply
([ADR 0055](../decisions/0055-linux-desktop.md)). Permission is `granted` when
`notify-send` is on PATH and `unavailable` otherwise; freedesktop has no per-app
authorization. A banner runs `notify-send --wait`, with a default action only
when the notice has a tab; the chosen action reaches the same `open_tab` path
as a macOS click. Without a notification daemon, `notify-send` fails at once and
delivery reports the unavailable error. Tones map to the freedesktop sound theme events
`message-new-instant`, `complete` and `bell`, played by `canberra-gtk-play` or,
failing that, by `pw-play` or `paplay` from the stock theme files. The notch
window is shown normally; the compositor decides its stacking.

## Verification boundary

`src/notifications.test.ts` covers compatibility, persistence, event/channel
selection and delivery failures. `src/alert.test.ts` covers deduplication,
visibility and eligibility. `e2e/notifications.spec.ts` exercises keyboard activation and control geometry
in Chromium and WebKit, plus live-notice navigation in Chromium. Native input
styles and focus-dependent navigation require a browser; preference compatibility
and event/channel selection remain in unit tests. Silent-default coverage stays
in `src/alert.test.ts`.
Rust tests validate payload limits; the IPC parity test checks registration.

The browser mock renders notices with the same view as the native overlay; it
does not request OS permission or play sound. With the `notch` style it also
shows the island at the top center, expanded on DOM hover.
`src/island.test.ts` covers the snapshot reducer, request lifecycle, input caps
and ordering. Notch geometry, pointer sampling, first-click delivery and
answering from the overlay require a bundled or development macOS app. Automated browser tests do not
prove Notification Center presentation, sound output, multi-monitor geometry,
or fullscreen/Focus interaction. Those require a bundled macOS app and native
manual verification. No live models are needed to use the Settings test button.
Under `tauri dev`, system banners and their authorization prompt are unavailable;
use `npm run app:bundle` for that native check. Notch and sound need no macOS
notification authorization and remain available during development.
The development bundle must be signed as a whole so its signing identifier
matches its bundle identifier; the executable's linker-only signature cannot
authorize notifications for the bundle. The development config selects ad-hoc
signing and the bundle launcher verifies it before opening.
