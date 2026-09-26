# Prometeu documentation

This directory is the project's technical source of truth. `AGENTS.md` works
as a short index for agents and `README.md` presents the product.

## Architecture

- [Design System](architecture/design-system.md): tokens, components, gallery and incremental adoption.
- [Desktop composition](architecture/desktop-composition.md): agent recipe for isolated screens and executable examples.

- [`../ARCHITECTURE.md`](../ARCHITECTURE.md): overall system map.
- [`architecture/conversation-flow.md`](architecture/conversation-flow.md):
  the path of a conversation and state ownership.
- [`architecture/dependency-rules.md`](architecture/dependency-rules.md):
  rules between presentation, application, domain and adapters.

## Contracts

- [Desktop presentation](contracts/desktop-presentation.md): typed snapshots, callbacks, focus and resource view lifetime.

- [Local notifications](contracts/notifications.md): event selection, native delivery and silent defaults.

- [Optional context evaluation](contracts/context-evaluation.md): bring-your-own-key TypeSafe integration, evaluation port and missing-context review.

- [Embedded MCP and delegation](contracts/embedded-mcp.md): local, opt-in orchestration infrastructure with client-owned agents and optional conversation context.

- [Browser and visual context](contracts/browser.md): preview, inspection, captures and sending to the draft.

- [Issue reporting and private feedback](contracts/feedback.md): public GitHub issues or account-backed private reports with text and images.

- [Organizations in the Cloud](contracts/cloud-organizations.md): CRUD, invitations, catalogs and relay access.

- [Catalog in the Prometeu account](contracts/cloud-catalog.md): plugins, MCP and skills managed in the SaaS, with private items and explicit sharing on the desktop.
- [Optional Prometeu account](contracts/cloud-account.md): SaaS, browser-based
  authentication, desktop connection and private persistence.

- [Reusable actions](contracts/actions.md): commands, profiles and local PR tracking.
- [`contracts/accounts.md`](contracts/accounts.md): accounts, login, global
  selection and quota isolation between agents.

- [`contracts/conversation-events-v1.md`](contracts/conversation-events-v1.md):
  canonical protocol owned by Prometeu.
- [`contracts/agent-runtime.md`](contracts/agent-runtime.md): discovery,
  capabilities and the agent execution port.
- [`contracts/plugin-marketplace.md`](contracts/plugin-marketplace.md): hub,
  portable package and per-provider adaptation.
- [`contracts/ipc.md`](contracts/ipc.md): the TypeScript/Rust boundary.
- [`contracts/git.md`](contracts/git.md): index, worktree, review and Git operations.
- [`contracts/persistence.md`](contracts/persistence.md): board and transcripts.
- [`contracts/releases.md`](contracts/releases.md): desktop packages, updater manifest and installation ownership.
- [`contracts/relay-v4.md`](contracts/relay-v4.md): encrypted collaboration and TOFU.

## Decisions

[Lifecycle and current index](decisions/README.md). Only implemented decisions
appear below. Retired documents and previous versions remain in Git history;
compatibility still in use stays documented in the current contracts.

- [ADR 0001](decisions/0001-repository-knowledge.md): Knowledge versioned in the repository.
- [ADR 0002](decisions/0002-canonical-conversation-protocol.md): Canonical conversation protocol.
- [ADR 0003](decisions/0003-agent-capabilities.md): Capability-driven features.
- [ADR 0004](decisions/0004-prometeu-independent-identity.md): Prometeu's independent identity.
- [ADR 0005](decisions/0005-portable-plugin-marketplace.md): One portable marketplace for Claude and Codex.
- [ADR 0007](decisions/0007-persistent-session-comments.md): Persistent comments alongside the session.
- [ADR 0008](decisions/0008-explicit-git-index.md): explicit Git index and per-repository operations.
- [ADR 0009](decisions/0009-reusable-actions.md): reusable commands and agents responsible for tasks.
- [ADR 0010](decisions/0010-default-code-review.md): Code review included in the actions registry.
- [ADR 0012](decisions/0012-provider-accounts.md): Local accounts and global selection per provider.
- [ADR 0013](decisions/0013-remove-provider-accounts.md): Account removal and empty selection.
- [ADR 0015](decisions/0015-cloud-rails.md): Optional account in a separate Rails service.
- [ADR 0017](decisions/0017-executable-design-system.md): Executable Design System components.
- [ADR 0018](decisions/0018-native-file-promises.md): receiving macOS file promises.
- [ADR 0020](decisions/0020-personal-catalog-and-local-items.md): Catalog in the SaaS and explicit sharing on the desktop.
- [ADR 0021](decisions/0021-cloud-organizations.md): Organizations and invitations in the Cloud.
- [ADR 0022](decisions/0022-end-to-end-encryption.md): end-to-end encryption in collaboration.
- [ADR 0023](decisions/0023-ordered-publication.md): Serialize snapshot publication and conversation delivery.
- [ADR 0024](decisions/0024-typed-ipc.md): Command-owned IPC argument and result types.
- [ADR 0026](decisions/0026-portable-collaboration-core.md): portable collaboration core and phone access through the relay.
- [ADR 0027](decisions/0027-companion-devices.md): companion devices in the relay.
- [ADR 0028](decisions/0028-mobile-web-app.md): Prometeu on the phone as a web app served by the Cloud.
- [ADR 0030](decisions/0030-remote-control.md): remote control independent of sharing.
- [ADR 0034](decisions/0034-mobile-pairing-continuity.md): Mobile pairing continuity.
- [ADR 0035](decisions/0035-feedback-requires-account.md): Authenticated private feedback through the Cloud.
- [ADR 0036](decisions/0036-second-mac-as-companion.md): a second Mac as a companion device.
- [ADR 0037](decisions/0037-browser-design-context.md): browser with visual context for the conversation.
- [ADR 0038](decisions/0038-browser-context-chips.md): selected elements as tags in the conversation.
- [ADR 0039](decisions/0039-organization-catalog-on-desktop.md): Organization catalogs available on the desktop.
- [ADR 0040](decisions/0040-open-source.md): Open source in a single public repository.
- [ADR 0041](decisions/0041-members-list-shows-people.md): The members list shows people, not devices.
- [ADR 0042](decisions/0042-automatic-key-rotation.md): A peer's new key is adopted automatically.
- [ADR 0043](decisions/0043-retire-unused-ipc.md): Retire unused desktop IPC commands.
- [ADR 0044](decisions/0044-current-documentation.md): Current documentation with history in Git.
- [ADR 0045](decisions/0045-layered-tool-selection.md): Layered selection of MCP, plugins and skills across global, project and workspace.
- [ADR 0046](decisions/0046-cli-inherited-mcp-base.md): CLI MCP set as visible inherited picker base.
- [ADR 0047](decisions/0047-tool-selection-boundaries.md): Reviewed tool declarations, effective provider and native IPC patches.
- [ADR 0048](decisions/0048-scoped-cleanup-offer.md): scoped worktree cleanup after archiving or finishing.

