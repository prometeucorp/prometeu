---
type: Decision
title: "ADR 0103 — Agents on a remote machine drive workspaces through its host"
description: "The host of the machine keeps its own registry of projects and workspaces. It launches Claude or Codex without a desktop, and it serves an agent socket and an MCP server that a token authorizes."
status: stable
---

# ADR 0103 — Agents on a remote machine drive workspaces through its host

Date: 2026-11-24

## Context

[ADR 0101](0101-remote-machines.md) keeps the board on the desktop. The host on the
machine only runs terminals, relays hooks and streams transcripts. The host
starts no process if a desktop does not send a request. ADR 0101 did not decide one
condition: "an external client on
the machine (for example a bot asking for workspaces) needs a separate decision."

An external MCP client must use Prometeu on the machine as a person does. It
must do these operations:

- Register and clone projects.
- Open workspaces.
- Read conversations, answer them, interrupt them and close them.

It must operate while the laptop of the person sleeps or is off. When the
desktop attaches, the items that the client made must show on the board of the
person. They must have the same transcript and status as items that the person
made on the desktop.

## Options considered

1. **Send the bot through the desktop.** The bot sends messages to the desktop
   (through the relay or the SSH attachment). The desktop makes the workspace
   as usual. There is one implementation. But no work occurs while the laptop
   is off.
2. **Queue requests on the host.** The host stores the requests of the bot.
   The desktop does them when it attaches. This option has the same limit. Also,
   the bot cannot read or answer items until the desktop attaches.
3. **Make the host the owner of the projects and workspaces.** The host
   keeps a registry. It makes worktrees and launches Claude. The bot and
   the desktop are its two clients. The desktop adopts the registry into its
   board.

For the transport of the client:

- **An MCP server through HTTP on the host.** This option must have a network
  listener. It must also have a separate deployment and a separate security
  policy for connections.
- **A local stdio MCP server.** All MCP clients can run the command and
  authenticate to the daemon through its Unix socket. Integrators decide how
  their infrastructure runs that command.

## Decision

Use option 3, with a local stdio MCP server and a preparation that does not
depend on the deployment. Prometeu owns the daemon, the MCP tools, the
authentication and the stable executable paths. Containers, volumes, network
access, user namespace maps and permission changes are the work of the
integrator. Settings shows the MCP configuration, not a procedure to install
infrastructure. Thus there is one interface for all clients, and Prometeu does
not add an HTTP service.

### Registry

