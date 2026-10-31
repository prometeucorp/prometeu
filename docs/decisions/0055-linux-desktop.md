---
type: Decision
title: "ADR 0055 — Linux desktop through system programs"
description: "Linux desktop through system programs."
status: stable
---

# ADR 0055 — Linux desktop through system programs

Date: 2026-09-23

## Context

Prometeu shipped only for macOS, but the backend already compiled on Linux:
AppKit, WebKit and UserNotifications code sat behind `cfg(target_os = "macos")`.
What failed on Linux was runtime behavior: macOS programs (`open`,
`caffeinate`, `afplay`, `screencapture`, `scutil`, `security`) and macOS-only
presentation. Arch Linux users asked for support.

## Options considered

- Tauri plugins for notifications, opener and shell. They add dependencies and
  IPC permissions for behavior the app already routes through its own commands,
  and the notification plugin cannot report clicks back on Linux.
- Linux D-Bus bindings (zbus) for notifications and logind. More precise, but a
  new async dependency for a handful of calls.
- Freedesktop command-line programs behind the existing command boundaries.
- For the AppImage's dock icon, leaving desktop integration to AppImageLauncher
  or Gear Lever. Without them, the dock shows a generic icon and the app cannot
  be pinned.

## Decision

Keep platform programs behind the app's command boundaries: `xdg-open`,
`systemd-inhibit` tied to the app PID through
`tail --pid`, `notify-send --wait` with a default action for clicks,
`canberra-gtk-play` with `pw-play`/`paplay` fallbacks, and `slurp` + `grim` on
Wayland. `src-tauri/src/platform.rs` holds the shared choices; features keep
their own validation and errors. Missing programs report the existing
unavailable or failure errors, so the frontend needs no new states. The later
[sleep policy](0061-background-energy-policy.md) adds a macOS-only system
assertion choice; Linux retains its original choices.

The frontend detects macOS from the user agent only for presentation. macOS
keeps its glyphs, app names and overlay title bar. Every other system gets a
native window frame instead of the reserved traffic-light space, key names in
shortcut labels (`Ctrl+Shift+D`), "computer" where the catalogs say "Mac", and
`<key>.generic` catalog variants for text naming macOS, Finder or System
Settings. Shortcut handlers already accepted Ctrl.

One GitHub release contains the macOS Apple Silicon DMG, Linux x86_64 AppImage
and experimental unsigned Windows installer. macOS and Linux use the same
version and updater signing key. Linux builds use
Ubuntu 22.04 as the glibc baseline. Platform jobs build and sign in parallel,
passing final files through workflow artifacts to one assembly job. That job
creates `latest.json` from the final signatures and is the only draft writer,
preventing concurrent manifest updates. It verifies files before upload and
again after downloading the draft. Reruns reject existing releases instead of
trusting assets from a potentially older commit of a moved tag. Drafts require
manual removal before rebuilding; published releases require a new version.
Uploads never replace assets, including during concurrent manual publication.
Publication requires all three platforms and verified macOS and Linux updater signatures.

The frontend uses Tauri's native `getBundleType()` to enable the Linux updater
only for AppImage. Arch `PKGBUILD`, Debian, RPM and source installations keep
external updates; an unknown bundle type also leaves updates disabled. This
preserves package-manager ownership without adding a custom IPC command.
The [release contract](../contracts/releases.md) preserves the macOS download
names, updater entry, endpoint and signing key.

Docks pair a window with the desktop entry whose `StartupWMClass` or file name
matches the window class; without one, GNOME draws a generic icon. Packages
install `Prometeu.desktop`, but a downloaded AppImage installs nothing, and
AppImageLauncher or Gear Lever are optional. Each launch from a mounted AppImage
therefore keeps a user entry, `co.prometeu.desktop.desktop`, and its 256 px
icon under `$XDG_DATA_HOME`. The entry starts that file, claims the window class
GTK derives from the program name, and is rewritten when the file moves. Its
`TryExec` hides it once the file is gone, and its distinct name never masks a
package's `Prometeu.desktop`. When another entry there already starts the same
file, Prometeu removes its own instead of listing the app twice.
`--appimage-extract-and-run` launches register nothing, since a menu could not
start the file again without FUSE.

## Trade-offs

Behavior depends on the desktop: notification daemons may ignore actions,
compositors decide where the notch window goes, and display wake depends on
logind support. X11 has no feedback capture. Finder file promises and
dictation stay macOS only. CI proves compilation, Clippy and Rust tests on
Linux, not desktop integration. Parallel release builds shorten the critical
path at the cost of workflow artifact transfer and an explicit manifest
assembly step. CI and release use the same Ubuntu baseline and compatible
cache keys; a newer Linux baseline needs separate validation and caches.
AppImage distribution starts with x86_64 only, and its Ubuntu baseline does not
prove compatibility with
every distribution. Native installation and update checks remain manual.
The AppImage writes outside Prometeu's data root without asking: a deleted
AppImage leaves a hidden entry and its icon behind, and with several copies the
entry starts whichever ran last.

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
