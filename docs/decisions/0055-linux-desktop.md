# ADR 0055 — Linux desktop through system programs

Date: 2026-09-23
Status: Accepted

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

## Decision

Keep every IPC command and payload unchanged and pick platform programs in the
backend: `xdg-open`, `systemd-inhibit` tied to the app PID through
`tail --pid`, `notify-send --wait` with a default action for clicks,
`canberra-gtk-play` with `pw-play`/`paplay` fallbacks, and `slurp` + `grim` on
Wayland. `src-tauri/src/platform.rs` holds the shared choices; features keep
their own validation and errors. Missing programs report the existing
unavailable or failure errors, so the frontend needs no new states.

The frontend detects macOS from the user agent only for presentation. macOS
keeps its glyphs, app names and overlay title bar. Every other system gets a
native window frame instead of the reserved traffic-light space, key names in
shortcut labels (`Ctrl+Shift+D`), "computer" where the catalogs say "Mac", and
`<key>.generic` catalog variants for text naming macOS, Finder or System
Settings. Shortcut handlers already accepted Ctrl.

One GitHub release contains the macOS Apple Silicon DMG and Linux x86_64
AppImage, using the same version and updater signing key. Linux builds use
Ubuntu 22.04 as the glibc baseline. Platform jobs build and sign in parallel,
passing final files through workflow artifacts to one assembly job. That job
creates `latest.json` from the final signatures and is the only draft writer,
preventing concurrent manifest updates. It verifies files before upload and
again after downloading the draft. Reruns cannot replace published assets.
Publication requires both platforms and verified updater signatures.

The frontend uses Tauri's native `getBundleType()` to enable the Linux updater
only for AppImage. Arch `PKGBUILD`, Debian, RPM and source installations keep
external updates; an unknown bundle type also leaves updates disabled. This
preserves package-manager ownership without adding a custom IPC command.
The [release contract](../contracts/releases.md) preserves the macOS download
names, updater entry, endpoint and signing key.

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

## Evidence

- [Linux guide and platform table](../operations/linux.md).
- [Platform helpers](../../src-tauri/src/platform.rs).
- [Sleep inhibitor test](../../src-tauri/src/awake.rs).
- [Freedesktop notifications](../../src-tauri/src/notifications/freedesktop.rs).
- [Shortcut label tests](../../src/platform.test.ts).
- [Updater installation tests](../../src/update-init.test.ts).
- [Release gate tests](../../scripts/test_release.py) and
  [release workflow](../../.github/workflows/release.yml).
