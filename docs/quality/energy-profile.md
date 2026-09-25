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
not change the workspace or repository identity no longer request status. A
shared raw porcelain snapshot serves compatible Changes and Files requests
within one second, while per-worktree single-flight prevents overlapping scans;
an invalidation during a scan forces a follow-up before returning its result.
Native Git actions and app file saves invalidate immediately. Real-repository
tests cover an external edit after the fallback, concurrent consumers,
conflicts and multi-repository status. These tests count Git invocations in a
controlled fixture; a comparable native multi-repository process trace is
unavailable from the baseline, so no measured energy reduction is claimed.

No `fswatch`, `inotifywait` or `watchman` tool is available in this environment.
A watcher would also need to follow the resolved gitdir, index, branch and
worktree on both macOS and Linux. Without a comparable idle-cost and
correctness benchmark, the visible fallback remains the implementation.
