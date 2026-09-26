# Issue #148 energy profiling

Status: measurement protocol and source-level baseline for commit `b03e768`.
The native release-build comparison belongs to the implementation review; the
figures below do not establish a battery-life change.

## Reproduce the comparison

1. Build the before and after revisions with `npm run build:app`; run the native
   bundle, not Vite or the browser mock. Record the Git SHA, bundle version,
   macOS/Linux version, power source, display brightness, external displays,
   selected sleep preference, thermal conditions, account/provider count,
   workspace/repository count and visible panels. Use an isolated
   `PROMETEU_ROOT` containing the same fixture state for both builds. Never
   publish credentials, local paths or raw process samples.
2. Repeat each scenario at least three times for equal intervals after a warm-up:
   idle foreground, hidden/minimized idle, one active streaming conversation,
   several visible desk conversations, Files and Changes visible, and one
   explicitly enabled PR monitor. Run on AC and battery when the hardware state
   is available. Change only the app revision between paired runs.
3. For each run, record main-process, WebKit and provider CPU separately;
   wakeups; app-started `ps`, provider, `git` and `gh` process launches; disk and
   network work; and sleep assertions (`pmset -g assertions` on macOS). Use
   Activity Monitor Energy Impact or controlled battery discharge when available.
   Record unavailable counters as unavailable, rather than extrapolating them
   from process CPU or wall time.
4. Capture time from focus/return/panel opening to refresh dispatch and to a
   usable result. A network or CLI timeout is separate from the scheduling
   budget. Confirm transcript, terminal, sharing and opt-in PR work continue
   while discretionary UI work is reduced.

## Source-level baseline at `b03e768`

| Work | Current trigger | Approximate idle frequency or cost |
| --- | --- | --- |
| Detailed process sampling | `machine.rs::watch` | One `ps` per 3 seconds while the app runs, about 1,200 per hour. |
| Account identity/quota probes | `usage.rs::watch` | Every registered profile is probed sequentially, followed by a 60-second sleep; actual per-profile spacing also includes probe duration. |
| Antigravity quota child wait | `antigravity.rs::discover` | Completion check every 20 ms, up to 10 seconds per invocation. |
| General PR discovery | `main.ts` | At startup and every 60 seconds. Explicit monitoring is a separate backend schedule. |
| Visible Changes and Files | `workspace.ts`, `tree.ts` | Each has a five-second fallback; workspace board redraws can add status requests. |
| Full status for one repository | `session/git.rs` | Multiple Git subprocesses per request; the issue's throwaway benchmark measured ten read-only commands at 165.1 ms median wall time and 82.5 ms mean child CPU on its repository. |

