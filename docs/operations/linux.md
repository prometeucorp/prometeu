---
type: Operations
title: "Linux"
description: "How to build and install Prometeu on Linux, and the differences of the platform."
status: stable
---

# Linux

The release workflow of Prometeu makes a Linux x86_64 AppImage together with the
macOS download. You can also build from the source code or install the Arch
package. [ADR 0055](../decisions/0055-linux-desktop.md) records these decisions.
We did tests of builds from the source code on Arch Linux, Debian 12 and Ubuntu
24.04 with WebKitGTK 4.1. For the AppImage of each release, the native checks in the
[release procedure](release.md) continue to be necessary.

## Install the AppImage

1. Download `Prometeu_x86_64.AppImage` from the
   [latest release](https://github.com/prometeucorp/prometeu/releases/latest).
2. Keep the file in a directory that your user can write to.
3. Run these commands:

```sh
chmod +x Prometeu_x86_64.AppImage
./Prometeu_x86_64.AppImage
```

The workflow builds the AppImage on Ubuntu 22.04. This version sets the minimum
glibc version. It does not make sure that the AppImage operates on all Linux
distributions. If FUSE is not available, run
`./Prometeu_x86_64.AppImage --appimage-extract-and-run`. The desktop integrations
also use the optional system programs in the table below.

Each start of the AppImage adds Prometeu to your application menu. Thus, the
dock shows the icon of Prometeu and can pin it. The AppImage writes these files:

- `~/.local/share/applications/co.prometeu.desktop.desktop` starts this AppImage.
- `~/.local/share/icons/hicolor/256x256/apps/co.prometeu.desktop.png` contains
  the icon.

If `$XDG_DATA_HOME` has a value, it replaces `~/.local/share`. If you move the
AppImage file, Prometeu updates the menu entry at the next start. If you delete
the file, the menu does not show the entry. If AppImageLauncher or Gear Lever
shows the file in its list, Prometeu removes its own entry. A start with
`--appimage-extract-and-run` does not write an entry.

An AppImage installation uses the signed updater in the application. An
installation from a package manager or from the source code does not use it.
To update these installations, use the package manager or build again. At this
time, Linux ARM64 is only available as a build from the source code. This
workflow does not publish a `.deb` or `.rpm` package.

## Install on Arch Linux

[`packaging/arch/PKGBUILD`](../../packaging/arch/PKGBUILD) builds the committed
tree of the checkout that contains it. It installs the binary, the desktop entry
and the icons.

1. Check out the release tag that you want to install:

```sh
git checkout v<version>
cd packaging/arch
makepkg -si
```

The recipe does not download a source archive. Thus, there is no checksum to
examine. The source is the clone that you have. A recipe in the repository
cannot pin the checksum of a release archive that contains the recipe. An AUR
package can pin that archive as an alternative. To update, build the package
again. The updater in the application stays off for this type of installation.
Install `claude-code`, `codex` or `agy` with the same procedure as on macOS. The
optional programs enable the features in the table below.

## Install on Debian and Ubuntu

WebKitGTK 4.1 operates only on Debian 12, Ubuntu 22.04 or a subsequent version.
Build a `.deb` package from a checkout and install it. The apt package manager finds
and installs the runtime libraries that the package declares:

```sh
sudo apt install build-essential curl git pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev
npm ci
npx tauri build --bundles deb --config '{"bundle":{"createUpdaterArtifacts":false}}'
sudo apt install ./src-tauri/target/release/bundle/deb/Prometeu_*.deb
```

Use the versions of Node and Rust in `package.json` and
`src-tauri/rust-toolchain.toml`. Get Node from nodejs.org or nvm, and get Rust
from rustup. Build on the oldest release that you want to operate with. A binary
that you build with a newer glibc does not start with an older glibc. The
package `libnotify-bin` supplies `notify-send` for the banners.

## Development

```sh
sudo pacman -S --needed base-devel git rustup nodejs npm webkit2gtk-4.1 gtk3 librsvg
rustup default stable
npm ci
npm run app
```

`rustup` installs the toolchain that `src-tauri/rust-toolchain.toml` pins. If
there is no default toolchain, the Tauri CLI stops with a panic when it runs
`rustc -vV` out of `src-tauri`. Other distributions use the same development
packages for WebKitGTK 4.1, GTK 3 and librsvg. CI uses these Ubuntu packages:
`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev`.

## Platform differences

| Feature | macOS | Linux |
| --- | --- | --- |
| Open links, folders and Run | `open` | `xdg-open` |
| Keep awake | `caffeinate -d -i -s -w <pid>` | `systemd-inhibit --what=idle:sleep` around `tail --pid <pid>`. The display stays on only if the desktop obeys logind. There is no effect without systemd. |
| Banner notifications | UserNotifications (bundled app) | `notify-send --wait`. A click opens the conversation if the daemon can show actions. |
| Notch notice | Window at the status level | Usual window. The compositor selects the position. |
| Sound | `afplay` system sounds | `canberra-gtk-play`, then `pw-play` or `paplay` with the freedesktop theme |
| Feedback screenshot | `screencapture` | `slurp` + `grim` on Wayland. Not available on X11. |
| Claude usage credentials | File, then Keychain | `~/.claude/.credentials.json` |
| Device name | `scutil --get ComputerName` | Kernel hostname |
| Window | Overlay title bar | Native decorations |
| Dock icon and app menu | App bundle | The desktop entry of the package. The AppImage writes a user entry when it starts. |
| Shortcut names | ⌘ ⇧ ⌥ | Ctrl, Shift, Alt |
| Interface text | Mac, Finder, macOS | computer, file manager, system (`<key>.generic` variants) |
| Dictation, Finder file promises | Available | Not available |
| Updates | Updater in the application | Updater in the application for the AppImage. Package manager or a new build for other installations. |

If an optional program is not installed, the interface shows the same errors
as on macOS. These errors tell that a function is not available or has a failure. CI
builds the backend and runs the Rust tests and Clippy on Linux. Manual checks
of the desktop integration in a Linux session continue to be necessary.
