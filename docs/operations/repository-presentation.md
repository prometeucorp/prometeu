# Repository presentation

The README puts the product, demo, latest downloads and star request before
feature details. Keep the top section compact and retain the experimental
Windows qualification beside the downloads. The first repository page still
shows GitHub's file tree above the README; the README cannot change that layout.

## Media

- [Demo GIF](../demo.gif): 15 seconds, 1200 × 650, looping, under 10 MB.
  Captured from the real browser interface in English, using `src/mock.ts`
  with two synthetic tasks and Claude/Codex conversation events. It shows
  sending both tasks, switching workspaces, reviewing a diff, resolving an
  incoming change and completing the merge through the Git controls.
  The recording identifies the simulated agents; it is not evidence of native
  execution, provider performance or actual changes to a repository.
- [Social preview](../social-preview.png): 1280 × 640 PNG, under 1 MB,
  composed from the existing logo, tagline and the same interface capture.

To refresh the demo, run `npm run dev`, use synthetic project and conversation
data in the browser mock, and capture the above sequence in English. Do not
capture credentials, private projects or real conversations. Keep the mock
label visible, encode a looping 10–20 second GIF under 10 MB, and inspect the
first, review and merge frames. Keep captions outside interactive controls.

Upload the PNG in GitHub **Settings → General → Social preview → Edit →
Upload an image**. Committing the PNG alone does not configure the preview.
GitHub documents the [upload procedure and image limits](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/customizing-your-repositorys-social-media-preview).

## Verification

Run `npm run docs:check`. Check that the GIF loops inline in GitHub's README
rendering and the logo through the star request fit a 1366 × 768 viewport when
the README begins. GitHub's file tree remains outside that measurement.

Follow all three `/releases/latest/download/` links and confirm successful
responses. Their stable asset names are part of the
[release contract](../contracts/releases.md).

After merging, inspect [Community Standards](https://github.com/prometeucorp/prometeu/community)
and verify the conduct, contributing, license, README, security, issue-template
and PR-template entries. Private vulnerability reporting must stay enabled for
the link in `SECURITY.md` to work.
