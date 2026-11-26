---
type: Contract
title: "Agent accounts"
description: "Accounts, login, the global selection and isolated quota limits for each agent."
status: stable
---

# Agent accounts

The app obeys this contract. ADRs [0012](../decisions/0012-provider-accounts.md)
and [0013](../decisions/0013-remove-provider-accounts.md) record the decisions.

## Selection and conversation

The selection is local to this Mac. It is global for each provider. It is not
part of the workspace, the board or the relay. When a process starts, it
records the account ID and the revision. A turn that operates continues with
that account. The next message uses the current selection. If the app must
replace the process, the new process resumes the same transcript. A message that
a person sends during that change stays in `pending_prompt` until the end of
the turn. Answers to questions and permissions stay in the process that sent the
request.

Action tasks use the same account selection. They keep the model, the
instructions, the permissions and the tools of the profile that the app
recorded when it made the task.

When a person connects an account, the app does not make it active. After the
login, the person clicks the account in the panel. The person can click all of
the card, and its border shows the selection. Connected cards that are not
active show “Use this account”. The launcher shows the selected account. When
no account is active, the launcher opens these same cards and keeps the draft
until the person selects one. Before the app makes a workspace, it also
examines the global selection. It does this before it publishes a card or
prepares folders, also for callers that do not use the launcher. No stored
format changes.

A new connection increases the revision. Thus, processes that do not operate
resume before the next message, also when the ID does not change. The app refuses a new
connection during a turn that uses that account. A login failure does not change
the selection.

A person can remove all accounts, also the account that came from the terminal.
When a person removes the active account, the app removes the selection for that
provider. It does not make a different account active automatically. The turn
that operates continues to its end with its recorded profile. New messages
cannot start until the person selects an account. When a person removes an
account that is not active, the current selection does not change. During a
login that is not completed, a person cannot remove an account. The person must
complete or cancel the login first.

## Registry ownership

