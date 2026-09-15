# Provider matrix

Status: current behavior observed in the code. "To confirm" means the feature
may exist, but does not yet have enough conformance evidence to become a
contractual capability.

## Legend

- **Native:** the CLI already speaks the form consumed today.
- **Adapted:** Prometeu converts or implements the feature.
- **Unavailable:** the UI does not offer it because the provider does not
  support the flow.
- **To confirm:** a fixture or dedicated test is missing.

| Capability | Claude | Codex | Main evidence |
| --- | --- | --- | --- |
| private feedback with an account, image and capture | independent of the CLI | independent of the CLI | `e2e/feedback.spec.ts`, the Cloud's `FeedbackTest` tests; native capture and real GitHub require a manual smoke test |
| organizations, invitations and institutional sharing | the same relay V4; local execution | the same relay V4; local execution | `team-organizations.test.ts`, `worker.integration.test.ts`, `e2e/organizations.spec.ts`, Rails integration/browser |
| automatic E2EE with TOFU in collaboration | the same HPKE Auth channel; no forward secrecy | the same HPKE Auth channel; no forward secrecy | `team-crypto.test.ts`, `team-security.test.ts`, `team-channel.test.ts`, `worker.integration.test.ts`, E2E comments in Chromium/WebKit |
| optional Prometeu account in the sidebar | independent of the CLI | independent of the CLI | `cloud.rs`, `e2e/cloud.spec.ts`; no transcript is sent |
| catalog of plugins, MCP and Actions in the account | independent of the CLI | independent of the CLI | `catalog.rs`, `catalog_test.rb`; secrets and installation stay per Mac |
| the organization's plugins, MCPs and skills | explicit local installation in the hub | the same UI and installation; existing adapters | `catalog.rs`, `e2e/cloud.spec.ts`, `organizations_test.rb`; it requires no personal copy and does not activate automatically |
| Actions form with shared components | the same UI | the same UI; options come from the catalog | `e2e/ui.spec.ts`, `e2e/actions.spec.ts`, Chromium and WebKit |
| the company's executable DS components | independent of the provider | independent of the provider | `e2e/design-system.spec.ts`, menu, submenu, password, focus, validation and error on both engines |
| Code review included and editable | the initial profile; the model/provider can be changed | can be chosen in the profile | `actions.rs`, `actions.test.ts`, `e2e/actions.spec.ts` |
| prompt commands and tasks | adapted by the app | adapted by the app | `actions.test.ts`, `e2e/actions.spec.ts` |
| per-task profile | instructions and permissions through flags | instructions and permissions through JSON-RPC | `session.rs` and `codex.rs` tests |
| PR tracking | local polling by the app | local polling by the app | `actions.rs`, `github.rs`; no real GitHub integration in tests |
| detecting the installation | adapted | adapted | `agents.rs` |
| accounts and global selection in the footer | adapted through `CLAUDE_CONFIG_DIR` | adapted through `CODEX_HOME` | `accounts.rs`, `e2e/accounts.spec.ts` |
| removing every account and an empty selection | supported; preserves the CLI's login | supported; preserves the CLI's login | `accounts.rs`, `e2e/accounts.spec.ts`; directories and credentials stay local |
| login through the app | the CLI's `auth login` and the browser | the app-server's `account/login/start` and the browser | identity fixtures in `claude.rs` and `codex/account.rs`; OAuth with two real accounts still requires manual validation |
| switching accounts between turns | resuming the shared transcript | resuming the shared rollout/index | `chat.rs`, profile tests; an authenticated continuation between two real accounts is not yet proven by the suite |
| quotas per account | stream and internal endpoint | app-server and internal fallback | `usage.rs`, `e2e/accounts.spec.ts` |
| live model catalog | native through a control request | native through the CLI's cache | `agents.rs` |
| starting a session | native | adapted to JSON-RPC | `chat.rs`, `codex.rs` |
| resuming a session | Claude's id/transcript | the app-server's thread | `session.rs`, Rust tests |
| workspace tools with tab model/effort overrides | same selections for new and resumed tabs | same selections for new and resumed tabs | `session.rs::new_and_resumed_tabs_preserve_workspace_tools_with_model_overrides` |
| ordered prompt, transcript, and live delivery | shared conversation mutex | shared conversation mutex | concurrent delivery, fast-response, and failed-write regressions in `chat.rs` |
| choosing a model | native through a flag | adapted in `thread/start`/`thread/resume` | `session.rs`, `codex.rs` |
| effort levels | catalog + fallback | Codex's catalog | `agents.rs`, `launcher.ts` |
| conversation footer adapted to the width | model and activity separated from the tools; remote control highlighted when active | the same UI | `e2e/composer.spec.ts`, Chromium/WebKit, Portuguese/English and narrow frames |
| initial plan mode | native through permission mode | unavailable | `session.rs`, `launcher.ts` |
| streaming text | adapted to V1 | adapted to V1 | `conversation.test.ts`, `timeline.test.ts`, `codex.rs` tests |
| thinking | adapted to V1 | adapted to V1 | `timeline.test.ts`, `codex.rs` tests |
| tool call and result | adapted to V1 | adapted to V1 | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| questions to the user | native | adapted from a JSON-RPC request | `chat.ts`, `codex.rs` tests |
| approval requests | native | adapted | `chat.rs`, `codex.rs` |
| interruption | control request | `turn/interrupt` | `codex.rs`, Rust tests |
| compaction | the CLI's command | `thread/compact/start` | `codex.rs` tests |
| context report | stream/transcript | synthesized from token usage | `context.test.ts`, `codex.rs` tests |
| MCP selection per workspace | the CLI's strict config | table and environment assembled by the app | `mcp.rs`, `codex.rs`; a preparation error prevents the spawn |
| plugin selection per workspace | session flags | marketplace + config isolated per workspace | `plugins.rs`, the CLI smoke test, `codex.rs`, `launcher.ts`, E2E |
| local and account skills | a package with SKILL.md through the plugin selection | the same package with a native manifest | `skills.rs`, `catalog.rs`, `e2e/cloud.spec.ts`; installation does not activate automatically |
| layered selection (global, project, workspace) per axis | resolved at spawn before the adapter | resolved at spawn before the adapter | `selection.rs` resolve tests, `session.rs::provenance_classifica_cada_item_do_hub`, `session.rs::projeto_so_injeta_depois_de_aprovado_e_reprova_quando_o_hash_muda` and `session.rs::o_payload_do_eixo_e_validado_antes_de_gravar`; the project `[tools]` layer is gated on trust-on-first-use of its hash, an undecided item stays `pending` and a rejected one stays `rejected` (both resolved yet not injected), and the setters refuse a malformed payload or an id on the wrong axis |
| CLI-inherited MCP base (ADR 0044) | discovered from `~/.claude.json` and the working directory's `.mcp.json` plus its ancestors, nearest first; visible in the picker with the `cli` provenance and removable as a workspace delta; a declared axis materializes the whole effective set through the strict config | no discovered base; the CLI keeps loading its own configuration outside the picker | `selection.rs::a_base_do_cli_participa_da_cadeia`, `mcp.rs` inherited/universe tests and `mcp.rs::escolhido_ausente_impede_a_materializacao`, `session.rs::provenance_classifica_a_base_herdada_do_cli`, `src/mcp.test.ts`, the picker scenario in `e2e/critical-flows.spec.ts` |
| hooks of a chosen plugin | active from `SessionStart` | `enabled = true` + trust limited to the `pluginId` and hash before the thread | `codex.rs` tests; a failure prevents the thread |
| attachments in a message, capture thumbnails and pasting | adapted through a local path; promise and pasteboard materialized by macOS | adapted through a local path; promise and pasteboard materialized by macOS | `file_drop.rs`, `chat.ts`, `paste.ts`; `e2e/file-drop.spec.ts` and dropped-file scenarios in `e2e/critical-flows.spec.ts` cover the UI over the mock; `paste.test.ts` covers the paste detour |
| the browser's visual context | a tag in the draft and the history; complete HTML, CSS, URL and PNG mention on send | the same interface and textual contract | `browser-context.test.ts`, `e2e/browser-inspector.spec.ts`, `e2e/browser.spec.ts`, `browser.rs` tests; WKWebView capture and the AppKit gesture still require native verification |
| unknown external event | ignored by the adapter | ignored by the adapter | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| the CLI's subagents | a sidechain off-screen; tasks in `background.changed` | `collabAgentToolCall.agentsStates`, `subAgentActivity` and known child events update the tasks; the children's content stays isolated | `claude.rs`; isolation, spawn, activity and partial-state tests in `codex.rs` |
| attention indicators without audio | completion, question and comment pending items in the Dock | the same rule over live local V1 events | `src/alert.test.ts` preserves counting and reading; `e2e/alerts.spec.ts` in Chromium/WebKit covers the absence of audio and of the sound option |
| live sharing | V1 after normalization | V1 after normalization | `team*.test.ts`, E2E over the mock |
| remote control from the owner's devices | the same relay v4; execution stays local | the same relay v4; execution stays local | `team-channel.test.ts`, `team-organizations.test.ts`, `e2e/organizations.spec.ts` |
| comments in a shared session | adapted after V1 | adapted after V1 | `notes.test.ts`, `team.test.ts`, `relay/src/logic.test.ts`, E2E over the mock |
| desk with several conversations at once | adapted (the same conversation screen) | adapted (the same conversation screen) | `desk.test.ts`, E2E over the mock |
| Git: unified/side-by-side review, stage, commit, remotes, branches and conflicts | adapted by the app; independent of the CLI | adapted by the app; independent of the CLI | `session/git_tests.rs`, `diff.test.ts`, `e2e/git.spec.ts` in Chromium/WebKit and a large review in `e2e/critical-flows.spec.ts`; the `git.md` contract |
| agents per workspace in the sidebar | each tab's brand and status | each tab's brand and status | sidebar flows in `e2e/critical-flows.spec.ts`; a remote one uses the owner's avatar, without inferring the provider |

