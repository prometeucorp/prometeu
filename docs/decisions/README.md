# Architecture decisions

Stable ADRs record accepted decisions that this checkout implements at this
time. Each ADR contains the current selection, its reasons, its consequences
and its evidence. [ADR 0044](0044-current-documentation.md) specifies the
lifecycle.

## Maintenance

- Update the related ADR and contract together with the implementation.
- When a new decision replaces a decision, move the guarantees that continue to
  apply into the new decision. Update the links to the old document and delete
  it.
- Keep the compatibility requirements that the code implements at this time.
  When you remove a document, this does not let you delete data or
  compatibility code.
- Keep ADR numbers stable. Do not use a retired number again, and do not close
  gaps in the numbers.
- Use Git for previous versions and retired records: `git log --all --
  docs/decisions/` and `git show <commit>:<path>`.
- Add each decision to this index. The bundle map links to this index. Run
  `npm run docs:check`.

## States

- **stable:** accepted guidance that the code implements and that applies at
  this time. This state does not tell about the deployment, the completion of a
  feature, or a verification that the evidence does not show.
- **draft:** proposed guidance that the team discusses. List it separately from
  the current decisions. Do not use it as implemented behavior.
- **deprecated:** legacy guidance that Prometeu continues to support. It has an
  explicit replacement and a compatibility scope. Delete retired decisions. Do
  not keep them in this state.

Superseded and rejected records are in the Git history, not in the current
navigation. This index tells about the code of the repository. The deployment
and the validation of external services must have their own evidence.

## Template

Each concept in the bundle starts with four single-line YAML string fields.
The type agrees with its area: `Architecture`, `Contract`, `Decision`,
`Operations` or `Quality`. Use one of these string forms:

- Plain strings without YAML punctuation.
- Single-quoted strings. Write an apostrophe in the text two times.
- Double-quoted strings that agree with JSON, as in the example.

Text that starts with a digit, for example `2026 roadmap`, is permitted. Put
quotes around titles that contain only numbers. Pages that persons write do
not need timestamps for generation, verification or expiry. Keep ADR dates in
the body. Use only the YAML field for the lifecycle status. Folder `README.md`
indexes do not need frontmatter.

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

Add a title and a description of one line to this index. List drafts under a
separate Proposals heading. Move them to Current decisions only after the team
implements them. Run `npm run docs:check` to validate the metadata, the local
links and the index coverage. Index entries must be rendered links. Links in
code samples do not count.

## Current decisions

