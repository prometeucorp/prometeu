---
type: Operations
title: "Repository presentation"
description: "The origin of the demo, the social preview and the GitHub checks."
status: stable
---

# Repository presentation

The README shows the product, the demo, the latest downloads and the request
for a star before the data about the features. Keep the top section small. Keep
the platform qualification adjacent to the downloads. The first page of the
repository continues to show the file tree of GitHub above the README. The
README cannot change that layout.

## Media

- [Demo GIF](../demo.gif): 20 seconds, 1280 × 820, 15 fps, it starts again at
  the end, less than 10 MB.
  - We recorded it from the browser interface of the product in English. It uses
    `src/mock.ts` with two synthetic tasks and Claude/Codex conversation events.
  - It shows two agents that work side by side, each on the branch of its
    workspace. It opens one conversation, sends a follow-up prompt and goes back
    to the desk.
  - The captions and the simulation label are out of the application window.
    The last frames change gradually into the first frame.
  - The GIF identifies the agents as a simulation. It is not evidence of
    native operation, of the performance of a provider or of changes to a
    repository.
- [Social preview](../social-preview.png): 1280 × 640 PNG, less than 1 MB.
  It contains the current logo, the tagline and a capture of the browser
  interface.

### Refresh the demo

1. Install the usual development dependencies, the Chromium of Playwright and
   FFmpeg.
2. Make sure that FFmpeg is on `PATH`, or that `FFMPEG` gives the path of its
   executable.
3. Run these commands:

```sh
npx playwright install chromium
npm run demo:record
```

The [recorder](../../scripts/record-demo.mjs) starts its own loopback Vite server
and an isolated browser context. It uses the current mock and the controls of the product
interface. It does not connect to providers and it does not read local projects.
It replaces `docs/demo.gif`. It also writes PNG frames, an MP4 and a local
`preview.html` in the ignored `test-results/demo/` directory. You can share the
MP4 in locations other than the README. The recorder operates without a new npm
dependency and without a capture mode in the product.

The recorder makes sure of these conditions:

- The two agents are fully on the screen.
- The follow-up prompt gets to the conversation.
- The completed response comes into view.
- No uncaught browser errors occur.
- The GIF stays less than 10 MB.

These checks validate the capture. They do not validate the native operation of
an agent. Examine the first shot, the open conversation and the last shot at the
880 px display width of the README. Keep the synthetic content and the
simulation label. Do not capture credentials, private projects or the conversations
of persons at any time.

To configure the social preview, do these steps:

1. In GitHub, go to **Settings → General → Social preview → Edit →
   Upload an image**.
2. Upload the PNG.

A commit of the PNG alone does not configure the preview. GitHub gives the
[upload procedure and the image limits](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/customizing-your-repositorys-social-media-preview).

## Verification

1. Run `npm run docs:check`.
2. Make sure that the GIF starts again at the end in the README page of GitHub.
3. Make sure that the GIF is not wider than the page.
4. Make sure that the download links are immediately below the GIF.
5. Open the two `/releases/latest/download/` links.
6. Make sure that each response is satisfactory.
7. After the merge, open
   [Community Standards](https://github.com/prometeucorp/prometeu/community).
8. Make sure that the contributing, license, README, security, issue-template
   and PR-template entries are correct.

The larger display makes the captions easy to read. Thus, the full
introduction does not go into one viewport. The file tree of GitHub stays above
the README. The [release contract](../contracts/releases.md) includes the
stable asset names of the download links.

Private vulnerability reporting must stay enabled. If a person disables it, the
link in `SECURITY.md` does not operate.
