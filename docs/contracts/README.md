# Contracts

Formats and guarantees that cross process or layer boundaries.

- [Agent accounts](accounts.md) — Accounts, login, global selection and quota isolation between agents.
- [Reusable actions and agents](actions.md) — Reusable commands, profiles, execution and waiting for external events.
- [Agent runtime contract](agent-runtime.md) — Discovery, capabilities and the agent execution port.
- [Portable application core](application-core.md) — Core ports, their desktop and WSL adapters and implementation obligations.
- [Browser and visual context](browser.md) — Preview, inspection, captures and sending to the draft.
- [Optional Prometeu account](cloud-account.md) — SaaS, browser-based authentication, desktop connection and private persistence.
- [Personal catalog and local items](cloud-catalog.md) — Plugins, MCP and skills managed in the SaaS, with private items and explicit sharing on the desktop.
- [Organizations in Prometeu Cloud](cloud-organizations.md) — CRUD, invitations, catalogs and relay access.
- [Optional context evaluation](context-evaluation.md) — Bring-your-own-key TypeSafe adapter, evaluation port and key test in Settings.
- [Conversation Events v1](conversation-events-v1.md) — Canonical protocol owned by Prometeu.
- [Cursor CLI](cursor.md) — ACP transport, external account, conversation lifecycle and capability limits.
- [Desktop presentation](desktop-presentation.md) — Typed snapshots, callbacks, focus and resource view lifetime.
- [Local diff review notes](diff-review.md) — Annotations, placement, persistence and explicit batch submission.
- [Embedded Prometeu MCP and delegation](embedded-mcp.md) — Local, opt-in orchestration infrastructure with client-owned agents and optional conversation context.
- [Public issues and private feedback](feedback.md) — Public GitHub issues or account-backed private reports with text and images.
- [Workspace Git](git.md) — Index, worktree, review and Git operations on the owner's machine.
- [GitHub issues and pull requests](github-issues.md) — GitHub App device flow, assigned and available issues with claim, authored PRs, requested reviews, PR status and isolated workspaces.
- [GitHub notifications](github-notifications.md) — GitHub App webhooks turned into metadata-only notifications in the Cloud and the desktop Notifications destination.
- [TypeScript ↔ Rust IPC contract](ipc.md) — Typed desktop commands, events, browser mocks and serialization compatibility.
- [Linear issue discovery and assignment](linear-issues.md) — Available team issues, OAuth write scope, claim flow and cache compatibility.
- [Local notifications](notifications.md) — Event selection, native delivery and silent defaults.
- [Local persistence](persistence.md) — Local board, transcript and account formats, migrations and recovery.
- [Plugin hub contract](plugin-marketplace.md) — Hub, portable package and per-provider adaptation.
- [Product analytics](product-analytics.md) — PostHog event vocabulary, payload, installation identity and rules for new events.
- [Relay protocol v4](relay-v4.md) — Encrypted collaboration and TOFU.
- [Desktop release and update contract](releases.md) — Desktop packages, updater manifest and installation ownership.
- [Local telemetry and insights](telemetry.md) — Content-free local events, usage measurements, SQLite storage and insight queries.
- [Native Windows application](windows-application.md) — Shared interface over WSL, command coverage, deadlines and paths.
- [WSL runtime protocol](wsl-runtime.md) — Framing, capabilities, resident attachment, storage and compatibility.
