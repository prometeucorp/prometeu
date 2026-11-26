---
type: Quality
title: "Issue #148 energy profiling"
description: "The measurement protocol and the source baseline at b03e768. It does not claim a measured change in battery life."
status: stable
---

# Issue #148 energy profiling

This page gives the measurement protocol and the baseline from the source code
for commit `b03e768`. To compare native release builds is a task is part of the
implementation review. The values below do not show a change in battery life.

## Reproduce the comparison

1. Build the revisions before and after the change with `npm run build:app`.
2. Run the native bundle. Do not use Vite or the browser mock.
3. Record the Git SHA, the bundle version and the macOS or Linux version.
4. Record the power source, the display brightness and the external displays.
5. Record the selected sleep preference and the thermal conditions.
6. Record the number of accounts, providers, workspaces and repositories.
7. Record the panels that are open.
8. Use an isolated `PROMETEU_ROOT` with the same fixture state for the two
   builds.
9. Do not publish credentials, local paths or raw process samples.
10. After a warm-up, do each scenario a minimum of three times for equal
    intervals.

    The scenarios are:

    - The app is idle in the foreground.
    - The app is idle and hidden or in the Dock.
    - One conversation streams.
    - Many desk conversations are open.
    - The Files and Changes views are open.
11. If the hardware state lets you, run on AC power and on battery.
12. Between paired runs, change only the revision of the app.
13. For each run, record the CPU of the primary process, WebKit and the
    provider independently.
14. Record the wakeups.
15. Record the launches of `ps`, provider, `git` and `gh` processes that the app
    starts.
16. Record the disk and network work.
17. Record the sleep assertions. On macOS, use `pmset -g assertions`.
18. If available, use Energy Impact in Activity Monitor or a controlled battery
    discharge.
19. If a counter is not available, record it as not available. Do not calculate
    it from the process CPU or the clock time.
20. Measure the time from a focus, a return or an open panel to the refresh
    request.
21. Measure the time from the same action to a result that a person can use.

A network or CLI timeout is different from the scheduling budget. Make sure that
the transcript, terminal, sharing and optional PR work continue while the app
does less optional interface work.

## Source-level baseline at `b03e768`

| Work | Current trigger | Approximate idle frequency or cost |
| --- | --- | --- |
| Detailed process sampling | `machine.rs::watch` | One `ps` each 3 seconds while the app runs. This is approximately 1,200 each hour. |
| Account identity/quota probes | `usage.rs::watch` | The app examines each profile in the list, one after the other. Then it waits 60 seconds. The time between the probes of a profile also includes the time of each probe. |
| Antigravity quota child wait | `antigravity.rs::discover` | A completion check each 20 ms, for a maximum of 10 seconds for each call. |
| General PR discovery | `main.ts` | At startup and each 60 seconds. |
| Visible Changes and Files | `workspace.ts`, `tree.ts` | Each view has a fallback of five seconds. When the workspace board renders again, it can add status requests. |
| Full status for one repository | `session/git.rs` | Many Git subprocesses for each request. A temporary benchmark of the issue measured ten read-only commands on its repository. The median clock time was 165.1 ms. The mean CPU time of the child processes was 82.5 ms. |

