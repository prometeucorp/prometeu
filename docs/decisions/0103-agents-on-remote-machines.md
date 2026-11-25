---
type: Decision
title: "ADR 0103 — Agents on a remote machine drive workspaces through its host"
description: "The machine's host keeps its own registry of projects and workspaces, launches Claude without a desktop, and serves a token-authorized agent socket and MCP server."
status: stable
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

- The desktop synchronizes with `registry_list` on its own socket on every
  attachment, and again whenever the host sends a `registry` event. The host
  sends one after each agent request that may add or close an entry, so the board
  follows the agent's work while the desktop stays attached.
- It adds registry projects that are missing as `remote:<machine>:<path>`
  projects, aliased by origin as in ADR 0101. Registry workspaces that are
  missing from the board become unread remote workspaces with the registry's ID,
  branch and worktree, and they name the agent that created them. Their
  conversations become terminal-mode Claude tabs with the registry's IDs. The
  existing reattachment then follows running terminals or recovers finished ones.
  A conversation whose terminal is gone (the machine restarted) is bound to its
  transcript on the machine, so opening it resumes like a conversation the
  desktop started.
- A conversation an agent opens in a workspace already on the board becomes a
  new tab there. An archived workspace returns to the active list, because an
  agent is working in it again.
- When the person creates a remote workspace, opens a conversation in one, or
  registers or clones a project on a machine, the desktop records it with
  `registry_put`. The host merges that record into what it holds. The desktop
  can add conversations and rename the workspace, but it never drops the
  conversations an agent opened or reopens a closed workspace. A conversation
  is recorded once its tab is on the board, so a synchronization never adopts
  one the desktop is still launching. Each synchronization also records the open
  remote projects, workspaces and conversations that the registry lacks, such as
  those created before this decision, so agents see them while the laptop is
  off. When a cleanup's close did not reach the machine, the next
  synchronization sends it again.
- An ID from the machine never takes over a board entry that belongs to this
  computer, to another machine or to another workspace.
- Cleaning a remote workspace on the desktop marks it closed in the registry.
  Removing one from the board leaves its checkout on the machine, and closing a
  tab leaves its conversation, so the registry keeps listing them. The same goes
  for a project removed from the board. The desktop remembers these removals in
  `<root>/remote/<machine>/dismissed.json` and does not adopt them again, until
  the person adds the project again.
- A workspace an agent closed is marked cleaned and archived on the board as
  soon as the desktop hears of it, and its tabs stop. Its transcript stays
  readable. When the worktree is gone but the branch could not be deleted,
  `workspace_close` names the branch in `branch_kept`.
- Agents see every workspace on the machine, including those the person created
  from the desktop. Local workspaces on the laptop never reach the host.

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
- Two places launch remote Claude processes. Their hook settings must stay
  identical, so both build them with `prometeu_host::claude_hook_settings`, and the
  desktop's local Claude terminals use the same function.

## Evidence

Implemented on the host and the desktop, and run in production with OpenBot
(2026-11-24, Prometeu 0.46.0).

- [Remote host protocol](../contracts/remote-host.md): registry, observed state,
  agent socket and MCP server.
- `src-tauri/crates/host/tests/agents.rs`: an agent registers a project, opens a
  workspace and reads, answers and closes it, with no desktop attached and a
  stand-in Claude. The test also covers the token refusal, folder trust, launch
  arguments, hook-derived state, cursors, the dirty-worktree refusal, and the
  registry as the desktop socket returns it. The registry event reaches an
  attached desktop, and the desktop's later record of a workspace keeps the
  conversations an agent opened. The MCP server lists tools and relays calls.
- `src-tauri/src/remote/registry.rs`: an agent's registry workspace becomes a
  terminal-mode board entry that names its creator, and the desktop's records
  carry the conversations being launched with their permission mode.
- Manual run on the VPS (2026-11-24) with Claude Code 2.1.331 and a test root.
  Through the agent socket, a workspace was created in a registered project.
  Claude wrote the requested file and answered, a multi-line follow-up typed
  into its running terminal was answered, and the workspace was refused while
  dirty, then closed with its worktree and branch removed.
- Production run on the VPS (2026-11-24, Prometeu 0.46.0). OpenBot runs in
  rootless Docker under its own Unix user, with `/srv/prometeu-agents` mounted
  read-only at `/prometeu`. That directory holds the agent socket and a copy of
  `prometeu-host`, and `agents setup --client-uid 65534` matched how the
  container sees the person's user. Through the MCP server, OpenBot listed the
  projects, created a workspace and followed its conversation, and closed it.
  The desktop, attached during the run, showed the workspace as created by the
  bot, followed it live and archived it on close. Creating a workspace with the
  desktop off was not repeated in production; the host tests cover it.

### Deployment gaps found in the production run

Enabling a bot still takes manual steps on the machine. None changes the
decision, but each is a follow-up:

- The desktop replaces an older host only when it attaches and the old host
  runs no terminal. A host busy at that moment kept serving after the update
  until the desktop reconnected.
- The socket directory outside `<root>` (here `/srv/prometeu-agents`) needed
  `sudo` to create, and `agents setup` needed the path and `--client-uid` by
  hand.
- The container runs a copy of `prometeu-host`. A plain copy kept the owner-only
  mode of `<root>`, so the container could not execute it until it was made
  0755, and the copy does not follow host updates.
- The bot's container needs the directory mounted and the token in its MCP
  configuration; the token is only read from `PROMETEU_AGENT_TOKEN`.
