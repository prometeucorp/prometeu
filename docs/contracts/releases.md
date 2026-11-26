---
type: Contract
title: "Desktop release and update contract"
description: "The desktop packages, the updater manifest and the ownership of the installation."
status: stable
---

# Desktop release and update contract

One version and one tag (`v<version>`) identify the macOS and Linux desktop
builds in `prometeucorp/prometeu`. A release stays a draft until these items
are complete: all builds, the signature checks for macOS and Linux, and the
native installation checks. The procedure for the team is in
[release](../operations/release.md). The decisions about the Linux packages are
in [ADR 0055](../decisions/0055-linux-desktop.md).

## Assets and manifest

The stable installation names are `Prometeu_aarch64.dmg` for macOS on Apple
Silicon and `Prometeu_x86_64.AppImage` for Linux x86_64. The same release also
has the macOS and Linux updater packages, their `.sig` files and `latest.json`.

The app keeps the endpoint
`https://github.com/prometeucorp/prometeu/releases/latest/download/latest.json`.
The manifest uses the static format of Tauri:

- `version` is the same as the tag without its `v` prefix.
- `notes` is the same as the changelog section of the version that the build job
  gets. Only the characters that are not visible at the start and at the end can
  be different.
- `platforms.darwin-aarch64` points to `Prometeu_aarch64.app.tar.gz`.
- `platforms.linux-x86_64` points to `Prometeu_x86_64.AppImage`.
- Each entry has a GitHub download `url` for its version and a `signature`.
  This value contains the minisign signature in base64. It is not a file path.
- Tauri also writes entries for each installer (`darwin-aarch64-app` and
  `linux-x86_64-appimage`). When these entries are in the manifest, they point
  to the same packages that the check examined.
- The entries for each installer are optional. [The target selection of Tauri
  2.10.1](https://github.com/tauri-apps/plugins-workspace/blob/d6a3898001a4bcc659e045f9501498751b77dbe6/plugins/updater/src/updater.rs#L567-L599)
  uses the generic `OS-ARCH` entry if the entry for the installer is not there.

The macOS and Linux builds use the current signing key. The Linux packages
only add to the release. The macOS asset names, the endpoint, the public key and
the platform entry stay compatible with the installed versions. The endpoint for
the latest release does not show drafts. To repair a defect in a published
release, publish a newer version.

## Assembly ownership

Each platform job uploads its last packages as a different workflow artifact.
One job assembles the manifest only after the two platform jobs complete
correctly. It uses the signatures adjacent to the last files. This includes the
AppImage that the job puts in a new package. The job writes the generic entries
and the entries for each installer. It examines the local artifacts before the
upload and the downloaded draft after the upload.

A second run of the job stops with an error if the release is there. After a
tag moves, the signatures, the version and the notes cannot prove which commit
made an older draft. To build again, a person must delete the draft manually.
An upload does not replace a current asset. This rule also applies when a
manual publication and the first draft check occur at the same time. The
public format, the asset names, the URLs and the updater key do not change.

## Installation ownership

The macOS installations and the Linux AppImage installations look for an update
at startup and one time each hour. The person must start the download and the
new start of the app. Tauri examines the signature before the installation. The
user must have write permission for the AppImage files.

The installed version starts to operate at the next start. Thus, it is not
necessary to start the app again to complete an update. **Restart to update**
shows a question first if a new start will stop local work. Local work is these
items:

- Conversations that work or wait.
- Workspaces that the app prepares.
- Terminals that run a command.

The question gives the number of each item. It also tells that the version
starts to operate at the next start. If the person cancels, the update stays
available and the app does not start again. If there is no local work to stop,
the app starts again immediately. See
[ADR 0097](../decisions/0097-ask-before-ending-the-app.md).

On Linux, the built-in Tauri command `plugin:app|bundle_type` selects this
behavior. No application IPC payload changes. An `appimage` result enables the
updater. Debian, RPM, Arch and builds without a package keep updates external.
If the bundle type is unknown or its detection gives an error, the app also
disables the updater. The browser mock returns `appimage` and no available
update. Thus, UI tests do not download packages.

## Verification

`scripts/verify-release.py --assemble` makes the manifest from the two signed
updater packages. The verifier examines these items:

- The changelog notes.
- The macOS and Linux updater targets.
- The accurate URLs for each version.
- All installation assets.
- The agreement between the signature files and the manifest.
- The minisign signatures against the embedded key.

`scripts/test_release.py` includes tests for these conditions:

- Missing platforms or assets.
- Incorrect versions or URLs.
- Signature failures.
- Draft assembly.
- A refusal to replace published assets.
- A refusal to publish builds that are not complete or that have a failure.

`src/update-init.test.ts` keeps the ownership of the package manager and the
macOS behavior. `src/update.test.ts` includes tests for the download, the
question before the new start and the new start. CI cannot prove the native
desktop integration. CI also cannot prove the replacement and the new start of
the installed app. The smoke test of the release is
necessary for these items.
