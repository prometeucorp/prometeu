---
type: Operations
title: "Repository presentation"
description: "Demo provenance, social preview and GitHub verification."
status: stable
---

# Repository presentation

The README puts the product, demo, latest downloads and star request before
feature details. Keep the top section compact and retain the experimental
Windows qualification beside the downloads. The first repository page still
shows GitHub's file tree above the README; the README cannot change that layout.

## Media

- [Demo GIF](../demo.gif): 20 seconds, 1280 × 820, 15 fps, looping, under 10 MB.
  Captured from the real browser interface in English, using `src/mock.ts`
  with two synthetic tasks and Claude/Codex conversation events. It shows two
  agents working side by side, separate workspace branches, opening a focused
  conversation, sending a follow-up and returning to the desk. Captions and the
  simulation label sit outside the app window. The closing frames blend back
  into the opening shot.
  The recording identifies the simulated agents; it is not evidence of native
  execution, provider performance or actual changes to a repository.
- [Social preview](../social-preview.png): 1280 × 640 PNG, under 1 MB,
  composed from the existing logo, tagline and a browser interface capture.

### Refresh the demo

Install the normal development dependencies, Playwright's Chromium and FFmpeg.
FFmpeg must be on `PATH`, or `FFMPEG` must name its executable. Then run:

```sh
npx playwright install chromium
npm run demo:record
```

The [recorder](../../scripts/record-demo.mjs) starts its own loopback Vite server
and an isolated browser context. It uses the existing mock and real UI controls;
it does not connect to providers or read local projects. It replaces `docs/demo.gif`
and writes PNG frames, an MP4 and a local `preview.html` under the ignored
`test-results/demo/` directory. The MP4 is useful for sharing outside the README.
No new npm dependency or product capture mode is needed.

The recorder checks that both agents fit on screen, the follow-up reaches the
conversation, the completed response appears, no uncaught browser errors occur
and the GIF stays under 10 MB. These checks validate the capture, not native
agent execution. Inspect the opening, focused conversation and closing shots
at the README's 880 px display width. Keep synthetic content and the simulation
label; never capture credentials, private projects or real conversations.

Upload the PNG in GitHub **Settings → General → Social preview → Edit →
Upload an image**. Committing the PNG alone does not configure the preview.
GitHub documents the [upload procedure and image limits](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/customizing-your-repositorys-social-media-preview).

## Verification

Run `npm run docs:check`. Check that the GIF loops inline in GitHub's README
rendering, scales without horizontal overflow and has the download links
immediately below it. The larger display favors readable captions over fitting
the entire introduction into one viewport. GitHub's file tree remains above
the README.

Follow all three `/releases/latest/download/` links and confirm successful
responses. Their stable asset names are part of the
[release contract](../contracts/releases.md).

After merging, inspect [Community Standards](https://github.com/prometeucorp/prometeu/community)
and verify the contributing, license, README, security, issue-template
and PR-template entries. Private vulnerability reporting must stay enabled for
the link in `SECURITY.md` to work.