We read these frequencies from the source code. They are not energy
measurements. The
[investigation in issue #148](https://github.com/prometeucorp/prometeu/issues/148)
recorded sleep assertions and a short CPU sample on a different release that
operated at that time. That page gives the limits of these data.

## Local baseline attempt, 2026-09-25

We compiled the `b03e768` release binary on macOS 26.5.2. We started it with an
isolated `PROMETEU_ROOT`. The machine was on AC power, and the battery charge was
68%. Three `top` samples of five seconds each for the isolated primary process
showed 0.0%, 0.0% and 0.1% CPU. They showed approximately 27 MB of resident
memory. The system load average was approximately 25. Other Prometeu processes
and processes that are not related were active.

`pmset -g assertions` did not show an assertion from this isolated app. The
saved sleep preference of the app was off. The native interface did not have a
controlled workload. Thus, these values are **not** scenario measurements that
you can compare.

`npm run build:app` made a release binary. But it stopped with an error in the
updater packaging, because `TAURI_SIGNING_PRIVATE_KEY` was not available. This
Command Line Tools installation also does not have `xctrace` from Instruments.
These measurements were not available for this baseline:

- Runs on battery.
- The CPU of WebKit and of the provider.
- The number of process launches.
- The wakeups.
- Energy Impact.

At this time, the comparison points are the counts from the source code above
and the focused tests of launch counts. A controlled native energy measurement
before and after the change continues to be necessary before a claim of better
battery life.

## Acceptance record

For each part of the implementation, add a comparison with a date. Include these
items:

- The two Git SHAs.
- The description of the build and the workload.
- Three run values with the median and the range.
- The assertions that you saw.
- The refresh latency.
- Each candidate that you did not do.

Examine these results independently of Energy Impact:

- When the app is hidden and idle, it does no detailed process or Git scans.
- The account probes and the general PR probes obey their documented
  background budgets.
- When a person goes back to the app, the app immediately queues the refreshes
  that are not current. During provider failures, it keeps the last correct quota
  value.
- A machine value changes only the resource display. It does not change footer
  controls that are not related.
- The selected sleep assertion agrees with the saved preference and with the
  state of the local agents. This includes native background child processes.

Do not report better battery life if a controlled energy or discharge comparison
does not show it.

## Sleep-assertion check, 2026-09-25

On AC power, a short isolated run of `caffeinate -i -s -w <pid>` made
`PreventUserIdleSystemSleep` and `PreventSystemSleep`. It did not make
`PreventUserIdleDisplaySleep`. With `-d`, it made all three assertions. We
stopped the two child processes after the inspection. The native macOS test in
`awake.rs` makes sure that the app selects these flags. It also makes sure that
the app releases the child process when the mode changes and at a usual
shutdown. We did not examine the assertions on battery on this machine.

## Quota scheduling verification

The implementation does not change `usage.json`. It schedules each profile in the
list independently. The tests in `usage_scheduler.rs` use a controlled clock.
They count the due tickets. A due ticket is the point where the app launches a
provider probe. The intervals are these:

| Condition | Selected profile | Profile that is not active |
| --- | ---: | ---: |
| Foreground, AC power | 60 seconds | 600 seconds |
| Battery | 120 seconds | 1,200 seconds |
| Hidden | 900 seconds | 1,800 seconds |

After a failure, the interval of that profile increases. A previous generation that
operates at that time cannot publish after a removal, a new connection or a
newer live event. These deterministic counts are a comparison from the source
code. They are not a trace of native process launches or an energy measurement.
For counts of launches before and after the change, a native provider fixture
with the same number of accounts continues to be necessary.

Login lifecycle regression tests also include these conditions:

- A probe operates when the login starts.
- A cancellation or a failure without a revision change makes the profile
  immediately eligible.

The selected profiles and the profiles that are not active resume. An older ticket cannot
complete the replacement probe. The start of a login and each exit from a login
start the scheduler again after the pending state agrees. Thus, after a
satisfactory login, the scheduler does not wait for the fallback deadline of the
watcher.

## Resource sampling verification

The `machine.rs` tests use a fake clock. They count when a sample is eligible,
before the launch of `/bin/ps`. The intervals are these:

| Condition | Interval | Samples each hour |
| --- | ---: | ---: |
| Initial loop | 3 seconds | 1,200 |
| Small display in the foreground, AC power | 15 seconds | 240 |
| Small display in the foreground, battery | 30 seconds | 120 |
| Open panel, AC power | 3 seconds | 1,200 |
| Open panel, battery | 5 seconds | 720 |

A hidden window or a window without focus has no deadline for a detailed sample.
It refreshes when it comes back. The native context starts the sampler again
directly, without a round trip through the webview.

If the time between two samples is more than two times the interval, the CPU history
starts again. Thus, the small display keeps its samples, and the panel is not
empty when it opens. The CPU percentage continues to divide the cumulative CPU
change by the clock time.

The app publishes the terminal and port counts when one of these events occurs:

- A dock PTY starts or stops.
- The assigned port of a workspace changes.

This publication does not use a `ps` sample. A port-assignment regression test
examines the publication after the app saves and releases the board lock. It
also examines the replacement of a port that the browser blocks. For these counts,
the app stays in each state for the full hour. They are **not**
measured `ps` launches or watts. A trace of native process launches was not
available in the baseline.

`e2e/statusbar.spec.ts` examines a risk that only a browser can show. It is
about the menu anchor and the process rows. A resource event can cause a change to the
footer DOM. Then the button of the open sleep menu can disconnect. Also, the
panel state can go back to its initial value. Rust unit tests can show the sampling
deadlines. But they cannot show the identity of browser nodes.

## Git refresh verification

At this time, the Changes view schedules fallback scans each five seconds in the
foreground on AC power. On battery, it schedules them each ten seconds. The Files
marks use 15 or 30 seconds. The two views pause while they are hidden or without
focus. They refresh when they come back.

When the board renders again and the identity of the workspace or repository
does not change, the views do not request status. The Files marks use the
porcelain snapshot that a Changes status read in the last second. A single-flight
lock for each worktree prevents scans at the same time.

The Changes view always reads its own porcelain between its index fingerprints.
Thus, the commit token agrees with the listed files. If an invalidation occurs
during a scan, the view does more scans before it returns its result. Native Git
actions and file saves in the app cause an invalidation immediately.

Tests with git repositories on the disk include an external edit after the fallback,
consumers at the same time, conflicts and status for many repositories. These
tests count Git calls in a controlled fixture. The baseline does not have a
native process trace for many repositories that you can compare. Thus, we do not
claim a measured decrease in energy.

No `fswatch`, `inotifywait` or `watchman` tool is available in this environment.
A watcher must also follow the resolved gitdir, index, branch and worktree on
macOS and on Linux. There is no benchmark of the idle cost and the correctness
that you can compare. Thus, the fallback for open views stays the implementation.

## General PR discovery verification

The interval of the general scan in the source code changes from 60 seconds to
these values:

| Condition | Interval | Maximum scheduled scans each hour |
| --- | ---: | ---: |
| Before the change | 60 seconds | 60 |
| Foreground, AC power | 3 minutes | 20 |
| Foreground, battery | 5 minutes | 12 |
| Hidden or without focus | 15 minutes | 4 |

When a person opens a workspace, the app immediately adds a request for the
repositories of that workspace. A turn that stops in the open workspace also adds
a request. These requests occur a maximum of one time each 20 seconds for each
workspace. Board updates record short turns also when menus or rename inputs
stop the rendering for a short time. One refresh stays for the next permitted render.

When the app comes back to the foreground, it immediately adds a general request
only if the last request occurred one minute ago or before. Thus, if a person changes
between apps frequently, the app adds a maximum of one scan each minute. A
request during a slow scan queues only one more scan. The app examines one clone
only one time in each general scan.

Rust tests examine the gate and a CLI process that stops before the end. They
also examine the exclusion of archived and cleaned workspaces and the PR
metadata that the app keeps. These values are limits of the scheduler. They are not measured `gh`
launches or watts. To compare the native app before and after the change, a
controlled fixture is necessary. It must have the same number of clones and the
same authenticated `gh` state.

## WebKit streaming and preview profile

We made a production Vite build and ran it in Playwright WebKit. After an initial
block of approximately 6 KiB, the test streamed 60 Markdown deltas, one frame
apart. It did this first in a workspace, then in two open desk conversations.

Temporary performance marks in the app counted these items:

- Flushes.
- Markdown parses.
- Piece grouping.
- Paints of the composer and of the comment pins.
- The body observer callbacks of the preview.

We removed the marks and the profiling test after the measurement. Each row is
one run on the same machine on AC power, with a load that we did not control.
The timing is for diagnosis. It is not an energy result that you can compare
statistically. This browser fixture is not the native WKWebView release bundle.

| Scenario and measure | Before `c7c888c` | After `c7c888c` |
| --- | ---: | ---: |
| Workspace, flushes and summed duration | 61 / 286 ms | 61 / 227 ms |
| Workspace, Markdown parses and summed duration | 61 / 67 ms | 61 / 70 ms |
| Workspace, piece-grouping summed duration | 2 ms | below 1 ms |
| Workspace, composer / pin paints | 61 / 61 | 1 / 1 |
| Workspace, body observer callbacks with preview closed | 64 | 0 |
| Two desk conversations, flushes and summed duration | 122 / 541 ms | 122 / 566 ms |
| Two desk conversations, Markdown parses and summed duration | 122 / 96 ms | 122 / 138 ms |
| Two desk conversations, composer / pin paints | 122 / 122 | 2 / 2 |
| Two desk conversations, body observer callbacks | 122 | 0 |

In the initial fixture, Markdown parsing used 23% of the summed flush time of
the workspace. Piece grouping used less than 1%. Thus, these results did not
justify an incremental parser or a timeline cache, which have more risk. After
the change, the desk timing increased, although the change removed work. This
result shows again that more controlled native measurements are necessary.

A WebKit journey makes sure that a token update keeps the send icon connected.
It also makes sure that hidden transcript content becomes current when the
person comes back. The current preview journeys in WebKit examine the opening,
the dialogs, the resize operations and the workspace lifecycle.

## Final implementation verification, 2026-09-25

The starting point in the source code is `b03e768`. The implementation through
streaming and preview is `c7c888c`. After that, this change added the guard for
hidden Git work and the correction for the file-tree render.

The last `npm run check` was satisfactory. It included these items:

- 515 web unit tests.
- 464 Rust unit tests, with seven ignored tests.
- The two Rust IPC-parity tests.
- 172 E2E scenarios in Chromium and WebKit.
- The documentation, architecture and format checks.
- The web and mobile builds.
- The configured Rust Clippy gate.

Five more WebKit runs of the regression test for the file save and the tree row
were satisfactory. These runs occurred after the change that cancels an
second render that is not necessary. `npm run build:app -- --no-bundle` made the
optimized native binary. The signature and the updater packaging continue to
use the private updater key.

The deterministic checks include these conditions:

- The removal of an account during a probe.
- The end of native child processes for sleep assertions.
- A long interval without resource samples.
- External Git edits and status consumers at the same time.
- General PR scans that overlap, and a timeout.
- The preview and dialog journeys and the hidden transcript journeys.

These checks do not replace a paired native log of process launches or energy.
The baseline in `969fe64` did not have a controlled battery run, equal
authenticated account fixtures or a usable Instruments trace. The
machine stayed on AC power with a load that was not related. There is no native
comparison of watts, Energy Impact or discharge before and after the change.
Thus, this issue does not show a measured improvement of battery life. A
release review must do the procedure above again on an idle machine. Use the
same fixture state, on AC power and on battery.
