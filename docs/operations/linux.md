---
type: Operations
title: "Linux"
description: "Building, installing and platform differences on Linux."
status: stable
---

# Linux

Prometeu's release workflow produces a Linux x86_64 AppImage alongside the macOS
download. Source builds and the Arch package remain available. These choices
are recorded in [ADR 0055](../decisions/0055-linux-desktop.md).
Source builds have been tested on Arch Linux, Debian 12 and Ubuntu 24.04 with
WebKitGTK 4.1; each release's AppImage still needs the native checks in the
[release guide](release.md).

## Install the AppImage

Download `Prometeu_x86_64.AppImage` from the
[latest release](https://github.com/prometeucorp/prometeu/releases/latest).
Keep it in a directory writable by your user, then run:

```sh
chmod +x Prometeu_x86_64.AppImage
./Prometeu_x86_64.AppImage
```

The workflow builds on Ubuntu 22.04 to set the glibc baseline. This does not
guarantee compatibility with every Linux distribution. If FUSE is unavailable,
run `./Prometeu_x86_64.AppImage --appimage-extract-and-run`. The optional system
programs listed below are still needed for desktop integrations.

Each launch adds Prometeu to your application menu so the dock shows its icon
and can pin it: `~/.local/share/applications/co.prometeu.desktop.desktop` starts
this AppImage, and
`~/.local/share/icons/hicolor/256x256/apps/co.prometeu.desktop.png` holds the
icon (`$XDG_DATA_HOME` replaces `~/.local/share` when set). Moving the file
updates the entry on the next launch; deleting it hides the entry. If
AppImageLauncher or Gear Lever already lists the file, Prometeu removes its own
entry. Launches with `--appimage-extract-and-run` register nothing.

AppImage installations use the signed in-app updater. Package-managed and source
installations do not: update them through their package manager or rebuild.
Linux ARM64 currently requires a source build. No prebuilt `.deb` or `.rpm` is
published by this workflow.

## Install on Arch Linux

[`packaging/arch/PKGBUILD`](../../packaging/arch/PKGBUILD) builds the committed
tree of the checkout it lives in and installs the binary, desktop entry and
icons. Check out the release tag you want first:

```sh
git checkout v<version>
cd packaging/arch
makepkg -si
```

It downloads no source archive, so there is nothing to checksum: the source is
the clone you already have. A recipe inside the repository cannot pin the
checksum of a release archive that contains the recipe itself; an AUR package
would pin that archive instead. Updates come from rebuilding the package; the
in-app updater stays off for this installation type.
Install `claude-code`, `codex` or `agy` as on macOS. Optional programs enable
the features in the table below.

## Install on Debian and Ubuntu

WebKitGTK 4.1 requires Debian 12 or Ubuntu 22.04 or later. Build a `.deb` from
a checkout and install it; apt resolves the runtime libraries it declares:

```sh
sudo apt install build-essential curl git pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev
npm ci
npx tauri build --bundles deb --config '{"bundle":{"createUpdaterArtifacts":false}}'
sudo apt install ./src-tauri/target/release/bundle/deb/Prometeu_*.deb
```

Node and Rust come from the versions in `package.json` and
`src-tauri/rust-toolchain.toml` (nodejs.org or nvm, and rustup). Build on the
oldest release you target: a binary built on a newer glibc does not start on an
older one. `libnotify-bin` provides `notify-send` for banners.

## Develop

```sh
sudo pacman -S --needed base-devel git rustup nodejs npm webkit2gtk-4.1 gtk3 librsvg
rustup default stable
npm ci
npm run app
```

`rustup` installs the toolchain pinned in `src-tauri/rust-toolchain.toml`.
Without a default toolchain, the Tauri CLI fails with a panic while running
`rustc -vV` outside `src-tauri`. Other distributions need the same WebKitGTK
4.1, GTK 3 and librsvg development packages; CI uses Ubuntu's
`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev`.

## Platform differences

| Feature | macOS | Linux |
| --- | --- | --- |
| Open links, folders and Run | `open` | `xdg-open` |
| Keep awake | `caffeinate -d -i -s -w <pid>` | `systemd-inhibit --what=idle:sleep` around `tail --pid <pid>`; display wake depends on the desktop honoring logind; no effect without systemd |
| Banner notifications | UserNotifications (bundled app) | `notify-send --wait`; clicking opens the conversation when the daemon supports actions |
| Notch notice | status-level window | ordinary window; placement is up to the compositor |
| Sounds | `afplay` system sounds | `canberra-gtk-play`, then `pw-play` or `paplay` with the freedesktop theme |
| Feedback screenshot | `screencapture` | `slurp` + `grim` on Wayland; unavailable on X11 |
| Claude usage credentials | file, then Keychain | `~/.claude/.credentials.json` |
| Device name | `scutil --get ComputerName` | kernel hostname |
| Window | overlay title bar | native decorations |
| Dock icon and app menu | app bundle | the package's desktop entry; the AppImage writes a user entry at launch |
| Shortcut labels | ⌘ ⇧ ⌥ | Ctrl, Shift, Alt |
| Interface wording | Mac, Finder, macOS | computer, file manager, system (`<key>.generic` variants) |
| Dictation, Finder file promises | available | unavailable |
| Updates | in-app updater | in-app updater for AppImage; package manager or rebuild for other installs |

Missing optional programs produce the same unavailable or failure errors the
UI already shows on macOS. CI builds the backend and runs Rust tests and Clippy
on Linux; desktop integration still needs manual checks on a Linux session.

## WSL2

Under WSLg, the window opens on the Windows desktop like any other Linux GUI
app. If it stays blank, set `WEBKIT_DISABLE_DMABUF_RENDERER=1`. WSL has no
systemd by default, so keep awake has no effect, banners need a notification
daemon and links open in a Linux browser. This setup is not tested.
