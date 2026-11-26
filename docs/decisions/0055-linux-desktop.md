---
type: Decision
title: "ADR 0055 — Linux desktop through system programs"
description: "The Linux desktop uses system programs."
status: stable
---

# ADR 0055 — Linux desktop through system programs

Date: 2026-09-23

## Context

Prometeu released builds only for macOS. But the compilation of the backend on
Linux was correct at that time, because the AppKit, WebKit and UserNotifications code was behind
`cfg(target_os = "macos")`. On Linux, the problem was the runtime behavior. The
app used macOS programs (`open`, `caffeinate`, `afplay`, `screencapture`,
`scutil`, `security`) and a presentation for macOS only. Arch Linux users sent
requests for Linux builds.

## Options considered

- Tauri plugins for notifications, opener and shell. They add dependencies and
  IPC permissions for behavior that the app sends through its own commands at
  this time. Also, on Linux the notification plugin cannot send clicks back to
  the app.
- Linux D-Bus bindings (zbus) for notifications and logind. This is more
  accurate, but it adds a new async dependency for a small number of calls.
- Freedesktop command-line programs behind the current command boundaries.
- For the dock icon of the AppImage, let AppImageLauncher or Gear Lever do the
  desktop integration. Without them, the dock shows a general icon, and the
  person cannot pin the app.

## Decision

Keep the platform programs behind the command boundaries of the app:

- `xdg-open`
- `systemd-inhibit`, connected to the PID of the app through `tail --pid`
- `notify-send --wait` with a default action for clicks
- `canberra-gtk-play`, with `pw-play`/`paplay` as fallbacks
- `slurp` + `grim` on Wayland

`src-tauri/src/platform.rs` contains the shared selections. Each feature keeps
its own validation and errors. If a program is missing, the feature returns the
current "unavailable" or failure errors. Thus new states in the frontend are not
necessary. The subsequent [sleep policy](0061-background-energy-policy.md) adds a
system assertion for macOS only. Linux keeps its initial selections.

The frontend finds macOS from the user agent, only for the presentation. macOS
keeps its glyphs, app names and overlay title bar. All other systems get these
items:

- a native window frame, not the space for the traffic-light buttons
- key names in shortcut labels (`Ctrl+Shift+D`)
- "computer" where the catalogs show "Mac"
- the `<key>.generic` catalog variants for text that names macOS, Finder or
  System Settings

The shortcut handlers accepted Ctrl before this change.

One GitHub release contains the macOS Apple Silicon DMG and the Linux x86_64
AppImage. The two platforms use the same version and the same updater signing
key. Linux builds use Ubuntu 22.04 as the glibc baseline. The release workflow
does these steps:

1. The platform jobs build and sign in parallel. They send the last files to
   one assembly job through workflow artifacts.
2. That job makes `latest.json` from the last signatures. It is the only job
   that writes the draft. Thus two jobs cannot update the manifest at the same
   time.
3. The job examines the files before the upload. It examines them again after it
   downloads the draft.

A new run rejects a release that is there before. It does not trust assets from
an older commit of a tag that a person moved. Before a new build, a person must
manually remove a draft. A published release must have a new version. Uploads
do not replace assets, also when a person publishes manually at the
same time. To publish, the release must have the two platforms and correct
macOS and Linux updater signatures.

The frontend uses the native `getBundleType()` function of Tauri. It enables the
Linux updater only for the AppImage. Arch `PKGBUILD`, Debian, RPM and source
installations keep their external updates. An unknown bundle type also keeps
the updates disabled. Thus the package manager continues to control updates,
and Prometeu does not add a custom IPC command. The
[release contract](../contracts/releases.md) keeps the macOS download names,
the updater entry, the endpoint and the signing key.

A dock connects a window to a desktop entry. The `StartupWMClass` or the file
name of that entry must agree with the window class. Without such an entry,
GNOME shows a general icon. Packages install `Prometeu.desktop`. But a
downloaded AppImage does not install a desktop entry, and AppImageLauncher and Gear Lever are optional. Thus, each time the
app starts from an AppImage, it keeps a user entry,
`co.prometeu.desktop.desktop`, and its 256 px icon under `$XDG_DATA_HOME`. This
entry has these properties:

- It starts that file. It claims the window class that GTK makes from the
  program name.
- Prometeu writes it again when the file moves.
- Its `TryExec` removes it from the menus when the file is not there.
- Its name is different, so it does not hide the `Prometeu.desktop` of a
  package.

If a different entry in that folder starts the same file, Prometeu removes its
own entry. Thus the app does not show two times in the list. A start with
`--appimage-extract-and-run` records no entry. The cause is that a menu cannot
start the file again without FUSE.

## Trade-offs

The behavior is different on each desktop:

- Notification daemons can ignore actions.
- Compositors select the position of the notch window.
- The display wakes only if logind can do it.

X11 has no feedback capture. Finder file promises and dictation stay on macOS
only. CI proves the compilation, Clippy and the Rust tests on Linux. It does not
prove the desktop integration.

Parallel release builds make the release faster. But the workflow must move
artifacts between jobs, and it has an explicit step that assembles the manifest.
CI and the release use the same Ubuntu baseline and compatible cache keys. A
newer Linux baseline must have its own validation and caches. At first,
Prometeu supplies the AppImage only for x86_64. Its Ubuntu baseline does not
prove compatibility with all Linux distributions (the Linux products of different companies and projects). The checks of the native installation and
of updates stay manual.

The AppImage writes files that are not in the data root of Prometeu. It does
not get the approval of the person first. If the person deletes the AppImage, a hidden entry and its icon
stay. If there are many copies, the entry starts the copy that ran last.

## Evidence

- [Linux guide and platform table](../operations/linux.md).
- [Platform helpers](../../src-tauri/src/platform.rs).
- [Sleep inhibitor test](../../src-tauri/src/awake.rs).
- [Freedesktop notifications](../../src-tauri/src/notifications/freedesktop.rs).
- [AppImage desktop entry and its tests](../../src-tauri/src/desktop_entry.rs).
- [Shortcut label tests](../../src/platform.test.ts).
- [Updater installation tests](../../src/update-init.test.ts).
- [Release gate tests](../../scripts/test_release.py) and
  [release workflow](../../.github/workflows/release.yml).