These frequencies are read from the source, not measured energy. The
[investigation in issue #148](https://github.com/prometeucorp/prometeu/issues/148)
observed sleep assertions and a short CPU sample on a different running
release, with the limitations described there.

## Local baseline attempt, 2026-09-25

The `b03e768` release binary was compiled on macOS 26.5.2 and launched with an
isolated `PROMETEU_ROOT`. The machine was connected to AC; the battery was
charging at 68%. Three five-second `top` samples of the isolated main process
reported 0.0%, 0.0% and 0.1% CPU and about 27 MB resident memory. The system
load average was about 25, with other Prometeu and unrelated processes active.
`pmset -g assertions` showed no assertion attributable to this isolated app;
its saved sleep preference was off. The native UI did not have a controlled
workload, so these readings are **not** comparable scenario measurements.

`npm run build:app` produced a release binary but exited with an updater
packaging error because `TAURI_SIGNING_PRIVATE_KEY` is unavailable. This
Command Line Tools installation also lacks Instruments' `xctrace`. Battery
runs, WebKit/provider CPU, process-launch counts, wakeups and Energy Impact
were unavailable for this baseline. The source-level counts above and focused
launch-count tests are the current comparison points; a controlled native
before/after energy measurement remains required before claiming a battery
gain.

## Acceptance record

For each implementation slice, add a dated comparison with its two Git SHAs,
build and workload description, three run values with median/range, observed
assertions, refresh latency, and any deferred candidate. Check these outcomes
independently of Energy Impact:

- Hidden idle performs no detailed process or Git scans.
- Account and general-PR probes respect their documented background budgets;
  the explicit PR monitor retains its configured interval.
- Returning to the app queues stale refreshes immediately and preserves the
  last valid quota reading during provider failures.
- A machine reading changes only the resource display, not unrelated footer
  controls.
- The selected sleep assertion matches the saved preference and the actual
  local agent state, including native background children.

Do not report a battery-life gain unless a controlled energy or discharge
comparison supports it.

## Sleep-assertion check, 2026-09-25

On AC power, a short isolated `caffeinate -i -s -w <pid>` run produced
`PreventUserIdleSystemSleep` and `PreventSystemSleep`, without
`PreventUserIdleDisplaySleep`. Adding `-d` produced all three. Both children
were terminated after inspection. The macOS native test in `awake.rs` verifies
the app chooses these exact flags and releases the child on mode change and
normal shutdown. Battery assertions remain unverified on this machine.

## Quota scheduling verification

The implementation keeps `usage.json` unchanged and schedules each registered
profile separately. The controlled-clock tests in `usage_scheduler.rs` count
due tickets, the point where a provider probe is launched: on foreground AC,
one selected profile is due after 60 seconds while one inactive profile is not
due until 600 seconds. On battery these intervals are 120 and 1,200 seconds;
when hidden they are 900 and 1,800 seconds. Failures back off per profile and
an old in-flight generation cannot publish after removal, reconnection or a
newer live event. These deterministic counts are a source-level comparison,
not a native process-launch trace or an energy measurement. A native provider
fixture with identical account count remains necessary for before/after launch
counts.

## Resource sampling verification

The `machine.rs` fake-clock tests count sample eligibility before `/bin/ps`
launches. A foreground compact display is eligible every 15 seconds on AC
(240 per hour) or 30 seconds on battery (120 per hour), versus the original
three-second loop (1,200 per hour). An open panel keeps the previous
three-second AC cadence or uses five seconds on battery. A hidden or unfocused
window has no detailed sample deadline and refreshes when it returns. A gap
over six seconds resets the CPU history; CPU percentage still divides the
cumulative CPU delta by actual elapsed time. These counts assume the app
remains in each state for the whole hour and are **not** measured `ps` launches
or watts. A native process-launch trace was unavailable in the baseline.

`e2e/statusbar.spec.ts` covers a browser-specific menu-anchor and process-row
preservation risk: replacing footer DOM after a resource event would detach the
open sleep menu's button and reset panel state. Rust unit tests can prove
sampling deadlines but cannot prove browser node identity.

## Git refresh verification

The Changes view now schedules fallback scans every five seconds on foreground
AC or ten seconds on battery. Visible Files marks use 15 or 30 seconds. Both
pause while hidden or unfocused and refresh on return. Board redraws that do
not change the workspace or repository identity no longer request status. Files
marks reuse the porcelain snapshot that a Changes status read within the last
second, while per-worktree single-flight prevents overlapping scans. Changes
always reads its own porcelain between its index fingerprints, so the commit
token matches the listed files; an invalidation during a scan forces follow-up
scans before returning its result.
Native Git actions and app file saves invalidate immediately. Real-repository
tests cover an external edit after the fallback, concurrent consumers,
conflicts and multi-repository status. These tests count Git invocations in a
controlled fixture; a comparable native multi-repository process trace is
unavailable from the baseline, so no measured energy reduction is claimed.

No `fswatch`, `inotifywait` or `watchman` tool is available in this environment.
A watcher would also need to follow the resolved gitdir, index, branch and
worktree on both macOS and Linux. Without a comparable idle-cost and
correctness benchmark, the visible fallback remains the implementation.

## General PR discovery verification

The source-level general scan interval changes from 60 seconds (up to 60
scheduled attempts per hour) to three minutes on foreground AC (up to 20),
five minutes on foreground battery (up to 12) and 15 minutes while hidden or
unfocused (up to four). Opening a workspace and returning to foreground add
immediate requests. A request during a slow scan queues only one follow-up;
one clone is queried once per general scan. Rust tests cover the gate, an
aborted CLI process, archived/cleaned exclusion, and preservation of known PR
metadata. The explicit task monitor retains its original cadence and richer
queries. These are scheduler limits, not measured `gh` launches or watts; a
controlled fixture with the same clone count and authenticated `gh` state is
still needed for a native before/after comparison.

## WebKit streaming and preview profile

A production Vite build in Playwright WebKit streamed 60 Markdown deltas after
an initial roughly 6 KiB block, one frame apart, first in a workspace and then
to two visible desk conversations. Temporary in-app performance marks counted
flushes, Markdown parses, piece grouping, composer and comment-pin paints, and
the preview's body observer callbacks. The marks and profiling-only test were
removed after measurement. Each row is one run on the same AC machine under
uncontrolled load; timing is diagnostic rather than a statistically comparable
energy result. This browser fixture is not the native WKWebView release bundle.

| Scenario and measure | Before Task 7 | After Task 7 |
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

Markdown parsing was 23% of summed workspace flush time in the initial fixture,
and piece grouping was under 1%; neither justified a riskier incremental parser
or timeline cache. The post-change desk timing increased despite removing work,
which reinforces the need for repeated controlled native measurements. A WebKit
journey verifies that a token update keeps the send icon connected and that
hidden transcript content catches up on return. The existing preview WebKit
journeys cover opening, dialogs, resizing and workspace lifecycle.

## Final implementation verification, 2026-09-25

The source-level starting point is `b03e768`; the implementation through
streaming/preview is `c7c888c`, followed by the hidden-Git guard and file-tree
redraw correction in this change. The final `npm run check` passed: 515 web unit
tests, 464 Rust unit tests (seven ignored), the two Rust IPC-parity tests, 172
Chromium/WebKit E2E scenarios, documentation/architecture/format checks, web
and mobile builds, and the configured Rust Clippy gate. Five additional WebKit
repetitions passed the file-save/tree-row regression after canceling a redundant
delayed redraw.
`npm run build:app -- --no-bundle` produced the optimized native binary;
signing and updater packaging still require the private updater key.

The deterministic checks cover account removal during a probe, native child
settlement for sleep assertions, a long resource-sampling gap, external Git
edits and concurrent status consumers, general-PR scan overlap and timeout,
and preview/dialog and hidden-transcript journeys. They do not replace a paired
native process-launch or energy log. The Task 0 baseline had no controlled
battery run, equal authenticated account fixtures or usable Instruments trace;
the machine remained on AC under unrelated load. No native before/after watts,
Energy Impact or discharge comparison is available, so this issue has no
verified battery-life gain. A release review should repeat the worksheet above
on an otherwise idle machine with equivalent fixture state on AC and battery.
