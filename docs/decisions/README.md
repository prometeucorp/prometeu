# Architecture decisions

Stable ADRs describe accepted decisions still implemented in this checkout.
Each one contains the current choice, its rationale, consequences and evidence.
The lifecycle is defined in [ADR 0044](0044-current-documentation.md).

## Maintenance

- Update the affected ADR and contract together with the implementation.
- When replacing a decision, consolidate the guarantees still in force into its
  replacement, update incoming links and delete the obsolete document.
- Keep compatibility requirements that the code still implements. Removing a
  document does not authorize deleting data or compatibility code.
- Keep ADR numbers stable; never reuse retired numbers or close numbering gaps.
- Use Git for previous versions and retired records: `git log --all --
  docs/decisions/` and `git show <commit>:<path>`.
- List each decision here; the bundle map links this index. Run `npm run docs:check`.

## States

- **stable:** accepted, implemented guidance in force. This does not assert
  deployment, feature completeness or verification beyond the stated evidence.
- **draft:** proposed guidance under discussion; list separately from current
  decisions and do not treat it as implemented behavior.
- **deprecated:** still-supported legacy guidance with an explicit replacement
  and compatibility scope. Retired decisions are deleted, not kept in this state.

Superseded and rejected records belong in Git history, outside the current
navigation. This index describes repository code; deployment and external
service validation require their own evidence.

## Template

Every concept in the bundle starts with four single-line YAML string fields.
The type follows its area: `Architecture`, `Contract`, `Decision`, `Operations`
or `Quality`. Use plain strings without YAML punctuation, single-quoted strings
(double embedded apostrophes), or JSON-compatible double-quoted strings as below.
No timestamps for generation, verification or expiry are required on human-written
pages. Keep ADR dates in the body and use only the YAML field for lifecycle status.
Folder `README.md` indexes do not need frontmatter.

```md
---
type: Decision
title: "ADR NNNN — title"
description: "The choice this decision records and why it matters."
status: draft
---

# ADR NNNN — title

Date: YYYY-MM-DD

## Context

## Options considered

## Decision

## Consequences

## Evidence
```

Add a title and one-line description to this index. List drafts under a separate
Proposals heading; move them to Current decisions only when implemented. Run
`npm run docs:check` to validate metadata, local links and index coverage.

## Current decisions

