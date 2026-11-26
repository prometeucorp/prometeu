---
type: Decision
title: "ADR 0001 — Knowledge versioned in the repository"
description: "A versioned knowledge bundle with a short map and one set of instructions for all agents."
status: stable
---

# ADR 0001 — Knowledge versioned in the repository

Date: 2026-09-03

## Context

The project grew with many comments in the code, a long README and special
instructions in `CLAUDE.md`. Important decisions and invariants were in the
repository. But to find them, a person had to know the correct file before.
Thus, a different agent or person can do the same investigation again. They can
also make an abstraction that is not compatible, or change a rule without
knowledge of its cause.

Long instructions that the agent loads in each session also use the context of
the task. When the project keeps a copy of the instructions in the format of
each agent, the copies become different with time.

## Options considered

1. Keep only comments and the README.
2. Keep a wiki or external documents.
3. Use one long instruction file for each tool.
4. Keep structured technical knowledge versioned adjacent to the code, with
   short entry files for agents.

## Decision

The `docs/` directory is the technical source of truth. It is also the OKF
knowledge bundle of the repository. Each concept has one clear responsibility
and a YAML envelope with `type`, `title`, `description` and `status`. The
current architecture documents and the large contracts stay as they are. This
format does not make it necessary to divide them. The `README.md` files of the
folders are indexes, and they do not have the envelope.

`docs/README.md` is a short map of the areas. The index of each area lists its
concepts by title and by a description of one line. When you do not know the
location, start there. Then open the applicable concepts and follow their
dependencies. When you know the path, open it directly. The project does not
have a second catalog or duplicate `index.md` files.

The root files `ARCHITECTURE.md`, `README.md`, `CONTRIBUTING.md`, `AGENTS.md`
and `CLAUDE.md` are not in the bundle. `ARCHITECTURE.md` is the map of the
system. `README.md` shows the product. `AGENTS.md` stays the short file of
instructions for all agents, with the invariants and the important commands.
The README files of components and fixtures, the GitHub templates and
`.claude/commands/` are also not in the bundle.

A file for one tool must import `AGENTS.md` or point to it. That file contains
only the differences of that tool. Contracts, decisions, quality and
operations have their own areas in `docs/`.

Documentation goes through the same versioning and review as the code. A change
to a behavior, a contract or a decision updates the applicable documentation in
the same change.

A new concept, or a concept with large changes, has the envelope and a line in
the index of its folder. `stable` means guidance that is in force. It does not
mean that all features are the same for all agents, or that the feature
operates in production. Proposals use `draft`, and the index lists them in a different
section. The [decision index](README.md) gives the format and the words for the
lifecycle. When an implementation plan is complete, it becomes a contract or an
ADR, or the project removes it from the tree. Git keeps its history. Pages that
persons write do not have a generation date, a verification date or an expiry
date. No `log.md` file copies the history of Git.

`scripts/check-docs.mjs` examines local links, the coverage of the indexes and
the four metadata fields. It also keeps the limit of lines for the agent map
and the check of the Claude import. These items are not part of this decision:

- Search or RAG, routing and graph tools.
- An OKF gem or validator, a Knowledge Catalog and a `.okf/` directory.

The decision does not add a knowledge store in Prometeu Cloud or in the relay.

The documentation in the repository tells about the current system. When a
decision becomes obsolete, the project moves its applicable guarantees to the
current documents. Then the project removes the obsolete decision. Git keeps
the history. [ADR 0044](0044-current-documentation.md) gives this lifecycle.

## Consequences

Positive:

- Persons and agents read the same source.
- Decisions get a history and an explicit status.
- The initial context stays small.
- Links, tests and CI can examine the documents.
- A new contributor can get the information without the author of the code.

Negative:

- Each change to the architecture adds a small cost to keep the documentation
  correct.
- Documents that are not correct can cause a person to trust incorrect
  information.
- The links and the agreement with the code must have a periodic review.

## Evidence

- `AGENTS.md` points to the documentation tree.
- `CLAUDE.md` imports the shared instructions.
- `docs/README.md` keeps the index for navigation.
- `scripts/check-docs.test.mjs` examines the reachability of the indexes, the
  metadata and the current checks of the standing instructions. This decision
  does not change application data, IPC or wire formats. Thus, compatibility
  here applies only to the navigation and the checks of the documentation.
- Contracts make a difference between the current state and proposals that do
  not have an implementation.

## References for this approach

- [OpenAI — Harness engineering](https://openai.com/index/harness-engineering/):
  a short `AGENTS.md` as the map and a versioned `docs/` as the source of truth.
- [AGENTS.md](https://agents.md/): a shared format of instructions for agents.
- [Claude Code — project memory](https://code.claude.com/docs/en/memory):
  short instructions for one repository.
- [C4 Model](https://c4model.com/diagrams): context and containers for the map
  of the architecture.
- [AWS — Architecture Decision Records](https://docs.aws.amazon.com/prescriptive-guidance/latest/architectural-decision-records/adr-process.html):
  context, decision, consequences and the lifecycle of an ADR.
