---
type: Operations
title: "CI and release"
description: "CI, versions and the release procedure."
status: stable
---

# CI and release

## Commits

The repository uses Conventional Commits in English:

```text
type(scope): description
```

The types `feat`, `fix` and `perf` show in the changelog. The version changes
as follows:

- A feature increases the middle number of the version.
- A compatible correction, a performance change or a revert increases the last
  number of the version.
- If a release has more than one type of change, the largest of these changes
  to the version applies.
- In the 0.x series, a change that is not compatible also increases the middle
  number. After 1.0, a change that is not compatible increases the first
  number.

These rules are in `cliff.toml`. The description is the public line of the
release. Write what the person sees, in lowercase and without a period at the
end.

The `.githooks/commit-msg` hook validates each message on the local computer.
The `commits` job examines each commit of the pull request.

## CI

`.github/workflows/ci.yml` runs on pull requests, also from forks, and on
pushes to `main`. It runs on runners that GitHub hosts:

- `macos` installs Chromium and WebKit and runs `npm run check`.
- `linux` uses the same Ubuntu 22.04 baseline as the release. It installs the
  WebKitGTK development packages. It runs `npm run build`,
  `npm run terminal-state:check`, `npm run test:rust` and `npm run lint:rust`.
  These commands include the Linux `cfg` branches. Refer to [Linux](linux.md).
- `commits` examines each commit message of the pull request.

The `test:rust` and `lint:rust` runs on macOS and Linux include all workspace
crates. `npm run architecture:check` in `npm run check` examines the limits of
the crate graph. GitHub discards the hosted runners after each job, and the
workflow has no secrets. Thus, the code of a fork runs without risk. Do not
add a self-hosted runner in this repository. The code is public, and the
pull request of a fork controls what the job runs.
Refer to [ADR 0040](../decisions/0040-open-source.md).

## Create a release

```sh
sh scripts/release.sh
sh scripts/release.sh 0.5.0
```

The script starts only under these conditions:

- The tree has no local changes.
- The branch is `main`, and it is the same as `origin/main`.
- There is a minimum of one public note after the previous tag.

The script calculates or gets the version. It updates the manifests and the
changelog and runs the tests. Then it makes the commit and the tag and pushes
them to the remote.

The release workflow builds the two platforms into one draft in this
repository. It uses the `GITHUB_TOKEN` of the job. The desktop application has
one version, one tag and one changelog. There is no release PAT.

| Platform | Build runner | Install asset | Update asset |
| --- | --- | --- | --- |
| macOS Apple Silicon | `macos-latest` | `Prometeu_aarch64.dmg` | `Prometeu_aarch64.app.tar.gz` and `.sig` |
| Linux x86_64 | `ubuntu-22.04` | `Prometeu_x86_64.AppImage` | The same AppImage and `.sig` |

The Linux build uses the Ubuntu 22.04 baseline. A build on a newer runner can
increase the necessary glibc version. For other distributions, a native smoke test
continues to be necessary. This workflow does not publish a binary for Linux ARM64 or
for Intel Macs.

The AppImage must not contain `libwayland-*`. The copies from the runner cause
EGL failures on current Mesa versions. Then WebKitGTK stops with
`EGL_BAD_PARAMETER` and shows a black window. tauri-bundler cannot remove
libraries from the bundle. Thus, after the build, `scripts/appimage-unbundle-wayland.sh` does
these steps:

1. It deletes the libraries.
2. It makes the AppImage again with a pinned appimagetool and runtime.
3. It stops with a failure if one of the libraries stays in the AppImage.

Then the Linux job signs the new file again and uploads its workflow artifact.
The last job reads that signature when it assembles `latest.json`. The
instructions for the Linux source build and the Arch package are in the
[Linux guide](linux.md).

The macOS and Linux jobs upload different workflow artifacts with stable file
names. Each job that uses signing credentials makes sure that its commit is on
`main`. Only the last job assembles `latest.json` and writes the draft. It
does this after all builds are satisfactory. The manifest contains the updater
entries for macOS and Linux.

The last job validates the updater signatures and the Apple notarization
before the upload. Then it downloads the draft and validates them again. A second run
stops with a failure if a release for the version is there. Thus, a moved tag
cannot use the binaries of an older draft without a warning. To build again,
remove the draft manually. A published release must have a new version. An upload
does not replace an asset at any time, also when a different person publishes
at the same time. GitHub runs the jobs for the same ref one after the other.
The [release contract](../contracts/releases.md) specifies the manifest and the
stable names.

CI and the release use the same Rust cache keys for each platform and Ubuntu
baseline. Thus, a new tag can use compatible dependency caches from `main`.
A cache that belongs to a previous tag is not available to the next tag. CI
prepares the caches for the test and Clippy profiles. The build of the optimized release
dependencies can start without a cache. This does not occur if a compatible
release build filled the cache of the default branch before.

