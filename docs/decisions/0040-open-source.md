# ADR 0040 — Open source in a single public repository

Date: 2026-09-11
Status: Accepted

Defines source and release hosting for the independent product in
[ADR 0004](0004-prometeu-independent-identity.md).

## Context

The code lived in a personal private repository, with CI and releases on a
self-hosted runner on this Mac. Releases were copied to
`prometeucorp/prometeu-releases`, which is public, because the updater and the
site need public URLs. That required an organization PAT in the workflow, a
second repository to maintain and a guard that refused the runner if the
repository stopped being private.

The product becomes open source. With the code public, the releases repository
loses its reason to exist.

## Options considered

1. Open the code and keep `prometeu-releases` as the destination for releases.
2. Open the code in `prometeucorp/prometeu` and publish releases there.
3. Open the code while keeping the self-hosted runner for CI.

## Decision

Option 2. The repository is transferred to `prometeucorp/prometeu` and becomes
public. Releases, the updater's `latest.json` and the site's download link point
to it. `prometeucorp/prometeu-releases` is archived.

CI and releases run on GitHub-hosted runners (macOS and Linux; CI also uses
Windows), free for a public repository. A self-hosted runner is forbidden: in a
public repository, a fork's PR chooses its own workflow's `runs-on`, and with
that it would run arbitrary code on this Mac. The release workflow uses the job's `GITHUB_TOKEN` with
`contents: write`; there is no PAT.

The update path is preserved by a single bridge: after the first release
published in the new repository, the same `latest.json` is added as a release in
`prometeu-releases`. Old installations read the old endpoint, download the
package from the new repository, and from then on query the new endpoint, which
is already embedded in that version. The minisign signature does not change.

## Consequences

Positive:

- a single repository for code, issues, releases and history;
- no organization credential in the workflows;
- CI for fork PRs without exposing the maintainer's machine;
- the Mac stops being build infrastructure.

Negative:

- a release build on a cold runner takes longer than the incremental local one;
- the signing and notarization secrets now live on third-party runners, still
  restricted to the `release` environment;
- `prometeu-releases` must keep serving the bridge while installations older
  than this change exist.

## Evidence

- `.github/workflows/ci.yml` uses hosted macOS, Linux and Windows runners,
  `windows-installer.yml` hosted Linux/Windows runners and `release.yml` hosted
  macOS/Linux runners; releases use `GITHUB_TOKEN`;
- `src-tauri/tauri.conf.json` points the updater to `prometeucorp/prometeu`;
- `scripts/release.sh` publishes in the same repository;
- the release's `verify` job validates `latest.json` against the app's public
  key.
