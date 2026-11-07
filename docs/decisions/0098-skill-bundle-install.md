---
type: Decision
title: "ADR 0098 — Install skill-bundle repositories as portable packages"
description: "Synthesize Claude and Codex manifests for skill-only Git sources so Install works without upstream plugin.json."
status: stable
---

# ADR 0098 — Install skill-bundle repositories as portable packages

Date: 2026-11-07

## Context

Authors increasingly publish agent skills as a Git repository with
`skills/*/SKILL.md` and, for Claude Code, a marketplace entry that lists those
skills with `strict: false` and no `.claude-plugin/plugin.json`. Prometeu's
installer only accepted folders that already carried a portable plugin
manifest ([ADR 0005](0005-portable-plugin-marketplace.md)), so pasting such a
URL failed with "no portable plugin". Clone directories also used the repository
basename alone, so two GitHub repos named `skills` could not coexist.

Standalone skills and plugin-shipped skills already run through the same
Claude `--plugin-dir` and Codex derived-marketplace pipeline. The gap was
import, not execution.

## Options considered

1. Ask authors to republish with `plugin.json` before Prometeu can install.
2. Import each `SKILL.md` body into the standalone skill hub and drop Git
   updates and supporting assets.
3. On install, synthesize Prometeu-owned portable packages from skill-bundle
   marketplaces and bare `skills/*/SKILL.md` trees, and clone GitHub URLs into
   `owner-repo` folders.

## Decision

Adopt option 3.

- Discovery, after the existing root and marketplace `plugin.json` paths,
  accepts marketplace entries whose `skills` array points at directories that
  contain `SKILL.md`, and as a last resort each `skills/*/SKILL.md` under the
  clone.
- For those units, Prometeu writes
  `<clone>/.prometeu/packages/<id>/` with `.claude-plugin/plugin.json`,
  `.codex-plugin/plugin.json` (`skills: "./skills/"`) and a copy of each skill
  tree. Hub `source` points at that package; `from` keeps the Git URL; `made`
  is true.
- GitHub clone directories use `owner-repo` so `caldoai/skills` and
  `typesafe-ai/skills` do not collide. Non-GitHub URLs keep the basename.
- Plugin update re-runs discovery on the clone root so synthesized packages
  refresh after `git pull`.
- Selection stays on the plugins axis; *Start with* continues to list
  `skills/*/SKILL.md` inside installed packages. Cursor remains without hub
  plugin/skill injection.

## Consequences

- Pasting a skill-only repository URL installs through the same dialog as
  plugins, without requiring upstream `plugin.json`.
- Supporting files next to `SKILL.md` survive the copy into the synthesized
  package.
- Existing clones under bare repository names keep working; only new installs
  use `owner-repo`.
- Empty repositories still fail, with copy that mentions skills as well as
  manifests.

## Evidence

- `crates/tools/src/plugins.rs` discovery, synthesis and `repo_name` tests
- Desktop `plugins.rs::clone_lists_discovered_plugins` skill-bundle case
- [plugin hub contract](../contracts/plugin-marketplace.md)
- [provider matrix](../quality/provider-matrix.md)
