# Architecture decisions

Accepted ADRs describe decisions still implemented in this checkout. Each one
contains the current choice, its rationale, consequences and evidence. The
lifecycle is defined in [ADR 0044](0044-current-documentation.md).

## Maintenance

- Update the affected ADR and contract together with the implementation.
- When replacing a decision, consolidate the guarantees still in force into its
  replacement, update incoming links and delete the obsolete document.
- Keep compatibility requirements that the code still implements. Removing a
  document does not authorize deleting data or compatibility code.
- Keep ADR numbers stable; never reuse retired numbers or close numbering gaps.
- Use Git for previous versions and retired records: `git log --all --
  docs/decisions/` and `git show <commit>:<path>`.
- Keep this index and the documentation index aligned. Run `npm run docs:check`.

## States

- **Accepted:** implemented guidance in force.
- **Proposed:** under discussion; list separately from accepted decisions.

Superseded and rejected records belong in Git history, outside the current
navigation. This index describes repository code; deployment and external
service validation require their own evidence.

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

## Current decisions

| ADR | Subject |
| --- | --- |
| [0001](0001-repository-knowledge.md) | Knowledge versioned in the repository |
| [0002](0002-canonical-conversation-protocol.md) | Canonical conversation protocol |
| [0003](0003-agent-capabilities.md) | Capability-driven features |
| [0004](0004-prometeu-independent-identity.md) | Prometeu's independent identity |
| [0005](0005-portable-plugin-marketplace.md) | One portable marketplace for Claude and Codex |
| [0007](0007-persistent-session-comments.md) | Persistent comments alongside the session |
| [0008](0008-explicit-git-index.md) | explicit Git index and per-repository operations |
| [0009](0009-reusable-actions.md) | reusable commands and agents responsible for tasks |
| [0010](0010-default-code-review.md) | Code review included in the actions registry |
| [0012](0012-provider-accounts.md) | Local accounts and global selection per provider |
| [0013](0013-remove-provider-accounts.md) | Account removal and empty selection |
| [0015](0015-cloud-rails.md) | Optional account in a separate Rails service |
| [0017](0017-executable-design-system.md) | Executable Design System components |
| [0018](0018-native-file-promises.md) | receiving macOS file promises |
| [0020](0020-personal-catalog-and-local-items.md) | Catalog in the SaaS and explicit sharing on the desktop |
| [0021](0021-cloud-organizations.md) | Organizations and invitations in the Cloud |
| [0022](0022-end-to-end-encryption.md) | end-to-end encryption in collaboration |
| [0023](0023-ordered-publication.md) | Serialize snapshot publication and conversation delivery |
| [0024](0024-typed-ipc.md) | Command-owned IPC argument and result types |
| [0026](0026-portable-collaboration-core.md) | portable collaboration core and phone access through the relay |
| [0027](0027-companion-devices.md) | companion devices in the relay |
| [0028](0028-mobile-web-app.md) | Prometeu on the phone as a web app served by the Cloud |
| [0030](0030-remote-control.md) | remote control independent of sharing |
| [0034](0034-mobile-pairing-continuity.md) | Mobile pairing continuity |
| [0035](0035-feedback-requires-account.md) | Authenticated private feedback through the Cloud |
| [0036](0036-second-mac-as-companion.md) | a second Mac as a companion device |
| [0037](0037-browser-design-context.md) | browser with visual context for the conversation |
| [0038](0038-browser-context-chips.md) | selected elements as tags in the conversation |
| [0039](0039-organization-catalog-on-desktop.md) | Organization catalogs available on the desktop |
| [0040](0040-open-source.md) | Open source in a single public repository |
| [0041](0041-members-list-shows-people.md) | The members list shows people, not devices |
| [0042](0042-automatic-key-rotation.md) | A peer's new key is adopted automatically |
| [0043](0043-retire-unused-ipc.md) | Retire unused desktop IPC commands |
| [0044](0044-current-documentation.md) | Current documentation with history in Git |
| [0045](0045-layered-tool-selection.md) | Layered selection of MCP, plugins and skills across global, project and workspace |
| [0046](0046-cli-inherited-mcp-base.md) | CLI MCP set as visible inherited picker base |
| [0047](0047-tool-selection-boundaries.md) | Reviewed tool declarations, effective provider and native IPC patches |
| [0048](0048-scoped-cleanup-offer.md) | Scoped worktree cleanup after archiving or finishing |
| [0049](0049-embedded-delegation-mcp.md) | Native MCP for client-owned delegation |
| [0050](0050-tested-application-boundaries.md) | Tested application boundaries before service extraction |
| [0051](0051-git-project-catalog.md) | Git projects in portable catalogs |
| [0052](0052-antigravity-runtime.md) | Replace Gemini CLI with Antigravity |
| [0053](0053-live-model-selection.md) | Live model catalogs and explicit selection identity |
| [0054](0054-local-notifications.md) | Opt-in local notifications with independent sound |
| [0055](0055-linux-desktop.md) | Linux desktop through system programs |
| [0056](0056-background-tasks-hold-completion.md) | A turn only completes when its background tasks drain |
| [0057](0057-skill-kickoff-and-artifact-path.md) | Start a conversation from a skill, with a project-declared artifact path |
| [0058](0058-optional-context-evaluation.md) | Optional context evaluation behind an application-owned port |

| [0059](0059-local-telemetry-foundation.md) | Local telemetry with canonical events in SQLite |

| [0060](0060-isolated-desktop-presentation.md) | Isolated Desktop presentation and executable compositions |