The host owns `<root>/registry.json` (0600, atomic rename). The
[remote host contract](../contracts/remote-host.md#registry) specifies its version.

- `projects`: `{ path, name, origin?, added_at, added_by }`. `path` is an
  absolute repository on the machine.
- `workspaces`: `{ id, project, title, branch, base, worktree, created_at,
  created_by, closed, archived?, keep_branch, conversations, supervision? }`. A conversation is `{ id, title,
  created_at, permission, provider, thread_id?, model, effort }`. The `id` is
  the tab ID and the terminal ID. Claude also uses it as its native session ID.
  Codex uses the separate `thread_id`. Earlier records use Claude by default. This is also true for remote
  conversations that the desktop makes.
- `created_by` / `added_by` is `{ kind: "desktop" | "agent", name? }`. `name`
  is the label that the agent client sent.

Only the host writes the file. Each change goes through a host method. Thus two
clients do not change the file at the same time. The desktop board continues
to be the authority for all items that it owns: pins, order, unread state and
the conversation journal. The registry records only the items that are on the
machine.

### Launching without a desktop

`workspace_create` validates the project and runs `git worktree add -b <branch>
<root>/worktrees/<repo>/<branch> <base>` (the layout of the desktop). Then it
records the workspace and opens the terminal of the conversation. When the
request does not give a branch name, the name follows the
`prometeu/<word>-<word>-<n>` convention of the desktop. The host starts Claude
as the desktop does:

```text
claude --session-id <id> --dangerously-skip-permissions \
  --settings <root>/sessions/<id>/settings.json [<prompt>]
```

- The settings name the hook command of the host for each hook, as the desktop
  writes them. The bridge values that a desktop gives (socket, token, nonce)
  are placeholders. The relay of the desktop replaces them with its own values
  when it sends a hook.
- Claude asks if it can trust each new folder, also without permission prompts.
  A conversation that no person monitors waits for that answer without a time
  limit. Thus the host marks each worktree that it makes as trusted in
  `~/.claude.json`. Only the accurate folder counts. The team examined that a
  trusted parent folder does not count.
- Conversations that an agent makes run **without permission prompts**. No
  person can answer them while the desktop is away, and the person selected this
  mode for agent work. The registry records `permission: "auto"`. The desktop
  shows the adopted tab in that mode.
- The terminal runs in the slice of the workspace. The memory and CPU limits of
  the machine are in `machines.json` on the desktop. Thus they apply only to
  terminals that the desktop opens.

### Observed state

The host reads each hook that it relays, also when no desktop is attached. It
keeps the state of each conversation:

- `starting`: no hook came at this time.
- `working`: `UserPromptSubmit` or tool hooks.
- `idle`: `SessionStart` or `Stop`.
- `permission`: `PermissionRequest`.
- `ended`: no terminal runs.

A `Notification` does not change the state. The host keeps its message in a
separate `notification` field until the conversation operates again. The
queued delivery to the desktop does not change.

The messages come from the transcript of Claude
(`~/.claude/projects/<slug>/<id>.jsonl`). The host returns the user text, the
assistant text and the tool calls with truncated inputs and results. Clients
read it in pages with a byte cursor. When a client sends a message, the host
writes it to the terminal of the conversation as a bracketed paste and then
Enter. Claude puts input that comes while it operates in a queue, as it does
when a person types.

### Codex without a desktop

The host uses the native engine and TUI from [ADR 0105](0105-codex-on-remote-machines.md).
A separate `codex exec` process would have a different input protocol and could
not share its terminal with the desktop. Reading rollout files would also give
a second source of conversation content. The host uses the engine RPC for all
Codex content and state.

The host checks the installed version and sign-in before it makes a workspace.
It starts an engine, verifies the thread identity and settings, and records the
native thread ID before it opens the TUI. A failed resume does not start a new
thread. The TUI gets the first prompt as an argument. Later messages use the
same terminal input path as Claude. Escape interrupts the current turn.

A host observer reads the RPC while all desktops are disconnected. It keeps
completed messages in a private compact transcript for MCP readers. Native
thread, turn and item IDs remove duplicates after an observer reconnects. The
observer reads the final native history before it stops an exited TUI's engine.
A subsequent launch waits for that cleanup before it resumes the same thread.
Failed final reads reconnect while the engine operates. Desktop signal and
forget requests wait for this recovery. They do not stop the observed engine
or remove its socket before the host completes the read.
The transcript stays outside the temporary session folder, including after a
host restart.

The registry uses version 4 to protect archive state, provider and thread identity from older
hosts. Earlier records default to Claude. The host advertises `codex_agents`.
The desktop records Codex only when the host advertises this capability. The
desktop also requests Codex records explicitly. Older desktops receive only
Claude conversations and cannot resume a Codex thread as Claude.

The host uses the shared Codex protocol decoder for compact tool messages and
child activity. Thus `prometeu-host` can depend on `prometeu-protocols`, which
continues to depend only on the core. The desktop keeps its own presentation
journal and reads the same native engine when it attaches.

### Agent socket and token

- Agents do not use the desktop socket (`host-v1.sock`) at any time. Its
  `exec`, `file_write` and terminal methods give the same access as a shell.
  The host serves a second socket, `agents-v1.sock`. This socket has only the
  methods in this list.
- The host serves the agent socket only while there is a token. `prometeu-host
  agents setup` does not need flags or root. The default folder is
  `~/.prometeu-remote-agents`, adjacent to the root, with the mode 0700. It
  holds the socket (0666) and the client executable, but not the token. The
  setup does not change the modes of current folders. It does not open the home
  folder or the parent folders to other users at any time. `--socket` selects a
  different folder. A person who can replace the socket or the client can
  collect a token. Thus the setup and the host reject these folders:
  - A folder that the host user does not own.
  - A folder that other persons can write.
  - Parent folders that are not safe, if they are not sticky as `/tmp` is.
- Current paths, tokens and folder modes stay valid after an upgrade. The setup
  keeps legacy `opened` records. A disable continues to close those folders and
  keeps the cleanup records that had a failure, for a new try. Prometeu does not
  support `--open-home` now.
- The host keeps an executable copy of itself, `prometeu-mcp` (0755), adjacent
  to the socket. It replaces the copy atomically each time it binds the socket.
  This includes the first bind after an update. Thus the configuration of a bot
  names the same command after updates. The bot does not run a build that the
  host removed at any time. The setup records the copy in `agents.json`. The
  host writes, and `agents disable` deletes, only that recorded file. If a file
  with that name is in a folder, and the setup did not install it, the setup
  rejects the folder.
- The setup always records the path of the socket. An installation from before
  this default has no record. Thus its host keeps the
  `<root>/agents/agents-v1.sock` path from earlier versions, and its clients
  continue to operate. The next setup moves the socket and tells the person.
- The token is 32 random bytes in `<root>/agents/token` (0600). The first
  `prometeu-host agents setup` makes it, and `agents setup --rotate` replaces
  it. `agents disable` deletes the token and the client executable. Then the
  host closes the socket and removes it. The first frame of each
  connection is `{"hello":{"protocol":1,"token":"…","client":"<label>"}}`.
  The host compares the token in constant time on that frame and again before
  each request. If the token of a connection does not agree, the host closes the
  connection. Thus a rotation stops clients that are connected.
- A person who can get access to the socket can keep connections open. Thus the
  host has a maximum number of connections that wait for authentication, and it
  gives them a time limit. It also has a maximum number of authenticated
  connections.
- The path of the socket can change at run time. The host replaces a current
  entry only when it is a socket that does not answer. When the socket moves,
  the previous listener stops and the host removes the previous socket. Thus the folder
  that the socket left does not give access to the host.
- Methods: `projects_list`, `project_add { path }`, `project_clone { url, path? }`,
  `workspace_create { project, prompt, title?, branch?, base? }`, `workspace_list`,
  `workspace_get { id }`, `conversation_open { workspace, prompt }`,
  `conversation_read { id, cursor?, limit? }`, `conversation_send { id, text }`,
  `conversation_interrupt { id }` and `workspace_close { id, force? }`.
- `workspace_close` stops the terminals of the workspace. It removes its
  worktree and local branch, as the cleanup of the desktop does. Without
  `force`, it rejects a close that causes a loss of work:
  - Changes that are not committed.
  - Commits that only the branch holds (not on its base and not on a remote).

  It examines these two conditions before it stops the conversations, and again
  after they exit. While a workspace closes, no conversation can start in it.
- When a request is lost with its connection, its result is unknown. The MCP
  server sends again only requests that do not change data. For the other
  requests, it tells the agent to examine the state before it tries again. Thus
  a message is not typed two times.

`prometeu-host mcp --socket <path>` is a stdio MCP server that shows those
methods as tools. It reads the token from `PROMETEU_AGENT_TOKEN`. Thus it agrees
with the environment field of the MCP configuration of a bot platform.

`agents setup` prints a local MCP configuration. This configuration runs the
stable `<directory>/prometeu-mcp mcp --socket <socket>` command with absolute
machine paths and a token. The client examines the Unix identity of the
listener before it sends the token. The setup sets `PROMETEU_AGENT_HOST_UID` to
the real UID of the host user as the default. `--client-uid` continues to be an
explicit identity override for integrations. Prometeu does not infer a
namespace map or a container mount. Clients in a different execution
environment adapt paths, permissions and identities.

### The desktop

- **Synchronization.** The desktop synchronizes with `registry_list` on its own
  socket at each attachment. It does this again each time the host sends a
  `registry` event. The host sends this event after each agent request that can
  add or close an entry. Thus the board follows the work of the agent while the
  desktop stays attached.
- **Adoption.** The desktop adds missing registry projects as
  `remote:<machine>:<path>` projects, with an alias by origin as in ADR 0101.
  Registry workspaces that are not on the board become unread remote
  workspaces. They have the ID, the branch and the worktree from the registry,
  and they name the agent that made them. Their conversations become terminal
  tabs for the recorded provider. Codex tabs also keep their native thread ID. The current
  reattachment then follows the terminals that run, or it collects the
  terminals that completed. Sometimes the terminal of a conversation is not
  there, because the machine started again. Then the desktop binds the
  conversation to its transcript on the machine. When the person opens it, it
  resumes as a conversation that the desktop started.
- **New conversations.** When an agent opens a conversation in a workspace that
  is on the board, the conversation becomes a new tab there. Registry adoption
  does not restore an archived workspace. Shared archive state follows
  [ADR 0104](0104-several-computers-on-one-machine.md#shared-workspace-archives).
- **Desktop records.** The desktop uses `registry_put` to record these actions
  of the person:
  - The person makes a remote workspace or opens a conversation in one.
  - The person registers or clones a project on a machine.

  The host merges that record into its data. These rules apply:

  - The desktop can add conversations and change the name of the workspace. But
    it does not drop the conversations that an agent opened, and it does not
    open a closed workspace again.
  - The desktop records a conversation only after its tab is on the board. Thus
    a synchronization does not adopt a conversation that the desktop launches
    at that time.
  - Each synchronization also records the open remote projects, workspaces and
    conversations that are not in the registry. This includes the items that
    persons made before this decision. Thus agents see them while the laptop is
    off.
  - When the close of a cleanup did not get to the machine, the next
    synchronization sends it again.
- **Ownership.** An ID from the machine does not take a board entry of this
  computer. It also does not take an entry of a different machine or workspace.
- **Removals.** When the person cleans a remote workspace on the desktop,
  Prometeu marks it as closed in the registry. When the person removes a
  workspace from the board, its checkout stays on the machine. The host archives
  the workspace for other computers. When the person
  closes a tab, its conversation stays. Thus the registry continues to list
  them. The same rule applies to a project that the person removes from the
  board. The desktop records these removals in
  `<root>/remote/<machine>/dismissed.json`. It does not adopt them again until
  the person adds the project again.
- **Closed by an agent.** An agent can close a workspace. When the desktop
  gets this information, it marks the workspace as cleaned and archived
  immediately. Its tabs stop. Its transcript stays readable. When the
  worktree is not there but the host cannot delete the branch,
  `workspace_close` names the branch in `branch_kept`.
- **Enable agents.** In Settings, a connected machine shows **Enable agents**.
  The desktop runs `agents setup --json` from the build that it attaches with,
  through the current SSH attachment. It shows a local MCP configuration with a
  copy button. It does not show container mounts, UID instructions or changes to
  home permissions.
  - The token goes only through that SSH connection. The screen hides the
    token. Thus a screenshot (for example, in a feedback report) does not
    contain it. When the person copies the configuration, the copy includes the
    token. Prometeu keeps the token in memory until the person closes the panel.
    It does not store, log or send the token through the relay at any time.
  - The panel is for the SSH destination that it was made for. When the person
    edits the destination or forgets the machine, the panel closes. Prometeu
    drops a late result for an earlier destination.
  - Settings shows agents as enabled only after the socket answers. If not, it
    shows why the host cannot serve the socket. The menu of the machine then
    shows a new token (rotation) and **Disable agents**. The two actions ask for
    a confirmation first, because they stop connected bots.
  - Each attachment reads `agents status` to show if agents are enabled.
- **Host updates.** A host update also applies to agents. The desktop replaces
  a host from an earlier version only while that host runs no terminal. If the
  host is busy at the attachment, the desktop asks again when a terminal that
  it follows ends. It also asks each minute, because terminals that agents
  start end without a message to the desktop. It does not ask while one of its
  own requests is in progress. The host rejects a stop while a command, a file
  write or a terminal launch runs. Thus an update does not lose the result of a
  `git worktree add` or a new token.
- **Visibility.** Agents see all workspaces on the machine, also the workspaces
  that the person made from the desktop. Local workspaces on the laptop do not
  go to the host at any time.

### Scope

Clients can select `provider: "claude"` or `provider: "codex"` when they make
a workspace or open a conversation. An omitted provider selects Claude, also in
a Codex workspace. Unknown providers fail before Git or the registry changes.
Clients cannot select a model, plugins, MCP servers or skills through these
methods. The configuration of the selected CLI on the machine applies. Workspaces with more than one repository stay on the desktop
only.

## Consequences

- Work starts and runs while the laptop is off, and a person or an agent can
  follow it. The board gets the changes at the next attachment.
- **All persons who have the token can make the selected agent run all commands that the
  Unix user of the person can run on the machine. The agent runs them as that user
  and without permission prompts.** The token is as sensitive as the SSH key of
  the person for that machine. A token rotation stops all agent clients.
- The host now knows about projects and workspaces. ADR 0101 kept these items
  on the desktop. The host stores only the items that are on the machine. The
  desktop keeps the presentation state.
- Two components launch remote Claude processes. Their hook settings must stay
  the same. Thus the two components make them with
  `prometeu_host::claude_hook_settings`. The local Claude terminals of the
  desktop use the same function.

## Evidence

- [Codex MCP integration tests](../../src-tauri/crates/host/tests/codex_agents.rs)
  examine creation, input, reads, interruption, reconnection, resume and cleanup
  with a native-protocol fixture. Final-read tests disconnect RPC after the TUI
  exits and send concurrent desktop cleanup requests. They verify the last
  reply and supervision completion without a false failure. They do not use a
  live model.
- [Desktop registry tests](../../src-tauri/src/remote/registry.rs) examine mixed
  providers, native identity and compatibility with earlier hosts.
- [Registry tests](../../src-tauri/crates/host/src/registry.rs) examine earlier
  conversation records and the refusal of unknown providers and newer formats.


The team implemented this decision on the host and on the desktop, and ran it
in production with OpenBot (2026-11-24, Prometeu 0.46.0).
The tests in this list cover the activation of agents from Settings without
shell steps:

- A setup without flags or root.
- A client executable that the host keeps current.
- The configuration with copy buttons.
- Host updates that wait for an idle time.

Automated tests cover the setup that does not depend on the deployment. The
team did not record a new production run through Settings at this time.

- [Remote host protocol](../contracts/remote-host.md): registry, observed state,
  agent socket and MCP server.
- `src-tauri/crates/host/tests/agents.rs`: an agent registers a project, opens a
  workspace, and reads, answers and closes it. No desktop is attached, and a
  test program operates as Claude. The test also covers these items:
  - The token refusal, the folder trust and the launch arguments.
  - The state from hooks, the cursors and the refusal of a dirty worktree.
  - The registry as the desktop socket returns it.
  - The registry event goes to an attached desktop. A subsequent record of a
    workspace from the desktop keeps the conversations that an agent opened.
  - The MCP server lists tools and relays calls.

  `setup_needs_no_flags_and_the_host_keeps_the_client_current` covers
  the modes of the default folder and the local JSON configuration. It also
  covers a host start that replaces a stale client, and the disable.
  `the_generated_local_config_lists_tools_and_relays_calls`
  runs that configuration directly and calls the tools.
- `src-tauri/src/remote/registry.rs`: the registry workspace of an agent
  becomes a board entry in the terminal mode that names its creator. The
  records of the desktop contain the conversations that it launches, with their
  permission mode.
- Unit tests:
  - `agents.rs`:
    - The setup does not change private home permissions.
    - Legacy paths, tokens and modes stay after the setup.
    - Legacy opened folders close on disable.
    - An installation from before the move keeps its socket.
    - The setup rejects shared folders, files that it did not install and a
      socket path that is in use.
  - `remote/connection.rs` (`a_busy_stale_host_is_retired_once_it_becomes_idle`).
  - `remote/agents.rs`: the setup output becomes a hidden preview and a local
    configuration that is prepared for a paste.
- `src-tauri/crates/host/tests/host.rs` (`an_update_waits_for_a_running_command`)
  and `tests/agents.rs` (`a_refused_socket_path_is_taken_once_it_is_free`). The
  second test also reads the bind error through `agents status`.
- A manual run on the VPS (2026-11-24) with Claude Code 2.1.331 and a test
  root:
  1. Through the agent socket, the agent made a workspace in a registered
     project.
  2. Claude wrote the requested file and answered.
  3. The agent typed a subsequent message with more than one line into its
     terminal, and Claude answered.
  4. The host rejected the close of the workspace while it was dirty.
  5. Then the agent closed the workspace, and the host removed its worktree and
     branch.
- A production run on the VPS (2026-11-24, Prometeu 0.46.0):
  - OpenBot runs in rootless Docker with its own Unix user.
    `/srv/prometeu-agents` is mounted read-only at `/prometeu`. That folder
    holds the agent socket and a copy of `prometeu-host`. The
    `agents setup --client-uid 65534` command agreed with the user of the
    person as the container sees it.
  - Through the MCP server, OpenBot listed the projects, made a workspace,
    followed its conversation and closed it.
  - The desktop was attached during the run. It showed that the bot made the
    workspace, followed it live and archived it on close.
  - The team did not make a workspace again in production while the desktop
    was off. The host tests cover this condition.
