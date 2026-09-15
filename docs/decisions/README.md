# Architecture decision record

Each ADR records a significant decision, its context and its consequences.
Use a new file to change an accepted decision; do not rewrite the past.

## States

- **Proposed:** under discussion; not a rule in force.
- **Accepted:** the project's current guidance.
- **Superseded:** another ADR took its place; keep links in both directions.
- **Rejected:** considered and not adopted.

## Template

```md
# ADR NNNN — title

Date: YYYY-MM-DD
Status: Proposed

## Context

## Options considered

## Decision

## Consequences

## Evidence
```

## Index

| ADR | State | Subject |
| --- | --- | --- |
| [0001](0001-repository-knowledge.md) | Accepted | repository knowledge as the source of truth |
| [0002](0002-canonical-conversation-protocol.md) | Accepted; mirror superseded | canonical conversation protocol |
| [0003](0003-agent-capabilities.md) | Accepted | capability-driven features |
| [0004](0004-prometeu-independent-identity.md) | Accepted | Prometeu's independent identity |
| [0005](0005-portable-plugin-marketplace.md) | Accepted | portable marketplace for Claude and Codex |
| [0006](0006-explicit-prometheus-import.md) | Accepted; importer removed | explicit, non-destructive Prometheus import |
| [0007](0007-persistent-session-comments.md) | Accepted | persistent comments alongside the session |
| [0008](0008-explicit-git-index.md) | Accepted | explicit Git index and per-repository operations |
| [0009](0009-reusable-actions.md) | Accepted | reusable commands and agents with local tracking |
| [0010](0010-default-code-review.md) | Accepted | Code review included once, editable and removable |
| [0011](0011-shared-ui.md) | Partially superseded by 0016 | shared interface primitives |
| [0012](0012-provider-accounts.md) | Accepted | local accounts and global selection per provider |
| [0013](0013-remove-provider-accounts.md) | Accepted | removal of accounts and empty selection |
| [0014](0014-optional-cloud-account.md) | Superseded (stack) | optional account and separate SaaS |
| [0015](0015-cloud-rails.md) | Accepted | SaaS in Rails with the desktop contract preserved |
| [0016](0016-company-design-system.md) | Partially superseded by 0017 | distributable Design System for the company's products |
| [0017](0017-executable-design-system.md) | Accepted | executable components and Rails adapter for the Design System |
| [0018](0018-native-file-promises.md) | Accepted | native reception of thumbnails and file promises |
| [0019](0019-cloud-catalog.md) | Superseded by 0020 | portable catalog of plugins, MCP and Actions in the Prometeu account |
| [0020](0020-personal-catalog-and-local-items.md) | Partially superseded by 0021 | authoring in the SaaS and explicit sharing of local items |
| [0021](0021-cloud-organizations.md) | Accepted | organizations, email invitations and collaboration authorized by the Cloud |
| [0022](0022-end-to-end-encryption.md) | Accepted; partially superseded by 0042 | end-to-end encryption in collaboration |
| [0023](0023-ordered-publication.md) | Accepted | ordered board publication and conversation delivery |
| [0024](0024-typed-ipc.md) | Accepted | command-owned IPC arguments and results |
| [0025](0025-completion-sound-per-execution.md) | Partially superseded by 0029 | completion sound per accepted execution |
| [0026](0026-portable-collaboration-core.md) | Accepted; stages decided in 0027 and 0028 | portable collaboration core and phone access through the relay |
| [0027](0027-companion-devices.md) | Accepted; extended by 0036, partially superseded by 0041 | companion devices in the relay with the additive `person` field |
| [0028](0028-mobile-web-app.md) | Accepted | Prometeu on the phone as a web app served by the Cloud |
| [0029](0029-remove-alert-sound.md) | Accepted | removal of sound alerts, preserving visual indicators |
| [0030](0030-remote-control.md) | Accepted | personal remote control independent of the team audience |
| [0031](0031-public-feedback.md) | Superseded by 0032 | public feedback |
| [0032](0032-private-feedback.md) | Superseded by 0033 | private feedback in the Cloud repository |
| [0033](0033-github-feedback-attachments.md) | Superseded by 0035 | feedback text and images in GitHub |
| [0035](0035-feedback-requires-account.md) | Accepted | feedback requires a Prometeu account |
| [0036](0036-second-mac-as-companion.md) | Accepted | a second Mac on the same account joins as a companion device |
| [0037](0037-browser-design-context.md) | Accepted | browser with visual context for the conversation |
| [0038](0038-browser-context-chips.md) | Accepted | selected elements as tags in the conversation |
| [0039](0039-organization-catalog-on-desktop.md) | Accepted | organization catalogs available on the desktop |
| [0040](0040-open-source.md) | Accepted | open source in a single public repository |
| [0041](0041-members-list-shows-people.md) | Accepted | the members list shows people and drops the permanent security code |
| [0042](0042-automatic-key-rotation.md) | Accepted | a peer's new key is adopted automatically, with no block or review |
| [0043](0043-layered-tool-selection.md) | Accepted; Authority amended by 0044 | layered selection of MCP, plugins and skills across global, project and workspace |
| [0044](0044-cli-inherited-mcp-base.md) | Accepted | the CLI's MCP set is the visible inherited base of the picker |
