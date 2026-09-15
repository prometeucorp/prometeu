# Prometeu documentation

This directory is the project's technical source of truth. `AGENTS.md` works
as a short index for agents and `README.md` presents the product.

## Architecture

- [Design System](architecture/design-system.md): tokens, components, gallery and incremental adoption.

- [`../ARCHITECTURE.md`](../ARCHITECTURE.md): overall system map.
- [`architecture/conversation-flow.md`](architecture/conversation-flow.md):
  the path of a conversation and state ownership.
- [`architecture/dependency-rules.md`](architecture/dependency-rules.md):
  rules between presentation, application, domain and adapters.

## Contracts

- [Browser and visual context](contracts/browser.md): preview, inspection, captures and sending to the draft.

- [Private feedback](contracts/feedback.md): widget with account, text and images in a private GitHub repository.

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
- [`contracts/relay-v4.md`](contracts/relay-v4.md): encrypted collaboration and TOFU.
- [`contracts/relay-v3.md`](contracts/relay-v3.md): history of the protocol without E2EE.

## Decisions

- [Decision index and lifecycle](decisions/README.md).
- [ADR 0001](decisions/0001-repository-knowledge.md): repository knowledge as the source of truth — Accepted.
- [ADR 0002](decisions/0002-canonical-conversation-protocol.md): canonical conversation protocol — Accepted; mirror superseded.
- [ADR 0003](decisions/0003-agent-capabilities.md): capability-driven features — Accepted.
- [ADR 0004](decisions/0004-prometeu-independent-identity.md): Prometeu's independent identity — Accepted.
- [ADR 0005](decisions/0005-portable-plugin-marketplace.md): portable marketplace for Claude and Codex — Accepted.
- [ADR 0006](decisions/0006-explicit-prometheus-import.md): explicit, non-destructive Prometheus import — Accepted; importer removed.
- [ADR 0007](decisions/0007-persistent-session-comments.md): persistent comments alongside the session — Accepted.
- [ADR 0008](decisions/0008-explicit-git-index.md): explicit Git index and per-repository operations — Accepted.
- [ADR 0009](decisions/0009-reusable-actions.md): reusable commands and agents with local tracking — Accepted.
- [ADR 0010](decisions/0010-default-code-review.md): Code review included once, editable and removable — Accepted.
- [ADR 0011](decisions/0011-shared-ui.md): shared interface primitives — Partially superseded by 0016.
- [ADR 0012](decisions/0012-provider-accounts.md): local accounts and global selection per provider — Accepted.
- [ADR 0013](decisions/0013-remove-provider-accounts.md): removal of accounts and empty selection — Accepted.
- [ADR 0014](decisions/0014-optional-cloud-account.md): optional account and separate SaaS — Superseded (stack).
- [ADR 0015](decisions/0015-cloud-rails.md): SaaS in Rails with the desktop contract preserved — Accepted.
- [ADR 0016](decisions/0016-company-design-system.md): distributable Design System for the company's products — Partially superseded by 0017.
- [ADR 0017](decisions/0017-executable-design-system.md): executable components and Rails adapter for the Design System — Accepted.
- [ADR 0018](decisions/0018-native-file-promises.md): native reception of thumbnails and file promises — Accepted.
- [ADR 0019](decisions/0019-cloud-catalog.md): portable catalog of plugins, MCP and Actions in the Prometeu account — Superseded by 0020.
- [ADR 0020](decisions/0020-personal-catalog-and-local-items.md): authoring in the SaaS and explicit sharing of local items — Partially superseded by 0021.
- [ADR 0021](decisions/0021-cloud-organizations.md): organizations, email invitations and collaboration authorized by the Cloud — Accepted.
- [ADR 0022](decisions/0022-end-to-end-encryption.md): end-to-end encryption, TOFU and security limits — Accepted; partially superseded by 0042.
- [ADR 0023](decisions/0023-ordered-publication.md): ordered board publication and conversation delivery — Accepted.
- [ADR 0024](decisions/0024-typed-ipc.md): command-owned IPC arguments and results — Accepted.
- [ADR 0025](decisions/0025-completion-sound-per-execution.md): completion sound per accepted execution, independent of reading — Partially superseded by 0029.
- [ADR 0026](decisions/0026-portable-collaboration-core.md): portable collaboration core and phone access through the relay — Accepted; stages decided in ADRs 0027 and 0028.
- [ADR 0027](decisions/0027-companion-devices.md): companion devices in the relay with the additive `person` field — Accepted; extended by 0036, partially superseded by 0041.
- [ADR 0028](decisions/0028-mobile-web-app.md): Prometeu on the phone as a web app served by the Cloud — Accepted.
- [ADR 0029](decisions/0029-remove-alert-sound.md): removal of sound alerts, preserving visual indicators — Accepted.
- [ADR 0030](decisions/0030-remote-control.md): personal remote control independent of the team audience — Accepted.

- [ADR 0031](decisions/0031-public-feedback.md): public feedback through the Cloud and GitHub Issues — Superseded by 0032.

- [ADR 0032](decisions/0032-private-feedback.md): private feedback in the Cloud repository — Superseded by 0033.
- [ADR 0033](decisions/0033-github-feedback-attachments.md): feedback text and images in GitHub — Superseded by 0035.
- [ADR 0034](decisions/0034-mobile-pairing-continuity.md): relay renewal without disconnection and mobile pairing continuity — Accepted.
- [ADR 0035](decisions/0035-feedback-requires-account.md): feedback requires a Prometeu account — Accepted.
- [ADR 0036](decisions/0036-second-mac-as-companion.md): a second Mac on the same account joins as a companion device — Accepted.
- [ADR 0037](decisions/0037-browser-design-context.md): browser with visual context for the conversation — Accepted.
- [ADR 0038](decisions/0038-browser-context-chips.md): selected elements as tags in the conversation — Accepted.
- [ADR 0039](decisions/0039-organization-catalog-on-desktop.md): organization catalogs available on the desktop — Accepted.
- [ADR 0040](decisions/0040-open-source.md): open source in a single public repository — Accepted; partially supersedes ADR 0004.
- [ADR 0041](decisions/0041-members-list-shows-people.md): the members list shows people and drops the permanent security code — Accepted.
- [ADR 0042](decisions/0042-automatic-key-rotation.md): a peer's new key is adopted automatically, with no block or review — Accepted.
- [ADR 0043](decisions/0043-layered-tool-selection.md): layered selection of MCP, plugins and skills across global, project and workspace — Accepted; Authority amended by ADR 0044.
- [ADR 0044](decisions/0044-cli-inherited-mcp-base.md): the CLI's MCP set is the visible inherited base of the picker — Accepted.

## Quality and operations

- [`quality/provider-matrix.md`](quality/provider-matrix.md): support per agent
  and expected evidence.
- [`operations/development.md`](operations/development.md): environment and tests.
- [`operations/release.md`](operations/release.md): CI, versioning and release.

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
