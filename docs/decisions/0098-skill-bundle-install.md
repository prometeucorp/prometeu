---
type: Decision
title: "ADR 0098 — Install skill-bundle repositories as portable packages"
description: "Prometeu makes Claude and Codex manifests for Git sources that contain only skills. Thus Install operates without an upstream plugin.json."
status: stable
---

# ADR 0098 — Install skill-bundle repositories as portable packages

Date: 2026-11-07

## Context

More and more authors publish agent skills as a Git repository with
`skills/*/SKILL.md`. For Claude Code, the repository also has a marketplace
entry that lists those skills with `strict: false` and no
`.claude-plugin/plugin.json`. The installer of Prometeu accepted only folders
that had a portable plugin manifest ([ADR 0005](0005-portable-plugin-marketplace.md)).
Thus, when a person pasted such a URL, the installation stopped with the
error "no portable plugin".

The clone folders also used only the base name of the repository. Thus two
GitHub repositories with the name `skills` cannot be on the same computer.

Standalone skills and skills that come in a plugin go through the same
pipeline. Claude uses `--plugin-dir`, and Codex uses a derived marketplace. The
missing function was the import, not the execution.

## Options considered

1. Tell authors to publish again with `plugin.json` before Prometeu can
   install their repository.
2. Import the body of each `SKILL.md` into the standalone skill hub. Do not
   keep Git updates and supporting assets.
3. At installation, make portable packages that Prometeu owns from skill-bundle
   marketplaces and from bare `skills/*/SKILL.md` trees. Clone GitHub URLs into
   `owner-repo` folders.

## Decision

Use option 3.

- Discovery first examines the current root and marketplace `plugin.json`
  paths. Then it accepts marketplace entries with a `skills` array that points
  to folders with `SKILL.md`. As a last step, it accepts each
  `skills/*/SKILL.md` in the clone.
- For those units, Prometeu writes `<clone>/.prometeu/packages/<id>/`. This
  folder contains `.claude-plugin/plugin.json`, `.codex-plugin/plugin.json`
  (`skills: "./skills/"`) and a copy of each skill tree. The hub `source` points
  to that package. `from` keeps the Git URL. `made` is true.
- GitHub clone folders use `owner-repo`. Thus `caldoai/skills` and
  `typesafe-ai/skills` do not use the same folder. URLs that are not from GitHub
  keep the base name.
- A plugin update runs discovery again on the clone root. Thus the packages
  that Prometeu made change after `git pull`.
- The selection stays on the plugins axis. *Start with* continues to list
  `skills/*/SKILL.md` in installed packages. Prometeu does not add hub
  plugins and skills to Cursor.

## Consequences

- When a person pastes the URL of a repository that contains only skills, the
  same dialog as for plugins installs it. An upstream `plugin.json` is not
  necessary.
- The supporting files adjacent to `SKILL.md` stay in the copy in the package that
  Prometeu makes.
- Current clones with bare repository names continue to operate. Only new
  installations use `owner-repo`.
- Empty repositories continue to cause an error. The error text tells about
  skills and also about manifests.

## Evidence

- `crates/tools/src/plugins.rs` discovery, synthesis and `repo_name` tests
- Desktop `plugins.rs::clone_lists_discovered_plugins` skill-bundle case
- [plugin hub contract](../contracts/plugin-marketplace.md)
- [provider matrix](../quality/provider-matrix.md)
