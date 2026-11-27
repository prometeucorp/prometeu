# Contracts

These documents specify the formats and the guarantees that go across the
limits between processes or layers.

- [Agent accounts](accounts.md) — Accounts, login, the global selection and isolated quota limits for each agent.
- [Reusable actions and agents](actions.md) — Commands and profiles that a person can use again, the execution of tasks and the PR discovery.
- [Agent runtime contract](agent-runtime.md) — Agent discovery, capabilities and the port that starts and controls agents.
- [Portable application core](application-core.md) — The ports of the core, their desktop adapters and the obligations of each implementation.
- [Browser and visual context](browser.md) — The preview, the inspection, the captures and how they go into the draft.
- [Optional Prometeu account](cloud-account.md) — The SaaS, the authentication in the browser, the desktop connection and the private storage.
- [Personal catalog and local items](cloud-catalog.md) — Plugins, MCP servers and skills in the SaaS, with private items and explicit sharing on the desktop.
- [Organizations in Prometeu Cloud](cloud-organizations.md) — Operations on organizations, invitations, catalogs and access to the relay.
- [Optional context evaluation](context-evaluation.md) — The TypeSafe adapter with the key of the person, the evaluation port and the key test in Settings.
- [Conversation Events v1](conversation-events-v1.md) — The canonical protocol that Prometeu owns.
- [Cursor CLI](cursor.md) — The ACP transport, the external account, the conversation lifecycle and the limits of the capabilities.
- [Desktop presentation](desktop-presentation.md) — Typed snapshots, callbacks, focus and the lifetime of resource views.
- [Local diff review notes](diff-review.md) — Notes on a diff, their position, how the app keeps them and how a person sends them as a batch.
- [Embedded Prometeu MCP and delegation](embedded-mcp.md) — Local orchestration that a person must enable, with agents that the client owns and optional conversation context.
- [Public issues and private feedback](feedback.md) — Public GitHub issues or private reports with text and images through an account.
- [Workspace Git](git.md) — The index, the worktree, the review and the Git operations on the machine of the owner.
- [GitHub issues and pull requests](github-issues.md) — The GitHub App device flow, the issues that a person can claim, PRs, review requests, PR status and isolated workspaces.
- [GitHub notifications](github-notifications.md) — GitHub App webhooks that become metadata notifications in the Cloud and in the Notifications destination of the desktop.
- [TypeScript ↔ Rust IPC contract](ipc.md) — Typed desktop commands, events, browser mocks and the compatibility of the serialization.
- [Linear issue discovery and assignment](linear-issues.md) — The issues of a team, the OAuth write scope, the claim flow and the compatibility of the cache.
- [Local notifications](notifications.md) — The selection of events, the native delivery and the silent default values.
- [Local persistence](persistence.md) — The local formats of the board, the transcripts and the accounts, their migrations and their recovery.
- [Plugin hub contract](plugin-marketplace.md) — The hub, the portable package and the adaptation for each provider.
- [Product analytics](product-analytics.md) — The PostHog event words, the payload, the installation identity and the rules for new events.
- [Standalone to-do list](todos.md) — The to-do list in Tasks, the computer that keeps it, its file, the IPC commands and the MCP tools of the desktop and of the host.
- [Relay protocol v4](relay-v4.md) — Encrypted collaboration and trust on first contact (TOFU).
- [Remote host protocol](remote-host.md) — The installation, the SSH connection, and the NDJSON methods and events of `prometeu-host`.
- [Workspace supervision](supervision.md) — Host events, supervisor bindings and generic signed webhooks through the Cloud.
- [Desktop release and update contract](releases.md) — Desktop packages, the updater manifest and which installer owns an installation.
- [Local telemetry and insights](telemetry.md) — Local events without content, usage measurements, the SQLite storage and the insight queries.
- [Native Claude decisions](native-claude-decisions.md) — Explicit hook responses for one generation, together with the native permission dialog.
- [Native Antigravity store](native-antigravity-store.md) — The recovery of the initial trajectory in its scope, a stable tool identity and full result files.
- [Native Antigravity decision boundary](native-antigravity-decisions.md) — The hook, ACP and Remote Control paths that we examined, and the local TUI connection API that is not available.
- [Native Codex engine](native-codex-engine.md) — The private app-server, the accurate thread settings and the native terminal connection.
- [Native terminal state helper](terminal-state.md) — Emulation in sequence, screen snapshots for one generation and which part answers terminal queries.
