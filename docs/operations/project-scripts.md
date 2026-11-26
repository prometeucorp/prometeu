---
type: Operations
title: "Project scripts"
description: "The setup, run and archive scripts that a repository declares, and the rules that start them."
status: stable
---

# Project scripts

A project can declare commands for the lifecycle of a workspace in
`.prometeu/settings.toml`.

```toml
[scripts]
setup = "npm install"
run = "npm run dev -- --port $PROMETEU_PORT"
archive = "docker compose down"
```

These commands are only examples. Use the commands that are applicable to your
project. Prometeu operates the scripts as follows:

- The setup script runs when Prometeu makes a worktree. It runs before the
  first prompt goes to the agent. If the setup script has a failure, the prompt
  continues to go to the agent, and Prometeu shows a warning.
- The run script starts when a person starts it from the interface of the
  workspace.
- The archive script runs before Prometeu archives the workspace.

To declare more than one run command, use this format:

```toml
[scripts.run.web]
command = "npm run dev -- --port $PROMETEU_PORT"
default = true
```

The scripts get the environment variables `PROMETEU_WORKSPACE_PATH`, `PROMETEU_ROOT_PATH`,
`PROMETEU_WORKSPACE_NAME` and `PROMETEU_PORT`. Each workspace reserves ten
ports. The first port is `PROMETEU_PORT`. The environment variable `PORT` has the same
first value. When more than one workspace operates, use these ports. Do not use
the same port for all workspaces.

The scripts and the terminals of a workspace operate in the terminal of
Prometeu. They see `TERM_PROGRAM=Prometeu` and `TERM=xterm-256color`. They do
not get the identity of the terminal that started Prometeu, for example `TMUX`
or `ITERM_SESSION_ID`. They also do not get the session markers of an agent that
operates Prometeu, for example `CLAUDECODE`. The full list is in the file
`terminal.rs` of `prometeu-process`.

If a worktree does not have a settings file, it uses the file of the initial
checkout. Thus, you can keep private setup commands out of version control.
