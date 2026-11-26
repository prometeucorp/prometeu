---
type: Decision
title: "ADR 0040 — Open source in a single public repository"
description: "The code, the CI and the releases are in one public repository, and CI uses only GitHub-hosted runners."
status: stable
---

# ADR 0040 — Open source in a single public repository

Date: 2026-09-11

This ADR specifies where the source and the releases of the Prometeu product
of [ADR 0004](0004-prometeu-independent-identity.md) are.

## Context

The code was in a personal private repository. CI and releases operated on a
self-hosted runner on this Mac. A workflow copied the releases to
`prometeucorp/prometeu-releases`, which is public, because public URLs are
necessary for the updater and the site. This had three costs:

- An organization PAT in the workflow.
- A second repository to update.
- A check that stopped the runner if the repository was not private.

The product becomes open source. When the code is public, the releases
repository has no purpose.

## Options considered

1. Open the code and keep `prometeu-releases` as the destination for releases.
2. Open the code in `prometeucorp/prometeu` and publish releases there.
3. Open the code and keep the self-hosted runner for CI.

## Decision

Use option 2. We moved the repository to `prometeucorp/prometeu` and made it
public. Releases, the `latest.json` of the updater and the download link of the
site point to it. `prometeucorp/prometeu-releases` is an archived repository.

CI and releases run on GitHub-hosted macOS and Linux runners. These runners are
free for a public repository. The project does not use a self-hosted runner. In a public
repository, the PR of a fork selects the `runs-on` value of its workflow. Thus,
it can run all code on this Mac. The release workflow uses the `GITHUB_TOKEN` of
the job with `contents: write`. There is no PAT.

One bridge keeps the update path. After the first release in the new
repository, we add the same `latest.json` as a release in `prometeu-releases`.
Installations from a previous version read the previous endpoint and download the package from the new
repository. After that, they send queries to the new endpoint, because that
version contains it. The minisign signature does not change.

## Consequences

Positive:

- One repository has the code, the issues, the releases and the history.
- The workflows contain no organization credential.
- CI operates for PRs from forks, and the machine of the maintainer is safe.
- The Mac is not a part of the build infrastructure.

Negative:

- A release build on a new runner uses more time than the incremental local
  build.
- After this change, the secrets for signing and notarization are on runners of a third party.
  Only the `release` environment can use them.
- `prometeu-releases` must continue to supply the bridge while installations
  that are older than this change continue to operate.

## Evidence

- `.github/workflows/ci.yml` and `release.yml` use hosted macOS and Linux
  runners. Releases use `GITHUB_TOKEN`.
- `src-tauri/tauri.conf.json` points the updater to `prometeucorp/prometeu`.
- `scripts/release.sh` publishes in the same repository.
- The `verify` job of the release validates `latest.json` with the public key of
  the app.
