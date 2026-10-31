---
type: Operations
title: "Project scripts"
description: "Repository-declared setup, run and archive scripts and their execution rules."
status: stable
---

# Project scripts

Projects can define workspace lifecycle commands in `.prometeu/settings.toml`.

```toml
[scripts]
setup = "npm install"
run = "npm run dev -- --port $PROMETEU_PORT"
archive = "docker compose down"
```

These are examples; use commands appropriate to your project. Setup runs when
a worktree is created, before the first prompt reaches the agent. If setup
fails, the prompt still proceeds with a warning. Run starts from the workspace
UI, and archive runs before archiving.

For multiple run commands:

```toml
[scripts.run.web]
command = "npm run dev -- --port $PROMETEU_PORT"
default = true
```

Scripts receive `PROMETEU_WORKSPACE_PATH`, `PROMETEU_ROOT_PATH`,
`PROMETEU_WORKSPACE_NAME`, and `PROMETEU_PORT`. Each workspace reserves ten
ports starting at `PROMETEU_PORT`; `PORT` has the same starting value.
Use these ports instead of a fixed port when running multiple workspaces.

Scripts and workspace terminals run in Prometeu's own terminal: they see
`TERM_PROGRAM=Prometeu` and `TERM=xterm-256color`, and do not inherit the
identity of the terminal that started Prometeu (such as `TMUX` or
`ITERM_SESSION_ID`) or the session markers of an agent it runs under (such as
`CLAUDECODE`). The full list is in `prometeu-process`'s `terminal.rs`.

If a worktree has no settings file, it inherits the original checkout's file.
This lets you keep private setup commands outside version control.
