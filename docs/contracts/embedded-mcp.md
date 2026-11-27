---
type: Contract
title: "Embedded Prometeu MCP and delegation"
description: "Local orchestration that a person must enable, with agents that a client owns and an optional conversation context."
status: stable
---

# Embedded Prometeu MCP and delegation

The implementation is complete. Decision:
[ADR 0049](../decisions/0049-embedded-delegation-mcp.md).

## Catalog and transport

`prometeu` is a virtual built-in MCP entry. All installations show it in the
current hub and pickers. It does not change a default selection. A person
cannot edit, remove or share the entry as a cloud catalog item. The current
selection rules for global, reviewed project and workspace selections apply. A
selection change applies when the next process starts or resumes. It does not
apply to an idle process that continues to operate.

Only a selection adds `Prometeu --prometeu-mcp`, with the executable path of
the app that operates. This entry point operates without a webview or a
login-shell startup. The providers get it in different forms:

- Claude gets a private MCP configuration.
- Codex uses the current wrapper with a private environment file.
- Cursor receives the stdio command and the environment through ACP
  `mcpServers`.

The app does not store credentials in the hub or in command-line arguments.

The subprocess sends JSON-RPC through stdio, with a newline after each message.
It agrees on MCP `2025-06-18` with the client. It accepts initialization, ping,
tools/list and tools/call. Notifications do not receive responses. Protocol
errors use JSON-RPC errors. Operation failures use `isError` tool results.
Correct results include text JSON and structuredContent. See the official
[transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports),
[lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)
and [tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
contracts.

A private Unix socket connects that subprocess to the open desktop app. The
socket is in a 0700 folder with a random name under `/tmp`. Its path is shorter
than the socket path limit of macOS. The socket is 0600. Each request
authenticates a client identity. The conversation context is optional.

The internal configuration gives a temporary credential. That conversation is
its context and its identity. When a process starts, the app replaces the
credential. When the app removes the process, it revokes the credential. Each
call must have the live process and the current tab of the credential.

An external registration gives a permanent client identity with explicit
project paths and no conversation context. The two types of client use the same
tools and application functions. It is not necessary that an external client
has a conversation that operates in Prometeu.

The private `<root>/mcp-socket` discovery file points to the current socket.
External stdio hosts resolve it on each tool call. Thus a current host can
connect again after the desktop starts again. Internal configurations keep
their explicit `PROMETEU_MCP_SOCKET` and the life of their process. When the
app exits, it removes its socket and the discovery file. It does not revoke
external registrations. Requests have a failure while the app is closed. No daemon
starts.

The transport input has a limit of 1 MiB. The responses have a limit of 768
KiB. Socket reads and writes have time limits. The server does tool operations
one at a time. The preparation and the execution continue asynchronously. The
app does not add an HTTP listener or a relay endpoint.

## External client registration

Use the installed executable. For the development app, use the development
executable. The administration mode and the stdio mode operate without a
webview:

```sh
prometeu_bin=/Applications/Prometeu.app/Contents/MacOS/Prometeu
"$prometeu_bin" --prometeu-mcp-client projects
"$prometeu_bin" --prometeu-mcp-client register "my local agent" PROJECT_ID
"$prometeu_bin" --prometeu-mcp-client list
"$prometeu_bin" --prometeu-mcp-client revoke CLIENT_ID
```

`projects` reads the stored project catalog and does not change the board. The
registration accepts one or more current project IDs and pins their source
paths. It returns `client_id` and a `mcpServers.prometeu` stdio definition that
a host can use immediately. The definition contains the command, the arguments
and an environment with `PROMETEU_ROOT` and `PROMETEU_MCP_TOKEN`.

To configure the external host, do these steps:

1. Copy that definition into the MCP settings of the external host.
2. Use the format that the host accepts.
3. Keep the configuration private, because it contains a bearer credential.

The app does not edit the configuration of a provider automatically. Then the
client can call these tools:

- `list_projects` and `list_agents`.
- `delegate` with a `project_id` from a response.
- The current status, message, file, script and preview tools.

A registration with only projects does not give access to the to-do list of the
person. To give that access, add `--todos`:
`register --todos "my local agent" [PROJECT_ID...]`. With `--todos`, the project
IDs are optional. A client without a project and without `--todos` is not
permitted. Refer to the [to-do contract](todos.md#authorization).

Each registration is a new and different identity. `list` returns the IDs, the
names and the project paths. It does not return credentials. A revocation
removes the registration. The next request has a failure, also from a stdio host that
is connected at that time. The work that the app accepted before continues in
Prometeu. A new registration with the same name does not get the ownership of
the delegations of the previous client. The registration and the revocation are
local administration commands. The app does not offer them to agents as tools.

External credentials are in `<root>/mcp-clients/<uuid>.json`, version 1. Each
file has `name`, `token` and `client: {id, conversation, projects, todos}`.
`todos` is false when it is missing. The client ID is `client:<uuid>`. The context is null. The projects are the permitted source
paths. The app writes the files atomically with 0600 in a 0700 folder.

The authentication loads the credential file of the client again on each call.
If the file is missing or corrupt, or if its version is not known, the call
has a failure and access stays closed. Tokens do not go into the hub or the board. The
folders are additive, and older versions ignore them. A rollback disables the
external access. It does not change the current internal delegation owners or
transcripts. A subsequent upgrade can use the saved external registrations and
ownership again.

This is an authorization of the application. It is not a sandbox of the
operating system. Agent processes keep the permissions of the local user. They
can access files without MCP through their usual tools. Other code that operates
as that user can read the private session configuration. The contract limits
only the operations through this MCP.

## Identities and ownership

- Workspace: the repositories, the worktrees and the branch.
- Delegated agent: the owner of one task, with a stable `agent_id`.
- Conversation: the transcript, which `conversation_id` identifies. In this
  version, it has the same ID as its delegated agent.
- Execution: one coordinator message or one turn that the app sees, with its own
  `execution.id`. Usual input that the agent accepts during an execution keeps
  that ID until the next turn that the app sees is complete.

`Board.delegations` stores these items:

- The client ID of the owner, the agent, the workspace and the initial task.
- The key and the hash of the creation request.
- The start commits of the repositories.
- The executions.
- The last background tasks that the app saw and the pending requests.

On previous boards, the field is an empty list by default. Legacy owner strings stay
conversation client IDs. External owners use the `client:` namespace. The app
does not write a transcript again. The ownership stays after the process starts
again. The app does not find the ownership from the tab sequence, the active tab
or the workspace membership.

Legacy tabs that a person added to a delegated workspace do not give authority
for those tabs. The app rejects more conversations in the workspace. A person
can continue to access the current transcripts. A target that the client does
not own and a target that is not there return the same
`delegation_not_found`. The app examines the project scope for
each owned target. This includes all repositories in a delegation with more than
one repository. When a person deletes an internal coordinator tab, its
delegations become inaccessible through MCP. Their workspaces stay available to
the person.

## Sub-workspace navigation

The desktop makes one navigation level from the current delegation records
([ADR 0099](../decisions/0099-sub-workspaces.md)). The primary workspace contains
the owner conversation. Other delegations show as siblings under the same main
workspace. Their initial `owner` values and their authorization for the exact
target do not change. The board, IPC and the relay do not get a parent field.

Sub-workspaces use one sidebar row with the status, the provider and the title.
Their branch and model stay in the tooltip. Their conversation view has a link
to the primary workspace. Each delegated workspace accepts only its initial
conversation. `new_tab` and the agent `action_start` return
`err.session.delegated`. This also applies during the preparation and when the
owner is external or is not there. Prompt actions and usual input continue to
use the current conversation. Requests to configure workspace scripts also use
that conversation. A person can continue to see and use the previous extra tabs,
but they cannot become larger groups. File, Changes, browser and terminal tabs
stay available. They are not agent conversations.

External owners and coordinator conversations that are missing or archived make
standalone entries. Archived children stay in the archive. When a person
moves the coordinator out of the archive, the group comes back. Children from a different
project follow the project of the primary workspace. The search includes their
titles. If a person pins one member, the full family is pinned. The pin flags
of the other members do not change.

The search also shows the pinned families that agree with the search. It does
not change their saved closed state. Closed project folders include the attention and running
states of the children. This also applies to children from a different project.
Worktree cleanup and archive operations do not apply to the children
automatically. Remote shares keep their current presentation.

## Tools

Each agent target is an `agent_id` that `delegate` returned. No tool accepts an
owner identity, a workspace ID that the client selects or a repository path that
the client selects.

| Tool | Inputs | Behavior |
| --- | --- | --- |
| `list_projects` | optional `offset`, `limit` (1–100) | Lists the board projects in the scope of the authenticated client, with `project_id` and the name. |
| `list_agents` | none | Lists the worker providers (`claude`, `codex`, `antigravity`) with their availability and the exact model IDs and efforts. It queries each CLI live for its selected account. |
| `delegate` | `request_key`, `title`, `task`, `provider`, optional `project_id`, `model`, `effort` | Makes one agent and one isolated workspace. It returns immediately during the preparation. |
| `list_delegations` | optional `offset`, `limit` (1–100) | Lists only the delegations of this client in its scope. |
| `get_delegation` | `agent_id` | Returns the preparation or failure of the workspace, the conversation status, the latest execution, the background, the pending requests and the setup and run runtime. |
| `get_execution` | `agent_id`, `execution_id` | Reads one recorded execution. |
| `send_message` | `agent_id`, `request_key`, `text` | Sends to an idle agent or resumes its stopped process. Returns an execution. |
| `interrupt` | `agent_id` | Requests a stop of the primary turn. It does not promise to stop all background tasks of the provider. |
| `read_conversation` | `agent_id`, optional `limit` (1–200) | Returns a tail of canonical V1 events with a limit. It does not return the transcript of a different tab. |
| `list_files` | `agent_id`, optional relative `path`, `offset`, `limit` (1–500) | Lists the entries in the owned workspace. |
| `read_file` | `agent_id`, relative `path`, optional `start_line`, `limit` (1–500) | Reads text with line pages. The first line is 0. |
| `run_workspace_script` | `agent_id`, `kind` (`setup` or `run`), optional `name` | Starts a configured script, or uses the same live script again. `name` selects a configured Run entry. |
| `read_workspace_script_log` | `agent_id`, `kind` (`setup` or `run`), optional `limit_bytes` (1–65,536) | Reads a tail of the kept terminal output with a limit, with the process state and the exit code. |
| `open_workspace_preview` | `agent_id` | Requests that the desktop app opens the delegated conversation and the workspace preview. It does not start a service. |
| `list_todos` | none | Lists the standalone to-dos of the person. |
| `create_todo` | `title` | Adds a to-do with the status `todo`. |
| `set_todo_status` | `id`, `status` (`todo`, `doing` or `completed`) | Changes the status of a to-do. |

The to-do tools do not use an `agent_id` or a project. Conversation clients can
use them. External clients can use them only with the `--todos` grant. Other
clients get `todos_not_allowed`. The tools use the list of the location that
the person selected, also when that location is a remote machine. Refer to the
[to-do contract](todos.md).

Without `project_id`, the delegation uses the repositories of the attached
conversation and the committed HEAD of each repository. Without a conversation
context, `project_id` is mandatory. An explicit project uses the committed HEAD
of the source clone of the project. It must be in the scope of the client. A project
ID that is not there and a project ID that is not permitted return the same
`project_not_found`. A tool argument cannot supply project paths.

The app does not copy changes that are not committed. In this version, a
delegation cannot use a folder that is not a Git repository or a repository
without a commit. The backend makes the branch names. `provider` is mandatory and
accepts `claude`, `codex` or `antigravity`. Cursor can be a coordinator, but it
is not a worker provider.

If the provider is the provider of the attached conversation, a delegation from
the context uses the model and the effort of the conversation. The call can
supply a different model or effort. A different provider, or an explicit
project, starts from the CLI defaults of that provider. For a delegation from
the context, the app copies the permission policy of an action profile and keeps
it on resume. Usual conversations keep the current default permission behavior.

The current workspace creation, the worktree rollback and the setup scripts
apply. A preparation with a failure stays visible. Initial workers explicitly
select no MCP servers, plugins or standalone skills. Thus they do not get the
delegation function of the coordinator or the global and project tool defaults.
The person can change their selections as usual. The other native behavior of
the provider does not change.

A creation retry with the same owner and request key returns the initial
result. If the creation arguments changed, the retry returns
`request_key_conflict`. Message keys apply only to the target agent and compare
a hash of the text. A retry returns the initial execution and does not send the
message two times. Keys have 1 to 128 bytes. The task text and the message text
must not be empty or contain only spaces, and they have a maximum of 64 KiB.

The app writes the ownership to disk before the preparation. It writes the
execution reservation before it sends the message. This does not promise that
the provider receives the message only one time after a crash. A reserved
execution can stop without a delivery. It is also possible that a provider
accepted the message before the app stored it. Examine the pending input and
the history. Do not use a new key automatically.

These conditions reject new MCP messages:

- The conversation is busy.
- There is pending input or a pending question.
- The workspace is archived, cleaned or has a failure.
- There are known active background tasks.

There is no API to steer or schedule. The execution host owns an `InputGates` registry that the
composition gives to it. Sends from a person and from MCP share its input gate for
each conversation. Immediately before the process write, the host examines the idle
state again under the process lock. A process start with a failure can keep the current
pending message for the usual recovery. There are no tools for deletion, Git
publication, approval answers, automatic orchestration or remote control.

### Provider and model routing

Coordinators own native subagents that cannot change the provider. Thus the
tool and server descriptions tell when a Prometeu worker applies. They also tell
which model family each provider runs. `delegate` must have `provider`. A model
does not select the provider, and the provider of the coordinator does not
select it. Thus each coordinator, also Cursor, selects the provider explicitly.

A call without `project_id` can name the provider of the attached conversation
without `model` or `effort`. Then it keeps the current selection of the
conversation and does not query the catalog. All other calls, which include all
explicit projects, query the live catalog of that provider for the account
before they make an item. They examine the model and effort that the worker will
use, from the call or from the conversation:

- If the catalog query has a failure, the call returns `provider_unavailable`
  with the catalog error.
- If a model from the call is not an exact catalog ID, the call returns
  `model_not_available`. The error lists the IDs of that provider that contain
  the requested value or that the value contains. If there are none, it lists
  the first IDs. It points to `list_agents`. A model from the conversation that
  is not in the catalog stays.
- If the model of the worker does not list an effort from the call, the call
  returns `effort_not_available`. If there is no model from the catalog (the
  CLI default, or a model from the conversation that is not in the catalog), one
  of the models of the provider must list the effort.
- If the model of the worker does not list an effort from the conversation, the
  worker uses the CLI default, because `effort` cannot be empty.

The app compares Codex efforts as the launch uses them. The `ultracode`
spelling that older Codex selections saved is the same as the native `ultra` of
Codex.

When the app rejects a call, it does not store data. Thus a person can try the same
`request_key` again with correct arguments. Compatibility: before this change,
calls without `provider` used the provider of the conversation, or Claude for an
explicit project. At this time, the app rejects these calls as incorrect arguments before
it sends them. MCP hosts read the requirement from the tool schema.

The app rejects model strings that are not in the catalog, for example full
Claude model names that are not catalog aliases. The error gives the hint, and
the string does not go to the CLI of the worker. The current timeout of 20
seconds limits the catalog queries. `list_agents` queries the providers in
parallel. Thus its reply comes before the read limit of 60 seconds of the stdio
host.

Delegation summaries include the `provider`, `model` and `effort` of the
worker. The app resolves them as the launch does: first the override of the tab,
then the settings of the workspace. This also applies during the preparation.
An empty model or effort means the CLI default. Antigravity workers do not have
approval or question prompts. Like usual Antigravity conversations, they operate
with automatic execution, but not when the copied action profile is Ask.

## Workspace scripts and preview

The creation runs the current setup. This includes the configured file copies
and the setup of more than one repository. The explicit setup tool is for
retries. It does not accept a script name. Run accepts only the names that the
current repository settings declare. It uses the same selection of the primary
repository and the default as the desktop. The two tools do not accept shell
commands, paths or workspace IDs from the client. The exact delegated
conversation must continue to be there. Changes reject workspaces that are in
preparation, have a failure, or are archived or cleaned.

Scripts use the current dock PTYs, the worktree environment and the reserved
port. Starts from the desktop and from MCP share the process check and the
insertion. The app uses a live script again. If a person requests a different
Run name while one is alive, the request fails with `err.dock.running`. MCP Run
does not start while the setup operates.

After a script exits, a different call starts it again explicitly and replaces
its previous log. Script calls do not remove duplicate request keys durably. The
commands return after the process starts. They do not wait for the setup or for
a service that operates for a long time to exit. The current desktop controls
stop scripts. Archive and removal keep their current cleanup behavior.

`get_delegation.runtime` contains `port`, `url`, `run_names`, and the
`setup`/`run` objects with `state` (`not_started`, `running`, `exited`), `name`
and `exit_code`. The runtime is `null` if the workspace is not there. If the app
did not keep a PTY, the state is `not_started`. This also applies after the app
starts again or after a person closes that dock. It does not prove that a script
did not run before. An exit code of `null` means that the app did not see
an exit code. It does not mean success. The runtime data is local to the process
and adds no stored board fields. A live process and a reserved URL do not
prove that the HTTP server answers.

Log reads give the last 16 KiB by default, with a maximum of 64 KiB of kept
bytes. They return `text`, `truncated`, `seq`, `running`, `name` and
`exit_code`. The text keeps the ANSI escape sequences. The decoder changes
incorrect UTF-8 at byte boundaries. `seq` belongs to the current PTY and is not a
durable cursor. The current scrollback limit of 512 KiB continues to apply.
Missing logs return `script_not_started`. The core terminal buffer and the
native factory port keep these fields. See the
[terminal boundary](application-core.md#processes-terminals-and-commands).

To open the preview, the app sends a local `workspace-preview` event to the main
webview, with the owned workspace ID and conversation ID. The desktop examines
again that they are there and available. It goes through its current workspace
flow and opens the current embedded preview. It gets the URL from the workspace
port. The MCP cannot supply a URL or JavaScript. The tool returns `requested`
and `url`. This acknowledges the navigation request. It does not show that the
page loaded or that the server answers. The preview opens only on an explicit
request, because it changes the workspace that the person sees.

## Observed execution state

The execution states are `queued`, `running`, `completed` and `stopped`. A
completed execution has the outcome `ok`, `error` or `interrupted`. Its source
shows if it is coordinator input, usual conversation input or a background
continuation that the app sees. When a process stops or the app starts again, the app
does not make an incorrect success. A queued execution with a stored pending message
keeps its ID on recovery. Other executions that are not complete become stopped.
Requests stay for the person to answer in Prometeu.

In V1, accepted usual input does not identify a separate provider turn. Thus
input that comes during a turn does not make execution records that operate at
the same time. The active ID keeps its initial source and receives the next
last outcome that the app sees. The provider can start a response again without a new
accepted input event. Then the current rule for background continuation makes
a different execution. This does not tell which earlier message caused it. MCP
sends continue to reject busy conversations. They keep a different ID for each
accepted request key.

A background of `null` means that there is no current authoritative
observation. `[]` is an empty set that the app sees. A correct primary turn does not clear
the background tasks. An empty background update alone does not complete a
turn. The execution records its outcome when the primary turn stops. It becomes
`completed` only when all the tasks are complete. Thus an outcome on a `running`
execution means that the primary turn stopped while its children continued to
operate. Only the `completed` state shows the completion.

An interruption completes immediately. It discards the last observation of the
children, because it also stopped them. Thus `get_execution` and `send_message`
agree: while the app rejects a send because the conversation is busy, the
execution is not completed. See
[ADR 0056](../decisions/0056-background-tasks-hold-completion.md). Claude and
Codex supply the current normalized signals. These signals do not show each
native child process fully. Native provider subagents are different from other
Prometeu delegations.

The core `SessionOutput` calls the `ExecutionObservation` port. The desktop
adapter sends the call to the current execution projection and publication. The
execution observation updates only memory under the publication lock of the
conversation, in the canonical event sequence. The lock sequence is chats →
conversation buffer → board. The delegation does not get a conversation lock
while it holds the board. The publication and the storage do not hold the
conversation locks. Thus a late busy reaction cannot replace a fast completion.

The history uses the current transcript tail, which has a limit. The provider
edge normalizes it. A response has a maximum of 256 KiB of full events and reports
`truncated`. One large event at the end can cause an empty truncated tail. The
returned transport sequence is local to the process. It is not a durable cursor
for pages. Files use the current boundary for canonical paths and symlinks. The
limit for a file is 2 MiB, and the limit for a page is 128 KiB. A line that is
too long returns an error. Folder pages show the file system at the time of the
call. They are not a snapshot.

## Evidence

- `delegation.rs` examines the mandatory worker provider and the use of settings
  only from the provider of the conversation. It examines that the live catalog
  rejects models and efforts with hints. It also examines the default for an
  effort from the conversation and the summary settings.
- `embedded_mcp.rs` examines the routing schema, the mandatory provider and the
  descriptions.
- `delegation.rs` also examines the ownership, the compatibility of previous and new
  boards, the different execution and background state, and usual input during a
  turn. It examines the behavior after a new start, the input limits, the
  availability of workspaces, the preview payloads and the limits of script logs.
- `mcp_access.rs` examines the private credential storage, the scope, corruption
  and the revocation. `delegation.rs` also examines different clients, the project
  selection and legacy owners.
- `tests/mcp_client.rs` examines an external stdio subprocess, the CLI
  registration, the new discovery of the socket and the revocation against a
  local socket fixture.
- `embedded_mcp.rs` examines the handshake, the schema validation, tool failures,
  the processing of notifications, the framing limits and the position of the
  credentials.
- MCP configuration tests examine the configuration formats of the two
  providers.
- `e2e/tools.spec.ts` examines that the built-in entry is available and is not
  active by default.
- `crates/process/src/terminal/tests.rs` keeps the output and the exit status of a
  process that operates. `e2e/browser.spec.ts` examines the preview navigation event through
  the browser mock in Chromium. Some preview interactions also operate in
  WebKit.
- The current `crates/files/src/entries.rs` tests examine the limits of paths
  and symlinks.
