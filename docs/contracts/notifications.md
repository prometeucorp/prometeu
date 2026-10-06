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
- `notification_dismiss()`: closes the overlay and clears its target.
- `notification_open()`: closes the overlay and opens its still-existing target.

Permission, delivery and sound commands accept only the main window. A banner
uses macOS UserNotifications, with a retained delegate for foreground delivery
and clicks. Denied permission and native delivery failures reach the UI.
Clicking sends `notification-open` with the tab ID to the main window, which
revalidates the workspace before opening it. Agent notices carry only the
workspace title, not conversation or tool output; GitHub notices carry only the
metadata above, never comment text.

The notch is a separate local Tauri window with a dedicated built entry and
event-listening capability. It renders text nodes, stays at the top center of
the main window's display and does not activate the application when shown.
Its 360-by-96-point surface reserves the top strip for the camera and clips its
lower corners in AppKit as well as CSS; an opaque rectangular window would
otherwise cover the rounded web content.
It also works on displays without a physical notch. One notice replaces the
previous notice; it disappears after eight seconds. A generation check prevents
an older timer from dismissing a replacement. Opening an actual conversation
is the only action that focuses the main window. The local page is not a new
remote IPC surface.

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
does not request OS permission or play sound. Automated browser tests do not
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
