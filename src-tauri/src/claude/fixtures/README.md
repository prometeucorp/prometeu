# Claude stream fixtures

`read-only.ndjson` was recorded on 2026-09-28 from Claude Code 2.1.283 with the
application's stream-json flags (including `--include-partial-messages`) and the
read-only task flags: `--restricted --permission-mode dontAsk --tools
Read,Grep,Glob,Bash --add-dir <worktree>`, `--strict-mcp-config` with an inline
empty MCP configuration and `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`. It
ran in a scratch repository with one uncommitted change. The prompt asked for
`git diff --stat`, a file write, `touch` and a subagent in a worktree.
`git diff` ran; Write and the subagent tool were unavailable; `touch` was
denied without a permission request, with a `system` `permission_denied`
record; no file or worktree was created.

The initialization record was minimized, thinking-token, status and rate-limit
records were dropped, the session identity became a fixture constant and paths
became `/fixture/worktree`.
