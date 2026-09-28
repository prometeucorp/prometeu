# Codex fixtures

`process-peer.mjs` simulates an app-server process for lifecycle tests.

`read-only.ndjson` was recorded on 2026-09-28 from codex-cli 0.154.0 through
`app-server`, after `thread/start` with `sandbox: "read-only"` and
`approvalPolicy: "never"`; the response echoed
`{"type":"readOnly","networkAccess":false}`. The prompt asked for
`git diff --stat`, `touch` and a patch adding a file. `git diff` ran; the write
and the patch failed in the sandbox with no server request, and no file was
created. Codex 0.154.0 does not report the sandbox-denied command and patch as
items; the agent's final message reports them. Only turn and item notifications
were kept; the thread identity became `t-1` and paths became `/wt`.