- [ADR 0001](0001-repository-knowledge.md) — A versioned knowledge bundle with short navigation and shared standing instructions.
- [ADR 0002](0002-canonical-conversation-protocol.md) — Canonical conversation protocol.
- [ADR 0003](0003-agent-capabilities.md) — Capability-driven features.
- [ADR 0004](0004-prometeu-independent-identity.md) — Prometeu's independent identity.
- [ADR 0005](0005-portable-plugin-marketplace.md) — One portable marketplace for Claude and Codex.
- [ADR 0007](0007-persistent-session-comments.md) — Persistent comments alongside the session.
- [ADR 0008](0008-explicit-git-index.md) — Explicit Git index and per-repository operations.
- [ADR 0009](0009-reusable-actions.md) — Reusable commands and agents responsible for tasks.
- [ADR 0010](0010-default-code-review.md) — Code review included in the actions registry.
- [ADR 0012](0012-provider-accounts.md) — Local accounts and global selection per provider.
- [ADR 0013](0013-remove-provider-accounts.md) — Account removal and empty selection.
- [ADR 0015](0015-cloud-rails.md) — Optional account in a separate Rails service.
- [ADR 0017](0017-executable-design-system.md) — Executable Design System components.
- [ADR 0018](0018-native-file-promises.md) — Receiving macOS file promises.
- [ADR 0020](0020-personal-catalog-and-local-items.md) — Catalog in the SaaS and explicit sharing on the desktop.
- [ADR 0021](0021-cloud-organizations.md) — Organizations and invitations in the Cloud.
- [ADR 0022](0022-end-to-end-encryption.md) — End-to-end encryption in collaboration.
- [ADR 0023](0023-ordered-publication.md) — Serialize snapshot publication and conversation delivery.
- [ADR 0024](0024-typed-ipc.md) — Command-owned IPC argument and result types.
- [ADR 0026](0026-portable-collaboration-core.md) — Portable collaboration core and phone access through the relay.
- [ADR 0027](0027-companion-devices.md) — Companion devices in the relay.
- [ADR 0028](0028-mobile-web-app.md) — Prometeu on the phone as a web app served by the Cloud.
- [ADR 0030](0030-remote-control.md) — Remote control independent of sharing.
- [ADR 0034](0034-mobile-pairing-continuity.md) — Mobile pairing continuity.
- [ADR 0035](0035-feedback-requires-account.md) — Authenticated private feedback through the Cloud.
- [ADR 0036](0036-second-mac-as-companion.md) — A second Mac as a companion device.
- [ADR 0037](0037-browser-design-context.md) — Browser with visual context for the conversation.
- [ADR 0038](0038-browser-context-chips.md) — Selected elements as tags in the conversation.
- [ADR 0039](0039-organization-catalog-on-desktop.md) — Organization catalogs available on the desktop.
- [ADR 0040](0040-open-source.md) — Open source in a single public repository.
- [ADR 0041](0041-members-list-shows-people.md) — The members list shows people, not devices.
- [ADR 0042](0042-automatic-key-rotation.md) — A peer's new key is adopted automatically.
- [ADR 0043](0043-retire-unused-ipc.md) — Retire unused desktop IPC commands.
- [ADR 0044](0044-current-documentation.md) — Current documentation with history in Git.
- [ADR 0045](0045-layered-tool-selection.md) — Layered selection of MCP, plugins and skills across global, project and workspace.
- [ADR 0046](0046-cli-inherited-mcp-base.md) — CLI MCP set as visible inherited picker base.
- [ADR 0047](0047-tool-selection-boundaries.md) — Reviewed tool declarations, effective provider and native IPC patches.
- [ADR 0048](0048-scoped-cleanup-offer.md) — Scoped worktree cleanup after archiving or finishing.
- [ADR 0049](0049-embedded-delegation-mcp.md) — Native MCP for client-owned delegation.
- [ADR 0050](0050-tested-application-boundaries.md) — Tested application boundaries before service extraction.
- [ADR 0051](0051-git-project-catalog.md) — Git projects in portable catalogs.
- [ADR 0052](0052-antigravity-runtime.md) — Replace Gemini CLI with Antigravity.
- [ADR 0053](0053-live-model-selection.md) — Live model catalogs and explicit selection identity.
- [ADR 0054](0054-local-notifications.md) — Opt-in local notifications with independent sound.
- [ADR 0055](0055-linux-desktop.md) — Linux desktop through system programs.
- [ADR 0056](0056-background-tasks-hold-completion.md) — A turn only completes when its background tasks drain.
- [ADR 0057](0057-skill-kickoff-and-artifact-path.md) — Start a conversation from a skill, with a project-declared artifact path.
- [ADR 0058](0058-optional-context-evaluation.md) — Optional context evaluation behind an application-owned port.
- [ADR 0059](0059-local-telemetry-foundation.md) — Local telemetry with common columns and typed JSON payloads in SQLite.
- [ADR 0060](0060-isolated-desktop-presentation.md) — Isolated Desktop presentation with a shared production/gallery view.
- [ADR 0061](0061-background-energy-policy.md) — Explicit system-only inhibition while a macOS agent works.
- [ADR 0062](0062-local-diff-review.md) — Local review notes and explicit batch submission.
- [ADR 0063](0063-account-connectors-in-the-mcp-base.md) — The account's claude.ai connectors join the inherited MCP base.
- [ADR 0082](0082-resident-wsl-attachments.md) — Resident WSL execution with disposable attachments.
- [ADR 0084](0084-shared-windows-desktop.md) — Shared desktop interface over an injected WSL runtime.
- [ADR 0085](0085-portable-core.md) — Portable core with injected effects.
- [ADR 0088](0088-github-app.md) — GitHub App for issues, PRs and notifications through the Cloud.
- [ADR 0089](0089-notch-island.md) — Persistent notch island that follows agents, answers requests and reopens conversations.
- [ADR 0090](0090-approved-remote-input.md) — Remote input only from member and key pairs approved on the owner's Mac, with separate view, message and control rights.
- [ADR 0091](0091-keep-unreadable-state.md) — Unreadable board and team files are copied to a recovery folder before anything replaces them, and saves wait while copying fails.
- [ADR 0093](0093-anonymous-product-analytics.md) — Anonymous feature and click events sent to PostHog from Rust, identified only by i18n keys and static IDs.
- [ADR 0094](0094-leaving-a-share.md) — A viewer leaves a share for all of their devices; the relay keeps them out until the owner readmits them.
- [ADR 0095](0095-cursor-acp.md) — Cursor CLI through ACP and an explicit external account.
- [ADR 0096](0096-terminal-sessions.md) — Terminal sessions owned by the app, persisted as records on the board.
- [ADR 0097](0097-ask-before-ending-the-app.md) — Quitting, closing the main window or restarting for an update asks first while local work would be cut off.