The rules for the registry, the selection and the login are in
`prometeu-core::accounts`. The desktop supplies the storage
(`prometeu-files::accounts`) and the native authentication (`account_login.rs`).
`accounts.rs` keeps the desktop facade, the executor, the quota effects and the
events. The provider adapters keep the credentials. Only identity metadata comes
back from the authentication. The obligations for failures, concurrency and
clean-up are in the [core contract](application-core.md#accounts-and-profiles).

## Registry and IPC

`<root>/accounts.json` stores `{ accounts, active }` with a private, atomic
write. Each account contains:

```ts
type Account = {
  id: string;
  provider: "claude" | "codex" | "antigravity" | "cursor";
  email: string | null;
  plan: string | null;
  connected: boolean;
  revision: number;
  authMethod?: string;
  keySuffix?: string;
};
type Accounts = {
  accounts: Account[];
  active: Partial<Record<ProviderId, string>>;
  login: { id: string; provider: "claude" | "codex" | "antigravity" | "cursor" } | null;
};
```

The `accounts` event and the return values of the commands use `Accounts`.
`login` is temporary, and the app does not write it to the disk.
The IDs `claude` and `codex` are the accounts of the CLIs that a person
installed before Prometeu. The backend makes UUIDs for all other IDs. When there is no
login, the interface identifies an account by its email or by a translated
label. The app ignores the previous `label` field when it reads the registry, and it
does not write that field. The registry accepts empty lists and providers
without a selection. Only when the file is missing does the app import the two
initial external profiles. When the app starts again, it does not add removed accounts again.

An older binary keeps the account entries of providers that it does not know as
JSON that it does not read. It also keeps their entries in `active`. When a known account
changes, these entries stay in the same `{ accounts, active }` shape. That
binary does not return them through IPC, query them, select them or use them
for a different purpose. The binary continues to fully validate the providers that it
knows. Forward compatibility does not make incorrect Claude or Codex entries
correct. This function starts with the versions that have it. If an earlier
binary rejected or replaced the entry of a newer provider, an upgrade cannot
make that data again.

| Command | Arguments | Return |
| --- | --- | --- |
| `accounts` | none | `Accounts` |
| `account_select` | `{ id }` | `Accounts` |
| `account_remove` | `{ id }` | `Accounts` |
| `account_login` | `{ provider, id: string \| null, method?: string }` | `Accounts` when the login is completed |
| `account_login_cancel` | `{ id }` | empty |
| `usage_refresh` | `{ provider?: ProviderId }` | empty. It schedules quota queries for selected profiles with expired data |

`id: null` makes a disconnected profile before the login starts. Thus, a person
can connect the profile again after a cancellation, an error or a new start of the
app. Only one login can operate at a time. The process has a deadline of ten
minutes, and a person can cancel it. The app stops the auxiliary process when
the operation is completed or has a failure. Tokens, stdout, stderr and the
authentication payloads of the provider do not go through IPC, the transcript
or the relay. There is no command to enter an API key.

If the automatic resume has a failure after the previous turn, `account-error`
shows the translatable error, and the message stays in `pending_prompt`.

If the registry is missing, the app keeps the current CLIs. If the registry is
incorrect, the app returns an error. It does not change to a different account
silently. A selection must point to an account in the registry with the correct
provider. The terminal account stays under external control. Prometeu does not
run a login or a logout on the initial profile. A removal removes the account from
the registry and from the next identity and quota queries. It does not
delete the profile folder, the credentials, the Keychain or the history.
Processes that operate at that time can continue to use them. A removal does
not revoke a login. It is possible that a query or an event that operates
completes, but it does not add the removed account to the registry again. When
there is no selection, `accounts::active` returns `err.account.noActive`. The
adapters do not silently use a removed terminal account.

## Profiles and credentials

`prometeu-profiles` has the profiles that follow. The desktop gives it to the
start of the provider and to the native authentication.

Managed profiles are in `<root>/accounts/<uuid>/`, with a `0700` folder. They
explicitly share the history, the skills and the plugins. They do not share the
inference credentials. The app applies the environment to the child
process. It does not change the environment of the app, and it does not replace
the global login of the CLI.

### Claude

The adapter uses `CLAUDE_CONFIG_DIR` and `claude auth login --claudeai`. The CLI
opens the browser and controls the tokens. `claude auth status --json` gives the
identity. On macOS, the CLI uses a different Keychain entry for each
folder. The `.credentials.json` fallback is also part of the profile.

`projects` points to the same folder as the initial Claude profile. Thus,
`--resume` and the previous UI paths continue to operate. Plugins and resources that
profiles can share also use links. The adapter writes the preferences, the MCP
servers and the project trust into the profile. It does not copy the identity
or the inference credentials. The adapter does not write the global
configuration again. The adapter removes the alternative credentials that the
environment and the derived preferences got from the parent. Before the adapter
opens a managed conversation, it examines the authentication in the project
folder. It refuses an alternative source that can replace the selected
subscription.

### Codex

The adapter uses a `CODEX_HOME` for each account and the official protocol of
the app-server: `account/login/start`, `account/login/completed`,
`account/read` and `account/rateLimits/read`. The app opens the official URL in
the browser. It does not interchange tokens or make new tokens. The CLI does these
operations. These
operations do not open threads and do not send prompts.

Managed profiles use `cli_auth_credentials_store = "file"`, with a private
`auth.json` in the profile. The adapter makes the configuration from the initial
configuration, with the OpenAI provider. It does not replace the global
configuration. Rollouts and the SQLite index stay shared. Thus, `thread/resume`
can operate under a different account. The model cache is part of the account.
The plugin layer is different for each workspace (or task session) **and
account**. The adapter does not change the authentication link of a process
that continues to operate. See [plugins](plugin-marketplace.md).

## Quotas and catalog

The local ID of the account is the key of the `usage` map. The previous `claude` and
`codex` keys stay correct for the initial profiles. Each value keeps
`{ windows, at }`. Process events update the account that the process recorded
at its start, also when the selection changed after that. The footer shows only
the limits of the selected account, or a dash when there is no selection. The
emails and the controls stay in the panel. The panel shows the accounts of the
provider, each with its quotas.

The native quota scheduler queries each profile in the registry independently.
When a provider does not answer, the scheduler keeps the last quota values. The
scheduler uses these intervals:

- A selected profile in the foreground: 60 seconds on AC power or 120 seconds on
  a battery.
- A profile that is not active in the foreground: 10 minutes on AC power or 20
  minutes on a battery.
- A window that a person cannot see or that does not have the focus: 15
  minutes for selected profiles and 30 minutes for other profiles.
- An unknown power source: the intervals for a battery.

After a failure, the scheduler waits 1, 2, 5, 15 and 30 minutes for each
account. Some actions of a person request a new try immediately, if the last try
occurred 30 seconds ago or before. These actions are: the person opens the
account panel, selects an account or moves the app back to the foreground. A selected quota that a person can see can refresh at a known reset
time, before its usual interval. These intervals tell when the scheduler sends
a probe. The
execution of the provider has its own timeout.

The scheduler starts a maximum of one probe of the current generation for each
account at a time. It is possible that a probe that the scheduler cancelled
completes, but the scheduler discards its result. A generation for each account rejects a late
probe after a removal or a new connection. Live events are more important than
the result of a probe that operates. When a login starts, the scheduler cancels
the older quota probe and stops new probes for that account.

Each end of a login, also a cancellation, a failure or a refusal during an
active turn, first removes the login that waits. Then it updates the scheduler
and starts it. The account becomes available for a probe immediately. This also
applies to accounts that are not active, when their revision did not change.
After a login failure, the quota of the account stays in the cache. When a
person removes an account, the app removes its quota from the cache. A process that operated before the
removal can complete its turn. But its late quota event cannot add again the
quota values of an account that a person removed or connected again.

The time values and failures of the scheduler are only in memory. `usage.json`
keeps the initial `{ windows, at }` format for each account. `at` changes only
when the quota values change. Claude continues to use the internal quota endpoint.
Codex uses the app-server first, and the previous endpoint is the fallback.
Antigravity uses its native `/usage` report. A missing quota is not a
percentage of 0. The dashboard quota queries of Cursor use the external CLI
login. Unified weekly quotas are more important than the previous monthly
summaries. See the
[Cursor contract](cursor.md#account-quotas). When the account changes, the app
loads the model catalog again. It discards late catalog responses from the
previous selection.

## Evidence and verification limits

- `crates/core/src/accounts/login/tests.rs`: these tests use injected effects.
  They examine the sequence of login and new connection, and the refusal during
  an active turn. They also examine cancellation, authentication and storage
  failures, limits when the account is busy, and the external connection.
- `crates/core/src/accounts/tests.rs`: registry failures, concurrent updates,
  revision checks, an empty selection and compatibility with providers that the
  binary does not know, through injected storage. `account_store.rs` examines
  private files and damaged storage.
- `crates/core/src/auxiliary.rs` and `crates/process/src/auxiliary/tests.rs`: the
  private authentication transport, the rules for cancellation and deadlines,
  and native clean-up. The provider adapters inject the Unix launcher. See the
  [core contract](application-core.md#processes-terminals-and-commands).
- `accounts.rs`: compatibility with previous registries, the removal of all
  accounts, the storage of the empty selection and cancellation that stops the
  process.
- `claude.rs`, `codex/account.rs`: native authentication and identity fixtures.
- `crates/profiles/src/claude.rs`, `codex.rs` and `tests.rs`: isolated
  credentials, shared history, explicit roots, an isolated environment and
  preparation failures, without Tauri. `account_profiles.rs` examines private
  desktop writes. `agent_launch.rs` and `account_login.rs` examine injected
  preparation failures.
- `crates/core/src/session/tests.rs`: account changes and the recovery of queued
  input through injected account and runtime ports. `chat/host.rs` supplies the
  account state of the app. `usage.rs` examines the isolated quotas.
- `usage_scheduler.rs`: the cancellation of probes at a login. It also
  examines a new start immediately after a cancellation or a failure. It does
  this for selected accounts and for accounts that are not active.
- `plugins.rs`: clean-up of the layers of each account. It does not follow links.
- `src/agents.test.ts`: a late catalog response.
- `e2e/accounts.spec.ts`: a stored selection for each provider, cancellation, a
  new connection and the removal of all accounts. It also examines a previous
  registry without nicknames and emails with special characters.
  Then it examines the draft that the launcher keeps until the person selects
  an account.
- The `empty_profiles_do_not_inherit_terminal_login` tests query the installed
  CLIs without a login or prompts. Run them independently.

The automated suite does not prove a full OAuth sequence with two actual
accounts. It
also does not prove that an authenticated conversation continues between them.
That verification is possible only with the logins of a person on the two
providers. Fixtures and mocks do not replace it.

## Shared account interface

Settings shows all providers, also the installations that are not available. It
uses the same cards and operations as the account switcher. The footer links to
that page. A provider with more than one method opens a dialog where the person
selects a method. A provider with only one browser method continues directly.
After a new connection, the new card gets the focus, but the app does not select
it. Secondary actions use the shared menu. The removal of an active account
shows a confirmation that tells about the empty selection. The removal of an
account that is not active stays one step. The account state, the errors and
the controls that a person can see use i18n and the primitives of the Design
System.

The cards show all of the identity, on more than one line when necessary. They put the plan and the
selection state together. They show each quota on its own meter, with a label for
the usage and a label for the reset time. When there is sufficient space, the popovers put Add
account and Manage accounts in the same footer row. After the external agy
profile is connected, the connection status replaces the disabled connection
button. When a person removes the profile, the connection action becomes
available again. The selector uses the same cards in a dialog body that scrolls,
with confirmation actions that do not move. The rules for selection, login and removal do
not change. This presentation does not make a format migration necessary.
`e2e/launcher-layout.spec.ts` examines narrow layouts, long identities and the
return of the focus in the selector, with some WebKit checks.

## Antigravity

`account_login` with the provider `antigravity` and the method `external`
explicitly connects the single external profile, with the ID `antigravity`. It
does not do OAuth, and it does not change the selection. `connected: false` and
identity fields with the value null tell that the app did not query the
identity. They do not tell that the CLI rejected the account. A person can
select external cards. The app does not make a removed external profile again at
the start. This provider does not accept managed UUID profiles or an API-key
method.

The child process uses the current environment and keyring of agy. The app does
not isolate or change that identity. If the user changes accounts in a different
application, the app cannot make sure that the identity stays the same. The official
interactive agy does the login and the consent in the browser. There is no
proven login API for Prometeu that uses only the browser. The adapter reads the
quotas from `agy -p /usage` one time each minute, without inference. The TSV
fields are the group, the window name, the remaining percentage and the RFC3339
reset time. The adapter changes the remaining value to the used value
(`100 - remaining`). It keeps different model groups in the current scope and
label fields of the window. If a query has a failure or incorrect data, the
adapter keeps the cache. Data that is not there stays not available. It does
not become 0. The panel shows the used and the remaining
percentages. The catalog reads `agy models`.

The previous Gemini accounts and active selections are entries that the app keeps
but does not read. A person cannot use them as accounts. The app does not touch their
folders, credentials and native histories. See
[ADR 0052](../decisions/0052-antigravity-runtime.md).

## Cursor

The provider `cursor` connects the single external CLI account, also with the
name `cursor`, with the method `external`. The connection and the selection stay
different operations. The provider does not accept a managed UUID profile, a
copy of the credentials or an API-key form. Its unknown identity fields do not
stop the selection of the external card. A removal stays after the app starts again, and
it does not run a logout. Cursor controls the account changes, not Prometeu.
Quota queries read the current CLI credential. They do not copy it or make a
new one.
Then they change the account limits of the dashboard into the shared usage
windows. See the [Cursor contract](cursor.md#account-quotas).