- [ADR 0049](decisions/0049-embedded-delegation-mcp.md): Native MCP for client-owned delegation.
- [ADR 0050](decisions/0050-tested-application-boundaries.md): Tested application boundaries before service extraction.
- [ADR 0051](decisions/0051-git-project-catalog.md): Git projects in portable catalogs.

- [ADR 0052](decisions/0052-antigravity-runtime.md): Replace Gemini CLI with Antigravity.

- [ADR 0053](decisions/0053-live-model-selection.md): Live model catalogs and explicit selection identity.
- [ADR 0054](decisions/0054-local-notifications.md): opt-in local notifications with independent sound.
- [ADR 0055](decisions/0055-linux-desktop.md): Linux desktop through system programs.
- [ADR 0056](decisions/0056-background-tasks-hold-completion.md): a turn only completes when its background tasks drain.
- [ADR 0057](decisions/0057-skill-kickoff-and-artifact-path.md): start a conversation from a skill, with a project-declared artifact path.
- [ADR 0058](decisions/0058-optional-context-evaluation.md): optional context evaluation behind an application-owned port.
- [ADR 0059](decisions/0059-local-telemetry-foundation.md): local telemetry with common columns and typed JSON payloads in SQLite.
- [ADR 0060](decisions/0060-isolated-desktop-presentation.md): isolated Desktop presentation with a shared production/gallery view.
- [ADR 0061](decisions/0061-background-energy-policy.md): explicit system-only inhibition while a macOS agent works.

## Local telemetry

- [Local telemetry contract](contracts/telemetry.md): canonical content-free
  events, usage semantics, SQLite persistence, coverage, queries and verification.

## Quality and operations

- [Desktop Design System audit](quality/desktop-design-system-audit.md):
  baseline evidence, preserved identity and the implemented Settings pilot.
- [`quality/provider-matrix.md`](quality/provider-matrix.md): support per agent
  and expected evidence.
- [Energy profiling](quality/energy-profile.md): issue #148 release-build
  measurement protocol and source-level baseline.
- [`operations/development.md`](operations/development.md): environment and tests.
- [`operations/contributing.md`](operations/contributing.md): recipes for provider,
  IPC, collaboration and Cloud contract changes; [contribution workflow](../CONTRIBUTING.md).
- [`operations/release.md`](operations/release.md): CI, versioning and release.
- [`operations/linux.md`](operations/linux.md): building, installing and platform differences on Linux.

## Update rule

A change must update the document that answers the affected question:

- visible behavior: `README.md` or the feature documentation;
- responsibility or flow: architecture;
- format between layers: contract;
- choice and trade-offs: ADR;
- support per agent: the provider matrix;
- build, test or publication: operations.

Proposed documentation must declare its status. It does not describe the
current system until the corresponding implementation is accepted.

## References for this approach

- [OpenAI — Harness engineering](https://openai.com/index/harness-engineering/):
  a short `AGENTS.md` as the map and a versioned `docs/` as the source of truth.
- [AGENTS.md](https://agents.md/): common instruction format for agents.
- [Claude Code — project memory](https://code.claude.com/docs/en/memory):
  concise instructions scoped to the repository.
- [C4 Model](https://c4model.com/diagrams): context and containers for the
  architectural map.
- [AWS — Architecture Decision Records](https://docs.aws.amazon.com/prescriptive-guidance/latest/architectural-decision-records/adr-process.html):
  context, decision, consequences and ADR lifecycle.