## Rule for a new feature

Before enabling a feature for a provider:

1. declare its common semantics in the contract;
2. add or adjust the capability in the descriptor;
3. capture a real CLI fixture without secrets or personal data;
4. prove the translation into common events;
5. run the same scenario in the reducer and in the mock;
6. update this matrix with the path to the evidence.

A missing test must not become `true` by similarity between providers.

## Known limitations

- a plugin source in a local `.zip` or a URL works in Claude and is refused with
  a visible error in Codex; a local folder is the portable format;
- skills, commands, MCP and hooks have a portable adaptation; `agents/*.md`
  stays exclusive to Claude because it is not part of the current Codex
  manifest;
- plugins enabled outside Prometeu stay subject to each CLI's global registry
  and are not part of the workspace's selection;
- the CLI-inherited MCP base (ADR 0044) covers Claude only; servers configured
  for Codex in `~/.codex/config.toml` are not discovered and stay invisible to
  the picker, a recorded follow-up;
- attachments have UI tests over the mock and native validation of the saved
  destination; the real thumbnail gesture was confirmed in Prometeu Dev on
  2026-09-06. Actual reading by the CLI still requires manual verification.
  Promises that fail or exceed 30 seconds produce a visible error.

## Desired conformance suite

| Scenario | External fixture | Adapter | Reducer | E2E |
| --- | --- | --- | --- | --- |
| simple message | per provider | required | required | smoke |
| streaming + final message | per provider | required | required | critical |
| successful tool | per provider | required | required | critical |
| tool with an error | per provider | required | required | critical |
| question and answer | per provider | required | required | critical |
| interruption | per provider | required | required | smoke |
| resume | per provider | required | replay | critical |
| compaction | per capable provider | required | required | smoke |
| background task | per capable provider | required | required | smoke |
| unknown event | synthetic | required | required | not needed |

E2E does not replace the contract: it covers a few expensive paths. Fixtures and
reducers give fast diagnosis for every combination.
