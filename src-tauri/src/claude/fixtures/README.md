# Claude stream fixtures

`read-only.ndjson` was recorded on 2026-09-28 from Claude Code 2.1.283 with the
read-only task flags (`--restricted --permission-mode dontAsk --tools
Read,Grep,Glob,Bash,Skill,Task --add-dir <worktree>`, `--strict-mcp-config` with
an inline empty MCP configuration and
`CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`) in a scratch repository with
one uncommitted change. The prompt asked for `git diff --stat`, a file write and
`touch`. `git diff` ran; with Write absent the model tried to write through
Bash, and both commands were denied without a permission request; no file was
created. The CLI also emitted a `system` `permission_denied` record per denial.

The recording has complete messages only (no `--include-partial-messages`). The
initialization record was minimized, thinking-token and rate-limit records were
dropped, the session identity became a fixture constant and paths became
`/fixture/worktree`.
