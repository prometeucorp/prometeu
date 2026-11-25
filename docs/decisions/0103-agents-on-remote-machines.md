---
type: Decision
title: "ADR 0103 — Agents on a remote machine drive workspaces through its host"
description: "The machine's host keeps its own registry of projects and workspaces, launches Claude without a desktop, and serves a token-authorized agent socket and MCP server."
status: draft
---

# ADR 0103 — Agents on a remote machine drive workspaces through its host

Date: 2026-11-24

## Context

[ADR 0101](0101-remote-machines.md) keeps the board on the desktop. The host on the
machine only runs terminals, relays hooks and streams transcripts, and nothing
starts there unless a desktop asks. It left one case open: "an external client on
the machine (for example a bot asking for workspaces) needs a separate decision."

The case is real now. A bot platform running on the same VPS (OpenBot, in rootless
Docker under its own Unix user) should use Prometeu the way a person does:
register and clone projects, open workspaces, read conversations, answer them,
interrupt them and close them. It must work while the person's laptop is asleep or
off. Whatever it creates must show up on the person's board when the desktop
attaches, with the transcript and status it would have if the person had created
it there.

## Options considered

1. **Route the bot through the desktop.** The bot talks to the desktop (over the
   relay or the SSH attachment) and the desktop creates the workspace as usual.
   One implementation, but nothing happens while the laptop is off.
2. **Queue requests on the host.** The host stores the bot's requests and the
   desktop carries them out when it attaches. Same limitation, plus the bot
   cannot read or answer anything in the meantime.
3. **Make the host the owner of the machine's projects and workspaces.** The
   host keeps a registry, creates worktrees and launches Claude by itself. The
   bot and the desktop are both its clients, and the desktop adopts the
   registry into its board.

For the bot's transport:

- **An MCP server over HTTP on the host.** The bots' container runs in rootless
  Docker with host loopback disabled, so it would mean listening on a non-loopback
  address, with more surface and more configuration.
- **A stdio MCP server inside the bots' container.** It reaches a Unix socket that
  the deployment mounts into the container. The host build is static, so the same
  executable runs there.

## Decision

Option 3, with the stdio MCP server.

### Registry

The host owns `<root>/registry.json` (0600, atomic rename, `version: 1`):

- `projects`: `{ path, name, origin?, added_at, added_by }`. `path` is an
  absolute repository on the machine.
- `workspaces`: `{ id, project, title, branch, base, worktree, created_at,
  created_by, closed, conversations }`. A conversation is `{ id, title,
  created_at, permission }`, where `id` is the tab ID, the Claude session ID and
  the host's terminal ID at once, as it already is for desktop-created remote
  conversations.
- `created_by` / `added_by` is `{ kind: "desktop" | "agent", name? }`. `name`
  is the label the agent client presented.

Only the host writes the file. Every change is made through a host method, so
two clients never race on it. The desktop board stays authoritative for
everything it already owns: pinning, ordering, unread state and the
conversation journal. The registry records only what exists on the machine.

### Launching without a desktop

