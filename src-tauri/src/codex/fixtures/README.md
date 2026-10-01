# Codex fixtures

`process-peer.mjs` simulates an app-server process for lifecycle tests.

`read-only.ndjson` was recorded on 2026-09-28 from codex-cli 0.154.0 through
`app-server` with the read-only task switches (`--disable` for `plugins`,
`hooks`, `apps`, `computer_use` and `browser_use`, `-c mcp_servers={}` and
`-c mcp_servers.<name>.enabled=false` for each server in `config.toml`), after
`thread/start` with `sandbox: "read-only"` and `approvalPolicy: "never"`; the
response echoed `{"type":"readOnly","networkAccess":false}`. No MCP server
started. The prompt asked for `git diff --stat`, `touch`, a patch adding a file
and the available MCP tools. `git diff` ran; the write and the patch failed in
the sandbox with no server request, no file was created, and the agent reported
no MCP tools. Codex 0.154.0 does not report the sandbox-denied command and patch
as items; the agent's final message reports them. Only turn and item
notifications were kept; the thread identity became `t-1` and paths became
`/wt`.

The current adapter instead reads the effective MCP configuration through
`config/read` and disables those servers in the `thread/start` or
`thread/resume` config. The recording only verifies the resulting read-only
conversation events; the startup handshake is covered by adapter tests.
