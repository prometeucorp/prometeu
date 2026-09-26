# CI and release

## Commits

The repository uses Conventional Commits in English:

```text
type(scope): description
```

`feat`, `fix` and `perf` appear in the changelog. Features bump the minor;
backwards-compatible fixes, performance changes and reverts bump the patch. The
largest required bump wins when a release mixes them. During the 0.x series,
breaking changes also bump the minor; after 1.0 they bump the major. These rules
live in `cliff.toml`. The description is the public release line: write what the
person sees, in lowercase and without a trailing period.

The `.githooks/commit-msg` hook validates locally and the `commits` job checks
every commit of the PR.

## CI

`.github/workflows/ci.yml` runs on PRs, including from forks, and on pushes to
`main`, on GitHub-hosted runners. The macOS job installs dependencies, installs
Chromium and WebKit and runs `npm run check`. The `linux` job uses the same
Ubuntu 22.04 baseline as release, installs the WebKitGTK development packages,
builds the frontend and runs the Rust tests and Clippy, which cover the Linux
`cfg` branches; see [Linux](linux.md). Hosted runners are disposable and
the workflow has no secrets, so fork code runs without risk. Do not register a
self-hosted runner in this repository: the code is public and a fork's PR
controls what the job runs. See
[ADR 0040](../decisions/0040-open-source.md).

## Create a release

```sh
sh scripts/release.sh
sh scripts/release.sh 0.5.0
```

The script requires a clean tree, the `main` branch, parity with `origin/main`
and at least one public note since the previous tag. It computes or receives the
version, updates the manifests and the changelog, runs the tests, creates the
commit/tag and pushes to the remote.

The release workflow builds and signs both platforms into one draft in this
same repository, with the job's `GITHUB_TOKEN`. There is one version, tag and
changelog for the desktop app. There is no release PAT.

| Platform | Build runner | Install asset | Update asset |
| --- | --- | --- | --- |
| macOS Apple Silicon | `macos-latest` | `Prometeu_aarch64.dmg` | `Prometeu_aarch64.app.tar.gz` and `.sig` |
| Linux x86_64 | `ubuntu-22.04` | `Prometeu_x86_64.AppImage` | the same AppImage and `.sig` |

Linux targets the Ubuntu 22.04 build baseline; building on a newer runner can
raise the required glibc version. Other distributions still need a native
smoke test. Linux ARM64 and Intel Macs have no published binary in this workflow.

The AppImage must not carry `libwayland-*`: the runner's copies break EGL on
current Mesa and WebKitGTK aborts with `EGL_BAD_PARAMETER`, leaving a black
window. tauri-bundler cannot exclude libraries, so after the build
`scripts/appimage-unbundle-wayland.sh` deletes them, repacks with a pinned
appimagetool and runtime, and fails if any remain. The Linux job then signs the
repacked file again before uploading its workflow artifact. The final job reads
that signature when assembling `latest.json`.
Linux source and Arch package instructions remain in the [Linux guide](linux.md).

The macOS and Linux jobs build and sign independently, then upload separate
workflow artifacts with stable filenames. Each job checks that its commit
belongs to `main` before using signing credentials. Only the final job assembles
`latest.json` and writes the draft, after both jobs succeed. It verifies updater
signatures and Apple notarization before upload, then downloads the draft and
verifies them again. Reruns leave existing drafts untouched and verify their
downloaded assets; incomplete drafts fail and require manual removal before a
new run. Uploads never replace assets, even if someone publishes concurrently.
Runs for the same ref remain serialized.
The manifest and stable names are defined in the
[release contract](../contracts/releases.md).

CI and release share Rust cache keys per platform and Ubuntu baseline. This lets
new tags restore compatible dependency caches from `main`; caches scoped to a
previous tag are not reusable by the next tag. CI warms the test/Clippy profiles;
optimized release dependencies may still compile cold unless a compatible release
build has populated the default-branch cache. Both release jobs reuse their
validated frontend output by overriding Tauri's `beforeBuildCommand` only in CI.
Local `tauri build` retains its normal frontend build hook.

`workflow_dispatch` builds signed packages for both systems without creating a
release or tag. Its workflow artifacts expire after seven days. It does not
replace the final draft verification or the native installation checks.

## Publish

Between the build and the publication there is a human check:

1. download the `.dmg` and `.AppImage` from the draft;
2. install and open the DMG on an Apple Silicon Mac; make the AppImage executable
   and open it on a Linux x86_64 desktop;
3. validate the affected flows on both systems, including starting an agent,
   opening a terminal and links, and notifications when affected; when an older
   installation is available, also verify update download and restart;
4. confirm that both platform builds and final verification passed, all assets
   are present, and `latest.json` contains both platforms;
5. publish with:

```sh
sh scripts/release.sh publish
```

The script refuses to publish unless the macOS and Linux assets, their updater
signatures and `latest.json` exist and the release workflow succeeded. Native
installation remains a human check, not something an asset check proves.

Assets do not carry the version in their name. These links always select the
latest published release:

- macOS: `releases/latest/download/Prometeu_aarch64.dmg`;
- Linux: `releases/latest/download/Prometeu_x86_64.AppImage`.

The site's existing macOS link stays valid. Its separate repository must add a
Linux download button using the second path; adding assets here does not change
the site. The README links to both downloads.

macOS and AppImage installations check for updates at startup and hourly.
On Linux, the native Tauri bundle type enables the updater only for AppImage;
Arch, Debian, RPM and unpackaged builds keep external updates. Store the
AppImage in a directory writable by your user so the updater can replace it.

The updater has no rollback to a lower version. A release published with a
defect must be fixed by a later version.

## Keys

The private signing key never enters the repository. The local copy lives in
`~/.tauri/prometeu.key`; its password lives in the Keychain under the
`prometeu-tauri-signing` service. CI uses the `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets. Losing the local copy and the
secrets makes it impossible to update existing installations. Both platforms use
the same updater key. The Linux job receives only the updater secrets, not the
Apple credentials.

That signature protects the updater. Distribution on macOS also uses the
`Developer ID Application: Gustavo Brancaglione (6MQT6A482B)` certificate from
the Keychain and Apple notarization. CI imports a `.p12` copy into a temporary
Keychain using `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD`, adds that
Keychain to the search list without changing the Mac's default Keychain and then
removes it. Notarization uses `APPLE_ID` and `APPLE_PASSWORD`; the second
contains an app-specific password, never the normal Apple account password. The
job fails before the build if the certificate or the secrets are missing. The
bundler notarizes the app; the workflow notarizes and staples the final DMG,
replaces the asset created before that step and then uses `stapler` and `spctl`
to validate the copy downloaded from the draft.

Do not run a cut, tag, push or publication as part of an ordinary task without
an explicit request.

## Verification

`npm run test:release` tests manifest assembly from final signatures, compatibility,
missing assets, signature verification failures, refusal to overwrite published
assets and the publication gate with a fake `gh`. It never
publishes or contacts GitHub and is part of `npm test`. `src/update-init.test.ts`
covers AppImage eligibility and keeps package-managed Linux installs disabled;
`src/update.test.ts` covers download and restart behavior. The release workflow
performs the actual minisign verification against both downloaded packages.
