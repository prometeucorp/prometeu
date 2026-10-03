# ADR 0057 — Start a conversation from a skill, with a project-declared artifact path

Date: 2026-09-24

Status: Accepted

## Context

Every conversation started from an empty box. A method the person already
trusts — a discovery interview, a spec-driven flow — had to be retyped as a
prompt each time, and whatever it produced lived only in the transcript. The
hub already installs such methods: standalone skills (`skill_hub`) and plugins
that ship `skills/*/SKILL.md` with supporting templates and references. A real
framework, such as an internal spec-driven kit, only travels as a plugin, so a
catalog of standalone skills alone would leave it out.

Two choices have trade-offs: how a conversation starts under a skill without
changing the layered tool selection of
[ADR 0045](0045-layered-tool-selection.md), and where the method's artifacts
land so everyone on a project puts them in the same place.

## Options considered

1. Tell the person to select the skill in the workspace picker and write the
   instruction in the prompt. Nothing new, but the selection persists in the
   workspace layer and the path stays a personal habit.
2. Write the chosen skill into the workspace layer at creation. The method
   would then load in every later conversation of the workspace, which is a
   selection decision the person did not make.
3. Add the chosen skill's package to the resolved set of the conversation that
   starts from it, and open its first message with an app-written line. Read
   the artifact path from the versioned `.prometeu/settings.toml`.

For the path, an app-local preference was also considered. It would differ per
person and invent a default for repositories that never chose one.

## Decision

Adopt option 3.

- **Catalog.** The launcher's *Start with* lists standalone hub skills and the
  skills installed plugins ship. The backend reads `name` and `description`
  from the frontmatter of each local-folder package's `skills/*/SKILL.md`
  (`plugin_skills`); `.zip` and URL sources are handed to the CLI unread and
  stay out of the catalog. The folder name stands in for a missing `name`.
- **Choice.** `Draft.kickoff` carries `<package>/<skill>`; empty means none.
  The backend validates it against the installed catalog before publishing a
  card (`err.kickoff.missing`) and requires `workspacePluginSelection`, since a
  skill rides the plugin-package pipeline (`err.kickoff.unsupported`).
- **Ephemeral availability.** At spawn the package joins the resolved
  `SessionLaunch` of that conversation only: a standalone `skill-<id>` on the
  skills axis, a plugin on the plugins axis. No global, project or workspace
  layer changes. `Tab.kickoff` records the choice so a resumed process adds the
  package again; the logical session keeps its method. On resume the skill is
  validated with the same catalog rule as creation. When a plugin update or an
  uninstall removed it, the resume still happens — the transcript is the
  session and resuming is never blocked — but the package is not added and the
  conversation receives a `system.notice` warning (code `kickoff.missing`),
  rendered in the display language, saying it continues without the skill.
- **Opening line.** The first message starts with one line naming the skill
  (and its plugin) and, when declared, the artifact path, followed by the
  attachments and the person's prompt. It is agent-facing text rendered by the
  backend in the display language, like other agent-injected notices; the
  transcript keeps it verbatim.
- **Artifact path.** `[method] artifacts = "docs/specs"` in the repository's
  settings file, read by `scripts.rs` with the same clone inheritance and
  primary-repository rule as `[tools]`. The path must be relative and stay in
  the repository; otherwise, or when absent, no path is named. In a
  multi-repository workspace it is prefixed with the primary repository's
  folder, because the agent runs in their common parent.
- **Default preference.** The last explicit *Start with* choice, including
  none, is remembered in webview localStorage (`prometeu:kickoff`), like the
  launcher's worktree switch and the issues feature's team. It is a
  per-installation convenience, not board state, and it falls back to none when
  the skill is no longer installed. The launcher keeps the person's choice
  apart from the draft: a provider without `workspacePluginSelection` only
  suppresses it, and switching back to Claude or Codex restores it.

The mechanism is a prompt plus the existing skills and plugins axes, so Claude
and Codex behave the same. Antigravity has no hub tool selection and does not
offer the choice.

Out of scope: a phase machine, gates or progress tracking; starting a
workspace from an existing artifact; an artifact index in the app; artifacts in
the Cloud.

## Consequences

Positive:

- a trusted method starts in one choice, including plugin-shipped frameworks
  with supporting files;
- the team shares the artifact location through a versioned file, and a
  repository that declares nothing gets no invented path;
- tool layers and their trust stay untouched, so the choice never leaks into
  later conversations.

Negative:

- the method is not re-announced to new tabs of the same workspace; that is a
  deliberate limit of a starting point;
- the frontmatter reader covers the scalar forms skills use, not full YAML; it
  does drop trailing `# comments` from plain scalars and ignores text after a
  closing quote;
- `Tab.kickoff` is a new optional board field, and `[method]` a new
  repository-facing key to keep stable.

## Evidence

- `kickoff.rs`: frontmatter forms and comments, discovery that skips
  standalone, remote and missing packages, catalog validation, axis-aware
  `ensure`, resume validation with its notice and the localized opening line.
- `scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented`.
- `session.rs::drafts_without_a_kickoff_still_deserialize`,
  `session.rs::first_message_opens_with_the_kickoff_line` and
  `session.rs::artifact_path_follows_the_primary_repository`.
- `crates/core/src/board.rs::tabs_without_a_kickoff_keep_the_previous_format`.
- `src/kickoff.test.ts`: catalog merge, the stored preference and the choice
  that survives provider switches.
- Contracts: [agent runtime](../contracts/agent-runtime.md#skill-kickoff),
  [plugin hub](../contracts/plugin-marketplace.md#skills-inside-packages),
  [IPC](../contracts/ipc.md#starting-from-a-skill) and
  [persistence](../contracts/persistence.md#skill-kickoff);
  [provider matrix](../quality/provider-matrix.md).
