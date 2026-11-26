---
type: Decision
title: "ADR 0057 — Start a conversation from a skill, with a project-declared artifact path"
description: "Start a conversation from a skill, with an artifact path that the project declares."
status: stable
---

# ADR 0057 — Start a conversation from a skill, with a project-declared artifact path

Date: 2026-09-24


## Context

Each conversation started from an empty box. A person can trust a method, for
example a discovery interview or a flow that starts from a specification. But
the person had to type that method again as a prompt each time. The results of
the method were only in the transcript. The hub installs such methods at this
time. These methods are standalone skills (`skill_hub`) and plugins that
contain `skills/*/SKILL.md` with templates and reference files. A full framework,
for example an internal kit for specifications, is available only as a plugin.
Thus a catalog of only standalone skills does not include it.

Two decisions have trade-offs:

- How a conversation starts with a skill, without a change to the layered tool
  selection of [ADR 0045](0045-layered-tool-selection.md).
- Where the artifacts of the method go, so that all persons on a project put
  them in the same folder.

## Options considered

1. Tell the person to select the skill in the workspace picker and write the
   instruction in the prompt. This adds no new function. But the selection stays in
   the workspace layer, and each person selects the path differently.
2. Write the selected skill into the workspace layer when Prometeu makes the
   workspace. Then the method loads in each subsequent conversation of the
   workspace. The person did not make this selection decision.
3. Add the package of the selected skill to the resolved set of the
   conversation that starts from it. Start its first message with a line that
   the app writes. Read the artifact path from the versioned
   `.prometeu/settings.toml`.

For the path, Prometeu also examined a local preference in the app. That
preference is different for each person. It also adds a default for
repositories that did not select a path.

## Decision

Prometeu selected option 3.

- **Catalog.** The *Start with* list of the launcher shows standalone hub
  skills and the skills in installed plugins. The backend reads `name` and
  `description` from the frontmatter of `skills/*/SKILL.md` in each package
  from a local folder (`plugin_skills`). Prometeu gives `.zip` and URL sources
  to the CLI and does not read them. These sources are not in the catalog. If
  `name` is missing, Prometeu uses the folder name.
- **Selection.** `Draft.kickoff` contains `<package>/<skill>`. An empty value
  means no skill. Before it publishes a card, the backend validates the value
  against the installed catalog (`err.kickoff.missing`). The provider must also have
  `workspacePluginSelection`, because a skill goes through the plugin package
  pipeline (`err.kickoff.unsupported`).
- **Temporary availability.** When Prometeu starts the agent, the package goes
  into the resolved `SessionLaunch` of that conversation only. A standalone
  skill goes on the skills axis as `skill-<id>`. A plugin goes on the plugins
  axis. No global, project or workspace layer changes. `Tab.kickoff` records
  the selection. Thus a resumed process adds the package again, and the logical
  session keeps its method.
- **Resume.** When Prometeu resumes the process, it validates the skill with the
  same catalog rule. A plugin update or an uninstall can remove the skill. In
  that condition, the resume continues, because the transcript is the session
  and Prometeu does not prevent a resume. But Prometeu does not add the package.
  The conversation receives a `system.notice` warning (code `kickoff.missing`)
  in the display language. The warning tells that the conversation continues
  without the skill.
- **First line.** The first message starts with one line that names the skill
  and its plugin. If the project declares an artifact path, the line also gives
  that path. The attachments and the prompt of the person follow this line. The
  backend writes this text for the agent in the display language, as it does
  for other notices to the agent. The transcript keeps the line without
  changes.
- **Artifact path.** The settings file of the repository declares the path, for
  example `[method] artifacts = "docs/specs"`. `scripts.rs` reads it with the
  same clone inheritance and the same primary-repository rule as `[tools]`. The
  path must be relative, and it must stay in the repository. If not, or if the
  value is missing, the line names no path. In a workspace with many
  repositories, Prometeu adds the folder of the primary repository before the
  path. The cause is that the agent operates in the parent folder of these repositories.
- **Default preference.** Prometeu keeps the last explicit *Start with*
  selection in the localStorage of the webview (`prometeu:kickoff`). This
  includes the "none" selection. The worktree control of the launcher and the
  team of the issues feature use the same method. This preference applies only
  to one installation. It is not board state. If the skill is not installed at
  that time, the preference changes to none. The launcher keeps the selection
  of the person apart from the draft. A provider without
  `workspacePluginSelection` only hides the selection. When the person selects
  Claude or Codex again, the selection comes back.

The mechanism is a prompt together with the current skills and plugins axes.
Thus Claude and Codex operate in the same manner. Antigravity has no hub tool
selection, and it does not show the selection.

These items are out of scope:

- a phase machine, gates or records of how the work continues
- a workspace that starts from a current artifact
- an artifact index in the app
- artifacts in the Cloud

## Consequences

Positive:

- A trusted method starts with one selection. This includes frameworks in
  plugins that have more files.
- The team shares the artifact location through a versioned file. A repository
  that declares no path does not get a path that Prometeu makes.
- The tool layers and their trust do not change. Thus the selection does not go
  into subsequent conversations.

Negative:

- New tabs of the same workspace do not get the method line. This limit is
  intentional, because the skill is only a start point.
- The frontmatter reader reads the scalar forms that skills use. It does not
  read full YAML. It removes `# comments` at the end of plain scalars. It
  ignores text after the last quotation mark.
- `Tab.kickoff` is a new optional board field. `[method]` is a new key in the
  repository settings, and it must stay stable.

## Evidence

- `kickoff.rs`: frontmatter forms and comments, and discovery that ignores
  standalone, remote and missing packages. It also examines catalog validation,
  the `ensure` function for each axis, resume validation with its notice and the
  translated first line.
- `scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented`.
- `session.rs::drafts_without_a_kickoff_still_deserialize`,
  `session.rs::first_message_opens_with_the_kickoff_line` and
  `session.rs::artifact_path_follows_the_primary_repository`.
- `crates/core/src/board.rs::tabs_without_a_kickoff_keep_the_previous_format`.
- `src/kickoff.test.ts`: the catalog merge, the stored preference and the
  selection that stays when the person changes the provider.
- Contracts: [agent runtime](../contracts/agent-runtime.md#skill-kickoff),
  [plugin hub](../contracts/plugin-marketplace.md#skills-inside-packages),
  [IPC](../contracts/ipc.md#starting-from-a-skill) and
  [persistence](../contracts/persistence.md#skill-kickoff).
  [Provider matrix](../quality/provider-matrix.md).