`workspace_create` validates the project, runs `git worktree add -b <branch>
<root>/worktrees/<repo>/<branch> <base>` (the desktop's layout), records the
workspace and opens the conversation's terminal. Branch names follow the
desktop's `prometeu/<word>-<word>-<n>` convention when none is given. The host
starts Claude as the desktop does:

```text
claude --session-id <id> --dangerously-skip-permissions \
  --settings <root>/sessions/<id>/settings.json [<prompt>]
```

- The settings name the host's hook command for every hook, exactly as the
  desktop writes them. The bridge variables that a desktop would supply (socket,
  token, nonce) are placeholders: the desktop's relay already replaces them with
  its own when it delivers a hook.
- Claude asks whether to trust every new folder, even without permission
  prompts, and an unwatched conversation would wait on that question forever.
  The host marks each worktree it creates as trusted in `~/.claude.json`. Only
  the exact folder counts; a trusted parent was verified not to.
- Conversations created by an agent run **without permission prompts**. Nobody
  could answer them while the desktop is away, and the person chose this mode for
  agent work. The registry records `permission: "auto"`, and the desktop shows the
  adopted tab in that mode.
- The terminal runs in the workspace's slice. The machine's memory and CPU
  limits live in the desktop's `machines.json`, so they apply only to terminals
  the desktop opens.

### Observed state

The host now reads every hook it relays, whether or not a desktop is attached,
and keeps each conversation's state: `starting` (no hook yet), `working`
(`UserPromptSubmit` or tool hooks), `idle` (`SessionStart` or `Stop`), `permission`
(`PermissionRequest`) and `ended` (no running terminal). A `Notification` does
not change the state; its message is kept in a separate `notification` field
until the conversation works again. Queued delivery to the desktop is unchanged.

Messages come from Claude's own transcript
(`~/.claude/projects/<slug>/<id>.jsonl`). The host returns user and assistant
text and tool calls with truncated inputs and results. Clients page through it
with a byte cursor. Sending a message writes it to the conversation's terminal
as a bracketed paste followed by Enter. Claude queues input that arrives while
it is working, as it does for a person typing.

### Agent socket and token

- Agents never use the desktop socket (`host-v1.sock`), whose `exec`,
  `file_write` and terminal methods amount to a shell. The host serves a second
  socket, `agents-v1.sock`, which offers only the methods below.
- Its directory is configurable, because exposing it to another Unix user or
  container is a deployment choice. The default is `<root>/agents/`, which only
  the owner can reach. The socket is 0666 inside its directory. What authorizes a
  client is the token.
- The token is 32 random bytes in `<root>/agents/token` (0600), created by the
  first `prometeu-host agents setup` and replaced by `agents setup --rotate`. Each
  connection's first frame is `{"hello":{"protocol":1,"token":"…","client":"<label>"}}`.
  The host compares the token in constant time on that frame and again before
  every request. A connection whose token no longer matches is closed, so
  rotation cuts off clients that are already connected.
- Whoever reaches the socket can hold connections open, so connections waiting
  to authenticate are capped and given a deadline, and authenticated
  connections are capped too.
- The socket's path can change at run time. The host replaces an existing entry
  only when it is a socket nobody answers. When it moves, the old listener stops
  and the old socket is removed, so the directory it left no longer reaches the
  host.
- Methods: `projects_list`, `project_add { path }`, `project_clone { url, path? }`,
  `workspace_create { project, prompt, title?, branch?, base? }`, `workspace_list`,
  `workspace_get { id }`, `conversation_open { workspace, prompt }`,
  `conversation_read { id, cursor?, limit? }`, `conversation_send { id, text }`,
  `conversation_interrupt { id }` and `workspace_close { id, force? }`.
- `workspace_close` ends the workspace's terminals and removes its worktree and
  local branch, as the desktop's cleanup does. Without `force`, it refuses what
  would lose work: uncommitted changes, or commits that only the branch holds
  (on neither its base nor any remote). It checks both before stopping the
  conversations and again after they exit, and while a workspace is closing, no
  conversation can start in it.
- A request lost with its connection has an unknown outcome. The MCP server
  sends again only requests that change nothing; for the others it tells the
  agent to check the state before retrying, so a message is never typed twice.

`prometeu-host mcp --socket <path>` is a stdio MCP server that exposes those
methods as tools. It reads the token from `PROMETEU_AGENT_TOKEN`, so it fits the
environment field of a bot platform's MCP configuration.

### The desktop

- On every attachment the desktop calls `registry_list` on its own socket. It
  adds registry projects that are missing as `remote:<machine>:<path>` projects,
  aliased by origin as in ADR 0101. Registry workspaces that are missing from the
  board become remote workspaces with the registry's ID, branch and worktree.
  Their conversations become terminal-mode Claude tabs with the registry's IDs.
  The existing reattachment then follows running terminals or adopts finished
  ones. Adopted workspaces show who created them.
- When the person creates a remote workspace, opens a conversation in one, or
  registers or clones a project on a machine, the desktop records it with
  `registry_put`, so agents see the person's work too. Removing or cleaning a
  workspace on the desktop marks it closed in the registry.
- A workspace an agent closed is marked cleaned and archived on the board at the
  next attachment. Its transcript stays readable.
- Agents see every workspace on the machine, including those the person created
  from the desktop. They are recorded when they are created, so agents see them
  while the laptop is off. Local workspaces on the laptop never reach the host.

### Scope

Claude only, as in ADR 0101. Agents cannot choose a model, plugins, MCP servers or
skills for the conversations they open; Claude's defaults on the machine apply.
Multi-repository workspaces stay desktop-only.

## Consequences

- Work starts, runs and can be followed with the laptop off. The board catches up
  at the next attachment.
- **Anyone holding the token can make Claude run, as the person's Unix user and
  without permission prompts, anything that user can run on the machine.** The
  token is as sensitive as the person's SSH key for that machine. Rotating it cuts
  off every agent client.
- The host now knows about projects and workspaces, which ADR 0101 kept on the
  desktop. It stores only what exists on the machine, and the desktop keeps the
  presentation state.
- Two places launch remote Claude processes. The settings and arguments must stay
  identical, so the hook configuration moves to a crate shared by both.

## Evidence

Implemented on the host. The desktop adoption and the OpenBot run are still
pending, so this decision stays a proposal.

- [Remote host protocol](../contracts/remote-host.md): registry, observed state,
  agent socket and MCP server.
- `src-tauri/crates/host/tests/agents.rs`: an agent registers a project, opens a
  workspace and reads, answers and closes it, with no desktop attached and a
  stand-in Claude. The test also covers the token refusal, folder trust, launch
  arguments, hook-derived state, cursors, the dirty-worktree refusal, and the
  registry as the desktop socket returns it. The MCP server lists tools and relays
  calls.
- Manual run on the VPS (2026-11-24) with Claude Code 2.1.331 and a test root.
  Through the agent socket, a workspace was created in a registered project.
  Claude wrote the requested file and answered, a multi-line follow-up typed
  into its running terminal was answered, and the workspace was refused while
  dirty, then closed with its worktree and branch removed.
