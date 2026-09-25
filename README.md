<p align="center">
  <img src="docs/icon.png" alt="Prometeu icon" width="128" height="128">
</p>

<h1 align="center">Prometeu</h1>

<p align="center">
  <a href="https://www.greptile.com/?utm_source=oss_badge&utm_medium=readme&utm_campaign=greptile_for_open_source">
    <img src="https://www.greptile.com/badge.svg" alt="Greptile: The War on Bugs">
  </a>
</p>

Prometeu is an open-source desktop app for macOS and Linux, built for working
with Claude Code, Codex, and Antigravity CLI.
Organize parallel tasks into workspaces, give each task its own Git worktree,
and keep conversations, files, terminals, and code review in one place.

The agents run on your computer through their installed CLIs. A Prometeu account is
optional for local work; model access comes from your own provider account.

[Download](https://github.com/prometeucorp/prometeu/releases/latest) ·
[Documentation](docs/README.md) ·
[Report an issue](https://github.com/prometeucorp/prometeu/issues) ·
[Changelog](CHANGELOG.md) ·
[MIT license](LICENSE)

## Features

- **Parallel workspaces.** Use isolated Git worktrees, work with multiple
  repositories, or open an ordinary folder without Git. Each workspace keeps
  its own conversations and terminals. Archiving or finishing offers to remove
  its worktree and local branch after explicit confirmation.
- **Claude Code, Codex, and Antigravity CLI.** Choose a model and supported effort
  level, manage accounts in Settings, and resume conversations after an agent
  process stops. Antigravity CLI uses the account already connected in `agy`;
  quotas are shown by model group. App-managed login, interactive approvals and
  plan mode are unavailable.
  See the [provider matrix](docs/quality/provider-matrix.md) for validation limits.
  Search models by name or agent, pin favorites, and select the effort directly.
  Catalogs refresh from each CLI; failed refreshes are visible and extra Codex
  models are available through “Show additional models”.

- **A desk for ongoing conversations.** Arrange conversations side by side,
  respond to questions, and follow agent activity without opening each workspace.
  Your workspace stage stays separate from the agent's status.
- **Local notifications.** Opt into completion, approval/input and error alerts
  in Settings. Choose a macOS banner, a notch overlay or sound alone.
  Notifications and sound start disabled; phone push is not included.
- **Optional request review.** Bring your own TypeSafe API key and turn the
  review on in Settings to add **Review request** to new workspaces. It looks
  for a missing decision, such as an undefined rule for existing records, and
  suggests at most two questions you can answer, hand to the agent or dismiss.
  It is off by default, sends your draft only when you ask, and never blocks
  creating the workspace. See [context evaluation](docs/contracts/context-evaluation.md).
- **Git review.** Read unified or side-by-side diffs, stage selected files,
  commit only the index, inspect history, compare branches, and resolve conflicts.
- **Files, terminals, and browser preview.** Edit code, read Markdown, view PDFs, CSVs and images,
  find text in the open file (⌘F), open any file by name (⌘P),
  create, rename and trash files from the side tree, restore deleted ones,
  run project scripts, and attach selected page elements or screenshots to a prompt.
- **Delegation through MCP.** The built-in `prometeu` MCP is available but off
  by default. Select it in the coordinator workspace’s tools to create agents
  in isolated workspaces, send messages and inspect their runs, background
  tasks, conversations and files. External local agents can register a scoped
  client and use the same MCP while Prometeu is open, without an internal
  conversation. Each client controls only agents it created, including their
  setup, Run scripts and preview. See [external registration](docs/contracts/embedded-mcp.md#external-client-registration).
- **Tools per workspace.** Select MCP servers, plugins, and skills. Reusable
  actions include an editable code review profile.
- **Optional collaboration.** Share live conversations and comment on them
  with your team, or continue from your own companion devices. Execution stays
  on the owner's computer.
- **Dictation.** Speak into the message box; the transcript appears as you
  talk, and the dictation language is a preference of its own.
- **Portuguese and English UI.** Agent output and your content keep their
  original language.

Support differs between providers. For example, initial plan mode is available
for Claude Code, and local plugin folders are the portable format for both
agents. See the [provider matrix](docs/quality/provider-matrix.md) for supported
features, limitations, and test coverage.

## Settings

Settings has five sections: **General** (language, notifications and updates),
**Agents** (new-workspace defaults and accounts), **Resources** (plugins, MCPs and
skills), **Actions**, and **Work and team** (organizations, projects and integrations).
Search settings by name; filter the resource library by type or text. Each resource's
menu keeps installation, authentication and catalog management together. Installing
an item does not enable it in existing workspaces. Account usage and detailed
notification preferences expand in place.

## Install

Each release contains separate downloads for **macOS on Apple Silicon** and
**Linux x86_64** under the same version.

### macOS

1. Download [Prometeu_aarch64.dmg](https://github.com/prometeucorp/prometeu/releases/latest/download/Prometeu_aarch64.dmg).
2. Open the disk image and move Prometeu to Applications.

### Linux

Download [Prometeu_x86_64.AppImage](https://github.com/prometeucorp/prometeu/releases/latest/download/Prometeu_x86_64.AppImage)
to a directory writable by your user, then run:

```sh
chmod +x Prometeu_x86_64.AppImage
./Prometeu_x86_64.AppImage
```

The AppImage is built on Ubuntu 22.04. See the [Linux guide](docs/operations/linux.md)
for runtime requirements, Arch and Debian source packages, ARM64 builds and
platform differences. macOS and AppImage installations offer in-app updates;
other Linux installations use their package manager or a rebuild.

### Connect an agent

Install Claude Code, Codex, or Antigravity CLI (minimum 1.2.7). The corresponding
`claude`, `codex`, or `agy` command must be available in your shell. Authenticate
through the CLI or Prometeu's account panel; Antigravity uses its external CLI login.

### Your first workspace

1. Add a local Git repository or folder.
2. Create a workspace, choose a model, and enter your task. Use a new worktree
   to keep Git changes separate from the original checkout.
3. Follow the conversation, open files and terminals, and review changes before
   committing. A folder without Git opens in place without a branch or worktree.

**Agent permissions:** by default, agents execute tools without per-tool
approval and have your local user's access to files and processes. A Git
worktree separates changes; it is not a sandbox. Use repositories and tools you
trust. See the [architecture](ARCHITECTURE.md) for execution boundaries.

## Develop locally

### Prerequisites

For the desktop app:

- macOS with Xcode Command Line Tools (`xcode-select --install`), or Linux
  with the WebKitGTK packages listed in [Linux](docs/operations/linux.md).
- Git.
- Node.js **24.14.0** and npm **11.9.0**, as declared in
  [package.json](package.json).
- Rust installed through rustup. The backend pins **1.88.0**, with rustfmt
  and Clippy, in [rust-toolchain.toml](src-tauri/rust-toolchain.toml).
- Claude Code, Codex, or Antigravity CLI installed and authenticated to exercise real sessions.

### Run the desktop app

```sh
git clone https://github.com/prometeucorp/prometeu.git
cd prometeu
npm ci
npm run app
```

`npm run app` starts the Tauri app through [scripts/app.sh](scripts/app.sh).
Development state defaults to `~/.prometeu-dev`, separate from the installed
app's `~/.prometeu`. When launched through a Prometeu workspace, the script
uses that workspace's name and port to isolate simultaneous development instances.

`npm run app:bundle` builds a debug `.app` and opens it through LaunchServices.
Use it to test dictation: macOS only honors the speech usage description of a
bundle the app launched itself, so `npm run app` hides the microphone button.

### Work on the UI in a browser

After cloning and installing npm dependencies:

```sh
npm run dev
```

Open the URL printed by Vite. This mode uses [src/mock.ts](src/mock.ts), so it
does not require Rust, an agent CLI, or a Prometeu account. It exercises the UI
with simulated data; real agent execution, filesystem operations, and native
macOS behavior require the desktop app. Use `PORT=1421 npm run dev` to select
another port.

### Validate changes

Install the test browsers once:

```sh
npx playwright install chromium webkit
```

Run the checks relevant to your change:

| Command | Coverage |
| --- | --- |
| `npm run docs:check` | Documentation links and index |
| `npm run typecheck` | TypeScript types |
| `npm run test:web` | Frontend and relay tests |
| `npm run test:rust` | Rust backend tests |
| `npm run test:e2e` | Browser flows against the mock |
| `npm run check` | Full CI validation, including build, architecture checks, Rust formatting, and Clippy |

Run `npm run check` before submitting a code PR. Playwright covers Chromium
and WebKit against the browser mock; it does not drive the native Tauri app.
Changes to native behavior also need a manual desktop check. See the
[development guide](docs/operations/development.md) for details.

## Add projects from your catalog

Register Git sources once in the Cloud's **Projects** tab, in your personal or
organization catalog. In the desktop, open **Settings / Work and team / Projects on this Mac** or **Add
project** in the sidebar. Select projects, choose a destination folder and click
**Add to this Mac**. Each successful clone appears in the project list. Failed
rows retain their errors and can be retried without repeating completed clones.

**Link existing folder** registers an existing clone after checking its Git
origin. **Add local folder** remains available without an account. Cloning uses
your Mac's Git authentication and does not run setup or install dependencies.
Deleting catalog definitions preserves local repositories. See the
[catalog contract](docs/contracts/cloud-catalog.md).

## Configure project scripts

Projects can define workspace lifecycle commands in `.prometeu/settings.toml`.
Existing `.conductor/settings.toml` files are also supported.

```toml
[scripts]
setup = "npm install"
run = "npm run dev -- --port $PROMETEU_PORT"
archive = "docker compose down"
```

These are examples; use commands appropriate to your project. Setup runs when
a worktree is created, before the first prompt reaches the agent. If setup
fails, the prompt still proceeds with a warning. Run starts from the workspace
UI, and archive runs before archiving.

For multiple run commands:

```toml
[scripts.run.web]
command = "npm run dev -- --port $PROMETEU_PORT"
default = true
```

Scripts receive `PROMETEU_WORKSPACE_PATH`, `PROMETEU_ROOT_PATH`,
`PROMETEU_WORKSPACE_NAME`, and `PROMETEU_PORT`. Each workspace reserves ten
ports starting at `PROMETEU_PORT`; `PORT` has the same starting value.
Equivalent `CONDUCTOR_` variables preserve compatibility with existing scripts.
Use these ports instead of a fixed port when running multiple workspaces.

If a worktree has no settings file, it inherits the original checkout's file.
This lets you keep private setup commands outside version control.

## Local data and optional Cloud features

Local work does not require a Prometeu account. App state and Codex/Antigravity conversation
transcripts live under `~/.prometeu`; Claude Code maintains its own transcripts.
See the [persistence contract](docs/contracts/persistence.md) for paths and
ownership.

A Prometeu Cloud account adds organizations, shared tool catalogs, live
collaboration, and companion access through the mobile web app. Sharing is
explicit. Remote conversations still execute only on the owner's Mac, which
must stay awake with Prometeu open.

Shared conversation content uses end-to-end encryption. The relay still sees
routing metadata, and member keys are trusted as the relay directory reports
them, including later key changes. There is no forward secrecy. This encryption does not cover content sent to model
providers or protect a compromised device. See the
[security design and limits](docs/decisions/0022-end-to-end-encryption.md).

This repository contains the desktop app, the mobile frontend, the shared
design system, and the Cloudflare relay. The Prometeu Cloud account service is
a separate project and is not required for local desktop development. See the
[Cloud account contract](docs/contracts/cloud-account.md) and
[organization contract](docs/contracts/cloud-organizations.md) for integration
details.

## Contribute

Bug reports, documentation improvements, and code contributions are welcome.
See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and
[contributor recipes](docs/operations/contributing.md) for common changes.

1. Search [existing issues](https://github.com/prometeucorp/prometeu/issues)
   before opening a report. Include reproduction steps, expected and actual
   behavior, macOS and Prometeu versions, and the agent CLI version when relevant.
2. Fork the repository and create a branch for your change.
3. Read [AGENTS.md](AGENTS.md) and [ARCHITECTURE.md](ARCHITECTURE.md), then the
   documents for the area you are changing. Keep the change focused and update
   affected documentation and tests.
4. Run the relevant checks and open a pull request against `main`. Explain
   the user-visible change, validation performed, and any remaining limitations.

Write code comments and doc comments in English. UI text belongs in the
Portuguese and English i18n catalogs. Commits follow **Conventional Commits in
English**, with a lowercase description and no trailing period:

```text
docs(readme): improve the instructions for new contributors
```

The description of a `feat`, `fix`, or `perf` commit becomes a changelog line,
so write what the person using the app sees. See the
[commit and release guide](docs/operations/release.md) for examples and rules.
The local commit hook and CI validate commit messages.

GitHub issues are public; remove credentials and private conversation content
from reports and attachments. The in-app **Feedback** panel opens a new public
GitHub issue without copying its draft or capture. Its private channel requires
a Prometeu account and delivers reports privately to the Prometeu team. See the
[feedback contract](docs/contracts/feedback.md).

## Documentation

- [Documentation index](docs/README.md): contracts, decisions, and operations.
- [Architecture](ARCHITECTURE.md): system boundaries and code ownership.
- [Development guide](docs/operations/development.md): environment and testing.
- [Provider matrix](docs/quality/provider-matrix.md): Claude Code, Codex, and Antigravity CLI support.
- [Design system](docs/architecture/design-system.md): shared UI components.
- [Architecture decisions](docs/decisions/README.md): accepted decisions and history.
- [Release guide](docs/operations/release.md): maintainer release workflow.

## License

Prometeu is licensed under the [MIT License](LICENSE).