- [ADR 0001](0001-repository-knowledge.md) — A versioned knowledge bundle with short navigation and shared standing instructions.
- [ADR 0002](0002-canonical-conversation-protocol.md) — Canonical conversation protocol.
- [ADR 0003](0003-agent-capabilities.md) — Features that agent capabilities control.
- [ADR 0004](0004-prometeu-independent-identity.md) — The independent identity of Prometeu.
- [ADR 0005](0005-portable-plugin-marketplace.md) — One portable marketplace for Claude and Codex.
- [ADR 0007](0007-persistent-session-comments.md) — Persistent comments adjacent to the session.
- [ADR 0008](0008-explicit-git-index.md) — An explicit Git index and operations for each repository.
- [ADR 0009](0009-reusable-actions.md) — Commands that persons can use again, and agents that are responsible for tasks.
- [ADR 0010](0010-default-code-review.md) — Code review in the actions registry.
- [ADR 0012](0012-provider-accounts.md) — Local accounts and a global selection for each provider.
- [ADR 0013](0013-remove-provider-accounts.md) — Account removal and an empty selection.
- [ADR 0015](0015-cloud-rails.md) — An optional account in a separate Rails service.
- [ADR 0017](0017-executable-design-system.md) — Executable Design System components.
- [ADR 0018](0018-native-file-promises.md) — Prometeu receives macOS file promises.
- [ADR 0020](0020-personal-catalog-and-local-items.md) — The catalog in the SaaS and explicit sharing on the desktop.
- [ADR 0021](0021-cloud-organizations.md) — Organizations and invitations in the Cloud.
- [ADR 0022](0022-end-to-end-encryption.md) — End-to-end encryption in collaboration.
- [ADR 0023](0023-ordered-publication.md) — Put the snapshot publication and the conversation delivery in sequence.
- [ADR 0024](0024-typed-ipc.md) — IPC argument and result types that each command owns.
- [ADR 0026](0026-portable-collaboration-core.md) — A portable collaboration core and phone access through the relay.
- [ADR 0027](0027-companion-devices.md) — Companion devices in the relay.
- [ADR 0028](0028-mobile-web-app.md) — Prometeu on the phone as a web app that the Cloud serves.
- [ADR 0030](0030-remote-control.md) — Remote control that does not depend on sharing.
- [ADR 0034](0034-mobile-pairing-continuity.md) — Mobile pairing continuity.
- [ADR 0035](0035-feedback-requires-account.md) — Authenticated private feedback through the Cloud.
- [ADR 0036](0036-second-mac-as-companion.md) — A second Mac as a companion device.
- [ADR 0037](0037-browser-design-context.md) — A browser with visual context for the conversation.
- [ADR 0038](0038-browser-context-chips.md) — Selected elements as tags in the conversation.
- [ADR 0039](0039-organization-catalog-on-desktop.md) — Organization catalogs are available on the desktop.
- [ADR 0040](0040-open-source.md) — Open source in a single public repository.
- [ADR 0041](0041-members-list-shows-people.md) — The members list shows persons, not devices.
- [ADR 0042](0042-automatic-key-rotation.md) — Prometeu adopts the new key of a peer automatically.
- [ADR 0043](0043-retire-unused-ipc.md) — Retire desktop IPC commands that Prometeu does not use.
- [ADR 0044](0044-current-documentation.md) — Current documentation, with the history in Git.
- [ADR 0045](0045-layered-tool-selection.md) — Selection of MCP, plugins and skills in layers: global, project and workspace.
- [ADR 0046](0046-cli-inherited-mcp-base.md) — The CLI MCP set as a visible inherited base in the picker.
- [ADR 0047](0047-tool-selection-boundaries.md) — Reviewed tool declarations, the effective provider and native IPC patches.
- [ADR 0048](0048-scoped-cleanup-offer.md) — A scoped worktree cleanup after an archive or a finish.
- [ADR 0049](0049-embedded-delegation-mcp.md) — Native MCP for delegation that the client owns.
- [ADR 0050](0050-tested-application-boundaries.md) — Tested application boundaries before the team extracts services.
- [ADR 0051](0051-git-project-catalog.md) — Git projects in portable catalogs.
- [ADR 0052](0052-antigravity-runtime.md) — Replace Gemini CLI with Antigravity.
- [ADR 0053](0053-live-model-selection.md) — Live model catalogs and an explicit selection identity.
- [ADR 0054](0054-local-notifications.md) — Opt-in local notifications with an independent sound.
- [ADR 0055](0055-linux-desktop.md) — The Linux desktop through system programs.
- [ADR 0056](0056-background-tasks-hold-completion.md) — A turn completes only when its background tasks drain.
- [ADR 0057](0057-skill-kickoff-and-artifact-path.md) — Start a conversation from a skill, with an artifact path that the project declares.
- [ADR 0058](0058-optional-context-evaluation.md) — Optional context evaluation behind a port that the application owns.
- [ADR 0059](0059-local-telemetry-foundation.md) — Local telemetry with common columns and typed JSON payloads in SQLite.
- [ADR 0060](0060-isolated-desktop-presentation.md) — An isolated Desktop presentation with a view that production and the gallery share.
- [ADR 0061](0061-background-energy-policy.md) — An explicit inhibition, only for the system, while a macOS agent operates.
- [ADR 0062](0062-local-diff-review.md) — Local review notes and an explicit batch submission.
- [ADR 0063](0063-account-connectors-in-the-mcp-base.md) — The claude.ai connectors of the account go into the inherited MCP base.
- [ADR 0085](0085-portable-core.md) — A portable core with injected effects.
- [ADR 0088](0088-github-app.md) — A GitHub App for issues, PRs and notifications through the Cloud.
- [ADR 0089](0089-notch-island.md) — A persistent notch island that follows agents, answers requests and opens conversations again.
- [ADR 0090](0090-approved-remote-input.md) — Remote input comes only from member and key pairs that the Mac of the owner approved. View, message and control rights are separate.
- [ADR 0091](0091-keep-unreadable-state.md) — Prometeu copies unreadable board and team files to a recovery folder before it replaces them. Saves stop while Prometeu cannot make the copy.
- [ADR 0093](0093-anonymous-product-analytics.md) — Rust sends anonymous feature and click events to PostHog. Only i18n keys and static IDs identify them.
- [ADR 0094](0094-leaving-a-share.md) — A viewer goes out of a share for all of the devices of that viewer. The relay keeps the viewer out until the owner lets the viewer in again.
- [ADR 0095](0095-cursor-acp.md) — Cursor CLI through ACP with an explicit external account.
- [ADR 0096](0096-terminal-sessions.md) — The app owns terminal sessions and keeps them as records on the board.
- [ADR 0097](0097-ask-before-ending-the-app.md) — If local work will stop, the app shows a question before a Quit, a close of the main window or an update.
- [ADR 0098](0098-skill-bundle-install.md) — Install skill-bundle repositories as portable packages.
- [ADR 0099 — One level of sub-workspaces](0099-sub-workspaces.md): delegated workspaces in a flat group, and one conversation for each delegated workspace.
- [ADR 0100](0100-conversation-transport-identity.md) — Keep the conversation transport separately from the provider and the model. macOS defaults apply only to new conversations.
- [ADR 0101](0101-remote-machines.md) — Workspaces on remote machines through SSH: a resident host owns terminal agents, and the desktop keeps the board and the interface.
- [ADR 0102: Remove workspace stages](0102-remove-workspace-stages.md)
- [ADR 0103](0103-agents-on-remote-machines.md) — Agents on a remote machine control workspaces through its host. The host owns a registry, launches Claude without a desktop and has a local MCP interface that a token authorizes. This interface does not depend on the deployment infrastructure.
- [ADR 0104](0104-several-computers-on-one-machine.md) — Several computers follow the same remote machine through SSH at the same time. Each device has one attachment, hooks and permission requests are shared, and the computer that types sets the terminal dimensions.
- [ADR 0105](0105-codex-on-remote-machines.md) — Codex on remote machines: the native engine and the TUI run on the machine. The desktop reads the RPC of the engine through a relay terminal on the host.
- [ADR 0106](0106-workspace-supervision.md) — Workspace supervision through durable Cloud events and generic signed webhooks. The first executors are remote hosts.
- [ADR 0107](0107-simplified-technical-english.md) — All technical documentation uses ASD-STE100 Simplified Technical English, Issue 9.
