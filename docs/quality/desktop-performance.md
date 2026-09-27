# Desktop presentation measurements

Status: opt-in reproducible browser benchmark; no native performance claim.

## Workload

`npm run bench:desktop` builds the production frontend, starts a local Vite
preview on an available port, and launches headless Chromium. One warmup is
discarded, then five fresh browser contexts use the English UI at 1600×1000.
Four 550×350 conversation tiles are shown; the other sample tiles are hidden.
External HTTP requests are blocked. The browser, contexts and preview are closed
on success or failure; no provider, account or personal state is used.

- **Startup:** navigation start to the first desk conversation tile followed
  by two animation frames. This includes loading the existing mock sample board
  and its transcripts, not native binary startup or provider discovery.
- **Concurrent streams:** the four sample conversations `t1`, `t2`, `t3`, `t9`
  receive 100 text deltas each, interleaved across 20 animation-frame batches.
  Measurement ends after final blocks/completions and two animation frames.
  Every final message must be present in its real chat view. Uncaught page
  errors fail the measurement.
- **Maximum frame gap:** largest interval between batches during streaming.
  It includes scheduling and display cadence; it is not pure reducer CPU time.

The script uses existing mock event injection and production conversation
rendering. It adds no instrumentation to production user flows and no ordinary
browser test. Browser execution is necessary here to measure DOM rendering and
frame scheduling, which reducer unit tests do not exercise.

## Comparing revisions

Run on the same machine with the same Node/browser versions, viewport and
workload, keeping other load as steady as possible. `PROMETEU_BENCH_SAMPLES`
selects 3–30 measured samples; `PROMETEU_BENCH_OUTPUT` chooses the report path.
The default is `benchmark-results/desktop-performance.json` (ignored by Git and
kept outside Playwright's disposable results directory).

Reports retain raw samples, median/min/max, commit, dirty flag, build digest, browser version,
CPU, OS and workload metadata. Compare like-for-like medians and ranges; one
run or a result from another machine does not establish a regression. Browser
process and filesystem caches remain warm even though contexts are fresh.
There is no absolute timing gate in CI until stable comparable baselines exist.
The report fails if the production build changes during measurement. Keep the
build digest with dirty-worktree measurements so the artifact remains identified
even before a commit exists.

For native startup, energy, process launches or hidden-window behavior, use the
[native measurement protocol](energy-profile.md). This browser workload cannot
establish those properties, backend throughput or live provider latency.

## Initial measurement — 2026-09-27

The [raw report](../../fixtures/performance/desktop-browser.json) records five
samples after one warmup on Linux/WSL2, AMD Ryzen 5 7600, Node 24.0.1 and
Chromium 151.0.7922.34. It identifies the production bundle by SHA-256 and the
uncommitted implementation by base commit `115df1e` plus its dirty flag. Background
machine load was not controlled; this is a reproducibility record,
not a before/after improvement claim or an acceptance threshold. Node also
differs from the repository's declared version and must be matched when using
this particular record for comparison.

| Measurement | Median | Range |
| --- | --- | --- |
| Startup to desk paint | 219.9 ms | 202.7–238.4 ms |
| Four streams through final paint | 364.2 ms | 363.5–365.0 ms |
| Maximum frame gap per sample | 19.4 ms | 18.5–19.7 ms |