The macOS and Linux release jobs use the frontend output that they validated
before. To do this, they override the `beforeBuildCommand` of Tauri only in CI.
A local `tauri build` keeps its usual frontend build hook.

The macOS build uses `npm run build:app`. This command does these steps:

1. It installs the private terminal helper from its lockfile.
2. It validates the pinned Node archive.
3. It bundles the SEA and the third-party notices through an explicit Tauri
   overlay.

Tauri signs the nested executable together with the application. Before the
DMG notarization, the workflow validates these items:

- The full signature.
- The accurate notice files.
- The protocol of the packaged helper with an empty PATH.

The helper operates without an installed Node and without more Apple
entitlements. At this
time, the Linux release does not package the helper. macOS enables the native
conversation mode. On Linux, a different platform gate keeps the mode
hidden and rejects terminal launches. The gate does this for all packaged
resources.

`workflow_dispatch` builds packages for the two systems. It does not make a
release or a tag. Its workflow artifacts expire after seven days. It does not
replace the last verification of the draft or the native installation checks.

## Publish

A person examines the release between the build and the publication:

1. Download the `.dmg` and the `.AppImage` from the draft.
2. Install and open the DMG on an Apple Silicon Mac.
3. Make the AppImage executable and open it on a Linux x86_64 desktop.
4. Make sure that the dock on Linux shows the Prometeu icon.
5. On the two systems, validate the flows that the release changes.
6. Include the start of an agent and the start of a terminal.
7. Include links. If the release changes notifications, include notifications.
8. If an older installation is available, validate the update download and the
   start after the update.
9. Make sure that all platform builds and the last verification were
   satisfactory.
10. Make sure that all assets are in the draft.
11. Make sure that `latest.json` contains the updater entries for macOS and
    Linux.
12. Publish with this command:

```sh
sh scripts/release.sh publish
```

The script does not publish if one of these items is missing:

- The assets of the two platforms.
- The updater signatures for macOS and Linux.
- `latest.json`.
- A satisfactory run of the release workflow.

The native installation stays a check that a person does. An asset check cannot
show it.

The names of the assets do not include the version. These links always select
the latest published release:

- macOS: `releases/latest/download/Prometeu_aarch64.dmg`.
- Linux: `releases/latest/download/Prometeu_x86_64.AppImage`.

The current macOS link of the site stays correct. The site is in a different
repository, and that repository must add a Linux download button. New assets in
this repository do not change the site. The README has links to the two
downloads.

The macOS and AppImage installations look for updates at startup and each hour.
On Linux, the native Tauri bundle type enables the updater only for the
AppImage. The Arch, Debian and RPM packages and the builds without a package
use external updates. Keep the AppImage in a directory that your user can write
to. Then the updater can replace it.

The updater cannot go back to a lower version. If a published release has a
defect, a subsequent version must repair it.

## Keys

The private signing key is not in the repository at any time. The local copy is
in `~/.tauri/prometeu.key`. Its password is in the Keychain, in the
`prometeu-tauri-signing` service. CI uses the `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets. If you do not have the local copy and the
secrets, you cannot update the current installations.

The two platforms use the same updater key. The Linux job gets only the updater
secrets. It does not get the Apple credentials.

That signature is only for the updater. On macOS, the application also uses Apple
notarization and the
`Developer ID Application: Gustavo Brancaglione (6MQT6A482B)` certificate from
the Keychain. CI uses `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD` to
import a `.p12` copy into a temporary Keychain. It adds that Keychain to the
list of Keychains that macOS examines. It does not change the default Keychain of the Mac. At the end, it
removes the temporary Keychain.

Notarization uses `APPLE_ID` and `APPLE_PASSWORD`. The second secret contains an
app-specific password. It does not contain the usual password of the Apple
account at any time. If the certificate or the secrets are missing, the job
stops with a failure before the build. The bundler notarizes the application.
The workflow notarizes and staples the last DMG. It replaces the asset that it
made before that step. Then it uses `stapler` and `spctl` to validate the copy
that it downloads from the draft.

Product analytics use the `PROMETEU_POSTHOG_KEY` repository variable. It is not
a secret, because a PostHog project key can only write events, and the
application contains it. The release builds put the key into the
application. Without the key, the application does not send analytics. Refer to
the [contract](../contracts/product-analytics.md).

Do not cut, tag, push or publish a release as part of a usual task. Do these
operations only when a person tells you to do them.

## Verification

`npm run test:release` uses a fake `gh` to test these items:

- The assembly of the manifest from the last signatures.
- Compatibility.
- Missing assets.
- Failures of the signature verification.
- The refusal to overwrite published assets.
- The publication gate.

This test does not publish and does not connect to GitHub at any time. It is
part of `npm test`. `src/update-init.test.ts` examines which AppImage
installations can update, and keeps the updater disabled for Linux installations
from a package manager. `src/update.test.ts` examines the download and the
start after the update. The release workflow does the minisign verification on the two
downloaded packages.
