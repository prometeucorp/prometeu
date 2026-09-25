# Provider matrix

Status: current behavior observed in the code. "To confirm" means the feature
may exist, but does not yet have enough conformance evidence to become a
contractual capability.

Browser evidence follows the [E2E scope policy](../operations/development.md#e2e-scope).
Chromium runs the retained scenarios; only cases explicitly tagged `@webkit`
repeat in WebKit. References to both engines mean representative coverage, not
every combination of providers, states, languages and screen sizes.

## Local telemetry foundation

[ADR 0059](../decisions/0059-local-telemetry-foundation.md) adds the same local
capture/store/settings path for all providers. Main completion remains separate
from the conversation settlement rule in ADR 0056.

| Measurement | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Main execution and accepted message turns | Captured | Captured | Captured |
| Independent child execution and explicit human waits | When canonical signals expose them | When canonical signals expose them | Unavailable |
| Main-agent input/output | Turn result; deduplicated message inputs provide partial coverage | Delta of verified thread totals; resume/reset without a baseline stays partial/unknown | Unknown |
| Cache/reasoning | Cache read/write are input subsets; reasoning unknown | Cache is an input subset; reasoning is an output subset | Unknown |
| Observed models/calls | Main assistant model IDs, partial per-model input, distinct message calls | Unknown without verified actual-model/call evidence | Unknown |
| Per-turn cost | Same-session cumulative delta; restored/reset/overlapping-child spend excluded | Unknown | Unknown |

`telemetry/tests.rs`, the adapters' `telemetry_*` tests and
`src/telemetry.test.ts` cover normalization, incomplete coverage, local query and
erasure behavior, including late pre-erasure terminals, concurrent WAL readers,
streaming export and symlink rejection. `src/telemetry-settings.test.ts` covers
workspace option refresh while a filter stays selected. Existing command/publication tests cover accepted/failed sends
and response ordering. Streaming input without a correlatable native terminal
retains unknown turn attribution; capture health exposes that limit. These are
fixture-based guarantees, not live certification of every installed provider
version. Full semantics are in the [contract](../contracts/telemetry.md).

## Legend

- **Native:** the CLI already speaks the form consumed today.
- **Adapted:** Prometeu converts or implements the feature.
- **Unavailable:** the UI does not offer it because the provider does not
  support the flow.
- **To confirm:** a fixture or dedicated test is missing.

| Capability | Claude | Codex | Antigravity | Main evidence |
| --- | --- | --- | --- | --- |
| grouped settings and searchable resource library | shared UI; existing account and tool capabilities | same UI and existing capabilities | same navigation; external account notice and unsupported tool execution remain unchanged | `src/settings-navigation.test.ts`, `e2e/settings.spec.ts`, existing accounts, Cloud, MCP and notifications scenarios |
| create, rename, trash and restore from the side file tree | independent of the CLI | independent of the CLI | shared application behavior | `src/tree-menu.test.ts` for the menus; `src/tree-moves.test.ts` for the tab strip, drafts and expanded folders following a moved entry, while their wiring in `workspace.ts` and `viewer.ts` was checked by hand in the browser mock and is not covered in E2E; `session/files.rs` tests for names, Git metadata, conflicts, case-only renames on a case-sensitive disk and symlinks; `restore_deleted` tests in `session/git_tests.rs` for staged content and deletions; the system trash and case-insensitive disks are not exercised in tests |
| Git marks in the side file tree | independent of the CLI | independent of the CLI | shared application behavior | `src/tree-git.test.ts`, `tree_marks` tests in `session/git_tests.rs`; not covered in E2E |
| copy a complete Markdown code block, including diff fences | shared presentation | shared presentation | shared presentation | `src/markdown.test.ts`, `e2e/markdown.spec.ts`; clipboard success and failure over the browser mock |
| public bug reporting and private feedback with an account, image and capture | independent of the CLI | independent of the CLI | shared application behavior | `e2e/feedback.spec.ts`, the Cloud's `FeedbackTest` tests; opening the system browser, native capture and real GitHub require a manual smoke test |
| organizations, invitations and institutional sharing | the same relay V4; local execution | the same relay V4; local execution | shared application behavior | `team-organizations.test.ts`, `worker.integration.test.ts`, `e2e/organizations.spec.ts`, Rails integration/browser |
| automatic E2EE with TOFU in collaboration | the same HPKE Auth channel; no forward secrecy | the same HPKE Auth channel; no forward secrecy | shared application behavior | `team-crypto.test.ts`, `team-security.test.ts`, `team-channel.test.ts`, `worker.integration.test.ts`, E2E comments in Chromium/WebKit |
| optional Prometeu account in the sidebar | independent of the CLI | independent of the CLI | shared application behavior | `cloud.rs`, `e2e/cloud.spec.ts`; no transcript is sent |
| Git project catalog, batch clone and local registration | independent of the CLI; no setup | independent of the CLI; no setup | shared application behavior | `catalog/projects.rs`, `e2e/projects.spec.ts`, Cloud `projects_catalog_test.rb` |
| catalog of plugins, MCP and Actions in the account | independent of the CLI | independent of the CLI | shared application behavior | `catalog.rs`, `catalog_test.rb`; secrets and installation stay per Mac |
| the organization's plugins, MCPs and skills | explicit local installation in the hub | the same UI and installation; existing adapters | Unavailable | `catalog.rs`, `organizations_test.rb`; optional installation form has no dedicated E2E; it requires no personal copy and does not activate automatically |
| Actions form with shared components | the same UI | the same UI; options come from the catalog | shared application behavior | `e2e/actions.spec.ts` in Chromium; shared form focus/validation in `e2e/design-system.spec.ts` on both engines |
| the company's executable DS components | independent of the provider | independent of the provider | shared application behavior | `e2e/design-system.spec.ts`, menu, submenu, password, focus, validation and error on both engines |
| Code review included and editable | the initial profile; the model/provider can be changed | can be chosen in the profile | Adapted; see verification boundary | `actions.rs`, `actions.test.ts` for defaults/removal; `e2e/actions.spec.ts` for profile editing |
| prompt commands and tasks | adapted by the app | adapted by the app | Adapted; see verification boundary | `actions.test.ts`, `actions.rs`; `e2e/actions.spec.ts` retains prompt editing, not the task-monitoring form |
| per-task profile | instructions and permissions through flags | instructions and permissions through JSON-RPC | Adapted; see verification boundary | `session.rs` and `codex.rs` tests |
| PR tracking | local polling by the app | local polling by the app | Adapted; see verification boundary | `actions.rs`, `github.rs`; no real GitHub integration in tests |
| detecting the installation | adapted | adapted | version checked (minimum 1.2.7) | `agents.rs` |
| accounts and global selection in the footer | adapted through `CLAUDE_CONFIG_DIR` | adapted through `CODEX_HOME` | single external agy account, explicit attachment | `accounts.rs`, `e2e/accounts.spec.ts` |
| grouped workspace creation and readable account controls | shared fields; plan mode available | same layout with model search and native effort | same layout; unsupported plan, effort and tool controls stay absent; attached agy profile shows attachment status | `e2e/launcher-layout.spec.ts`, `e2e/accounts.spec.ts`, `e2e/model-picker.spec.ts`; English, narrow windows, Chromium/WebKit |
| removing every account and an empty selection | supported; preserves the CLI's login | supported; preserves the CLI's login | supported; preserves external login | `accounts.rs`, `e2e/accounts.spec.ts`; directories and credentials stay local |
| login through the app | the CLI's `auth login` and the browser | the app-server's `account/login/start` and the browser | Unavailable; official interactive agy login | identity fixtures in `claude.rs` and `codex/account.rs`; OAuth with two real accounts still requires manual validation |
| switching accounts between turns | resuming the shared transcript | resuming the shared rollout/index | Unavailable; externally managed identity | `chat.rs`, profile tests; an authenticated continuation between two real accounts is not yet proven by the suite |
| quotas per account | stream and internal endpoint | app-server and internal fallback | Native /usage; used and remaining by model group | `usage.rs`, `statusbar.test.ts`, `e2e/accounts.spec.ts` |
| live model catalog | native `list_models` control request | native paginated app-server `model/list`, including additional models | native `agy models` | `agents/catalog.rs`, `agents.test.ts`; per-agent errors and stale state |
| starting a session | native | adapted to JSON-RPC | native NDJSON; real text smoke passed | `chat.rs`, `codex.rs` |
| resuming a session | Claude's id/transcript | the app-server's thread | explicit native conversation ID | `session.rs`, Rust tests |
| model search and favorites | same picker, native labels | same picker; additional models opt-in | same picker, separate provider identity even for overlapping IDs | `e2e/model-picker.spec.ts`, `e2e/search-picker.spec.ts`; Chromium/WebKit |
| workspace tools with tab model/effort overrides | same selections for new and resumed tabs | same selections for new and resumed tabs | shared application behavior | `session.rs::new_and_resumed_tabs_preserve_workspace_tools_with_model_overrides` |
| tools in mixed-provider tabs and actions | CLI base follows the effective Claude provider and configured home | hub-only universe even in a Claude workspace | Adapted; see verification boundary | `session.rs::tool_resolution_uses_the_tab_provider_and_configured_claude_home`, `e2e/tools.spec.ts` |
| tool reset and project trust | null restores inheritance; decisions bind to the displayed hash, including empty declarations | same contract | Adapted; see verification boundary | `session.rs::tool_axis_ipc_preserves_absent_null_and_replacement`, `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes`, `e2e/tools.spec.ts` |
| ordered prompt, transcript, and live delivery | shared conversation mutex | shared conversation mutex | Adapted; see verification boundary | concurrent delivery, fast-response, and failed-write regressions in `chat.rs` |
| choosing a model | native through a flag | adapted in `thread/start`/`thread/resume` | native --model slug | `session.rs`, `codex.rs` |
| effort levels | only advertised native levels plus provider default | only advertised native levels; legacy ultracode maps to ultra | no effort control without advertised levels | `model-choice.test.ts`, `e2e/model-picker.spec.ts` |
| conversation footer adapted to the width | model and activity separated from the tools; remote control highlighted when active | the same UI | Adapted; see verification boundary | `e2e/composer.spec.ts`, Chromium/WebKit, English and narrow frames |
| dictation in the message box | independent of the CLI; WebKit recognition | independent of the CLI; WebKit recognition | shared application behavior | `voice.test.ts`; a real microphone needs a manual smoke test in the bundled app |
| initial plan mode | native through permission mode | unavailable | Unavailable | `session.rs`, `launcher.ts` |
| streaming text | adapted to V1 | adapted to V1 | Adapted; see verification boundary | `conversation.test.ts`, `timeline.test.ts`, `codex.rs` tests |
| thinking | adapted to V1 | adapted to V1 | Adapted; see verification boundary | `timeline.test.ts`, `codex.rs` tests |
| tool call and result | adapted to V1 | adapted to V1 | Adapted; see verification boundary | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| questions to the user | native | adapted from a JSON-RPC request | Unavailable | `chat.ts`, `codex.rs` tests |
| approval requests | native | adapted | Unavailable | `chat.rs`, `codex.rs` |
| interruption | control request | `turn/interrupt` | Adapted; see verification boundary | `codex.rs`, Rust tests |
| compaction | the CLI's command | `thread/compact/start` | Unavailable | `codex.rs` tests |
| context report | stream/transcript | synthesized from token usage | Unavailable | `context.test.ts`, `codex.rs` tests |
| MCP selection per workspace | the CLI's strict config | table and environment assembled by the app | Unavailable | `mcp.rs`, `codex.rs`; a preparation error prevents the spawn |
| MCP connection checks and local OAuth, including Cloud definitions | shared hub actions; token supplied at process launch | same hub actions and local token | MCP execution unavailable | `e2e/mcp.spec.ts` simulates checks, login retry, logout and independence from Cloud revisions; real browser consent requires manual validation |
| plugin selection per workspace | session flags | marketplace + config isolated per workspace | Unavailable | `plugins.rs`, the CLI smoke test, `codex.rs`, `launcher.ts`, E2E |
| local and account skills | a package with SKILL.md through the plugin selection | the same package with a native manifest | Unavailable | `skills.rs`, `catalog.rs`, `e2e/cloud.spec.ts`; installation does not activate automatically |
| start a conversation from a skill with the declared artifact path | package added to the first conversation's resolved `--plugin-dir` set; opening line in the first message | package added to the derived marketplace of that conversation; same opening line | Unavailable; no hub tool selection | `kickoff.rs` tests, `session.rs::first_message_opens_with_the_kickoff_line`, `session.rs::artifact_path_follows_the_primary_repository`, `scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented`, `src/kickoff.test.ts`; plugin-shipped skills must come from a local folder; the method is not re-announced to later tabs; a resume whose skill was removed continues without it and warns |
| layered selection (global, project, workspace) per axis | resolved at spawn before the adapter | resolved at spawn before the adapter | Adapted; see verification boundary | `selection.rs` resolve tests, `session.rs::provenance_classifies_each_hub_item`, `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes` and `session.rs::tool_axis_payload_is_validated_before_persistence`; the project `[tools]` layer is gated on trust-on-first-use of its hash, an undecided item stays `pending` and a rejected one stays `rejected` (both resolved yet not injected), and the setters refuse a malformed payload or an id on the wrong axis |
| CLI-inherited MCP base (ADR 0046) | discovered from `~/.claude.json` and the working directory's `.mcp.json` plus its ancestors, nearest first; visible in the picker with the `cli` provenance and removable as a workspace delta; a declared axis materializes the whole effective set through the strict config | no discovered base; the CLI keeps loading its own configuration outside the picker | Unavailable | `selection.rs::cli_base_participates_in_the_chain`, `mcp.rs` inherited/universe tests and `mcp.rs::missing_selected_servers_prevent_materialization`, `session.rs::provenance_classifies_inherited_cli_configuration`, `src/mcp.test.ts`, mixed-provider picker and inheritance reset in `e2e/tools.spec.ts` |
| hooks of a chosen plugin | active from `SessionStart` | `enabled = true` + trust limited to the `pluginId` and hash before the thread | Unavailable | `codex.rs` tests; a failure prevents the thread |
| attachments in a message, capture thumbnails and pasting | adapted through a local path; promise and pasteboard materialized by macOS, pasted image by GTK on Linux | adapted through a local path; promise and pasteboard materialized by macOS, pasted image by GTK on Linux | Adapted; see verification boundary | `file_drop.rs`, `chat.ts`, `paste.ts`, `tree-menu.ts`, `changes-menu.ts` and the shared `file-menu.ts`; `e2e/file-drop.spec.ts`, dropped-file scenarios and the file tree menu in `e2e/critical-flows.spec.ts` cover the UI over the mock; `paste.test.ts` covers the paste detour, including the empty WebKitGTK clipboard; `paths.test.ts` covers reading sent attachments back as numbered image tags, shared by both agents; `tree-menu.test.ts` covers the menu an agent without attachments receives, `changes-menu.test.ts` the same groups on a changed file |
| the browser's visual context | a tag in the draft and the history; complete HTML, CSS, URL and PNG mention on send | the same interface and textual contract | Adapted; see verification boundary | `browser-context.test.ts`, `e2e/browser-inspector.spec.ts`, `e2e/browser.spec.ts`, `browser.rs` tests; WKWebView capture and the AppKit gesture still require native verification |
| unknown external event | ignored by the adapter | ignored by the adapter | Adapted; see verification boundary | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| the CLI's subagents | a sidechain off-screen; tasks in `background.changed` | `collabAgentToolCall.agentsStates`, `subAgentActivity` and known child events update the tasks; the children's content stays isolated | no child session signal; the turn settles on its terminal | `claude.rs`; isolation, spawn, activity and partial-state tests in `codex.rs` |
| a turn that ends while its subagents run | tab, Stop button, queued input, delegation execution and notice all wait for the drain ([ADR 0056](../decisions/0056-background-tasks-hold-completion.md)) | the same rule over the same normalized signal | settles on the terminal, since no task is ever reported | `chat.rs::work_tests`; each adapter's real output drives the same rule in `claude.rs` and `codex.rs`; held-completion and interruption tests in `delegation.rs`; `src/timeline.test.ts`, `src/alert.test.ts`; Stop control in `e2e/composer.spec.ts` |
| local notifications and Dock attention indicators | completion, error and request events; silent until opt-in | same rule over canonical live events | completion/error over canonical events; interactive requests unavailable | `src/alert.test.ts`, `src/notifications.test.ts`, `e2e/notifications.spec.ts`; native verification limits in [notifications](../contracts/notifications.md) |
| live sharing | V1 after normalization | V1 after normalization | shared application behavior | `team*.test.ts`, E2E over the mock |
| remote control from the owner's devices | the same relay v4; execution stays local | the same relay v4; execution stays local | shared application behavior | `team-channel.test.ts`, `team-organizations.test.ts`, `e2e/organizations.spec.ts` |
| comments in a shared session | adapted after V1 | adapted after V1 | shared application behavior | `notes.test.ts`, `team.test.ts`, `relay/src/logic.test.ts`, E2E over the mock |
| optional missing-context review before creating a workspace (bring-your-own TypeSafe key) | independent of the CLI; no protocol change | independent of the CLI; no protocol change | shared application behavior | `src/context-review.test.ts` (rules, EN/PT examples, stale results), `evaluation.rs` and `typesafe.rs` tests (port, credential lifecycle, adapter failures with synthetic responses); System One wire checked against live API, see [context evaluation](../contracts/context-evaluation.md) |
| desk with several conversations at once | adapted (the same conversation screen) | adapted (the same conversation screen) | shared application behavior | `desk.test.ts`, E2E over the mock |
| Git: unified/side-by-side review, stage, discard, commit, remotes, branches, conflicts and the changed-file menu | adapted by the app; independent of the CLI | adapted by the app; independent of the CLI | shared application behavior | `session/git_tests.rs`, `diff.test.ts`, `changes-menu.test.ts`, `e2e/git.spec.ts` in Chromium/WebKit and a large review in `e2e/critical-flows.spec.ts`; the `git.md` contract |
| Markdown file reader and source editor | shared file viewer; independent of the CLI | shared file viewer; independent of the CLI | shared application behavior | `e2e/critical-flows.spec.ts`; source edits and drafts retain existing save behavior |
| find in the open file and quick open by name | shared file viewer and `find_paths`; independent of the CLI | shared file viewer and `find_paths`; independent of the CLI | shared application behavior | `find.test.ts` for matching, smart case, wraparound and marker markup; `session/find.rs` for path ranking; the viewer find bar and Command-P palette have no dedicated E2E gate |
| scoped worktree cleanup after archiving or finishing | independent of the CLI | independent of the CLI | shared application behavior | `e2e/audit-regressions.spec.ts`, `session.rs` cleanup tests; blocked worktrees require explicit force selection |
| agents per workspace in the sidebar | each tab's brand and status | each tab's brand and status | Adapted; see verification boundary | sidebar flows in `e2e/critical-flows.spec.ts`; a remote one uses the owner's avatar, without inferring the provider |
## Rule for a new feature

Before enabling a feature for a provider:

1. declare its common semantics in the contract;
2. add or adjust the capability in the descriptor;
3. capture a real CLI fixture without secrets or personal data;
4. prove the translation into common events;
5. cover common semantics in the reducer; add browser coverage only for a core
   UI journey or a concrete exception under the E2E scope policy;
6. update this matrix with the path to the evidence.

A missing test must not become `true` by similarity between providers.

## Known limitations

Desktop distribution is independent of the provider. macOS Apple Silicon and
Linux x86_64 AppImage share one release and signed in-app updates for Claude,
Codex and Antigravity. Other Linux installation types keep external updates.
Evidence: `scripts/test_release.py`, `src/update-init.test.ts` and
`src/update.test.ts`; native installation and replacement remain manual checks
under the [release contract](../contracts/releases.md).

- a plugin source in a local `.zip` or a URL works in Claude and is refused with
  a visible error in Codex; a local folder is the portable format;
- skills, commands, MCP and hooks have a portable adaptation; `agents/*.md`
  stays exclusive to Claude because it is not part of the current Codex
  manifest;
- plugins enabled outside Prometeu stay subject to each CLI's global registry
  and are not part of the workspace's selection;
- the CLI-inherited MCP base (ADR 0046) covers Claude only; servers configured
  for Codex in `~/.codex/config.toml` are not discovered and stay invisible to
  the picker, a recorded follow-up;
- attachments have UI tests over the mock and native validation of the saved
  destination; the real thumbnail gesture was confirmed in Prometeu Dev on
  2026-09-06. Actual reading by the CLI still requires manual verification.
  Promises that fail or exceed 30 seconds produce a visible error.

## Desired conformance suite

| Scenario | External fixture | Adapter | Adapted; see verification boundary | E2E |
| --- | --- | --- | --- | --- |
| simple message | per provider | required | Adapted; see verification boundary | smoke |
| streaming + final message | per provider | required | Adapted; see verification boundary | critical |
| successful tool | per provider | required | Adapted; see verification boundary | critical |
| tool with an error | per provider | required | Adapted; see verification boundary | critical |
| question and answer | per provider | required | Unavailable | critical |
| interruption | per provider | required | Adapted; see verification boundary | smoke |
| resume | per provider | required | Adapted; see verification boundary | critical |
| compaction | per capable provider | required | Unavailable | smoke |
| background task | per capable provider | required | Adapted; see verification boundary | smoke |
| unknown event | synthetic | required | Adapted; see verification boundary | not needed |

E2E does not replace the contract: it covers a few expensive paths. Fixtures and
reducers give fast diagnosis for every combination.

## Built-in delegation MCP

Both Claude and Codex can opt into the bundled `prometeu` MCP and create either
provider in an isolated workspace. Ownership, execution IDs, busy-send
rejection, file access and canonical history share the backend implementation.
Claude receives a private strict MCP file; Codex receives the stdio command
and private environment wrapper. Workers start with empty MCP/plugin/skill
selections. Background reporting is limited to each adapter’s existing
`background.changed` signals; unknown is distinct from an observed empty set. An
execution holds `running` with its outcome recorded until those tasks drain, so
it never reports completion while sends are still rejected as busy.

Both providers also use the same configured setup/Run controls, bounded dock
logs, process/exit status and explicit desktop preview opening. These tools
accept only an owned `agent_id`; there is no provider-specific execution path
or arbitrary shell command/URL argument. Setup continues to run on creation.
Runtime status is ephemeral and does not imply HTTP readiness.

Evidence: `delegation.rs` and `embedded_mcp.rs` unit tests, built-in
materialization tests in `mcp.rs`, and the opt-in picker test in
`e2e/tools.spec.ts`. Workspace controls and log limits are tested in
`delegation.rs`/`embedded_mcp.rs`, real PTY exit/output retention in `pty.rs`, and
preview navigation in `e2e/browser.spec.ts` on Chromium, with selected browser
interactions repeated on WebKit.
Browser tests use the catalog mock and do not run models.
Live model delegation requires installed/authenticated CLIs; automated tests
do not spend model credits. See the [contract](../contracts/embedded-mcp.md).

External local MCP hosts use the same stdio protocol and tools with either worker
provider, without a coordinator conversation in Prometeu. Registered clients have
explicit project scope and revocable credentials that survive desktop restart.
Explicit project delegation defaults to Claude; `provider` can select Codex.
Evidence: `mcp_access.rs` and `delegation.rs` authorization/compatibility tests,
and `tests/mcp_client.rs` for real stdio subprocess registration and socket
rediscovery against a fixture. No live provider or native webview is exercised
by that subprocess test. See [registration](../contracts/embedded-mcp.md#external-client-registration).


## Antigravity verification boundary

The integration targets installed agy 1.2.7, whose native headless protocol is
separate from Gemini CLI ACP. A real cached-account prompt returned `AGY_OK`;
`agy -p /usage` returned real remaining quotas by model group, with a TSV fixture
and parser regressions. `agy models` returned 14 account-visible slugs. Real tool execution, restart/resume
with a remembered marker, and SIGINT passed. Adapter fixtures and tests cover
text deltas, tools, completion, interruption, explicit native resume and the
`--add-dir` worktree argument on both creation and resume. Future real-runtime
verification must assert the tool's working directory, not only tool success.
The browser mock consumes only the generated V1 fixture.

Prometeu cannot offer agy interactive approval responses, managed multiple
Google accounts or a verified direct browser-login API. Initial plan
mode and hub selections are unavailable. The original issue's complete
account/approval scope is not concluded by this replacement. External login and
account switching remain owned by agy; no isolated-profile guarantee is made.
Former Gemini accounts and transcripts are preserved and cannot launch another
runtime accidentally. See [ADR 0052](../decisions/0052-antigravity-runtime.md).
