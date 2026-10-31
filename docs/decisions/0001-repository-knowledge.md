---
type: Decision
title: "ADR 0001 — Knowledge versioned in the repository"
description: "A versioned knowledge bundle with short navigation and shared standing instructions."
status: stable
---

# ADR 0001 — Knowledge versioned in the repository

Date: 2026-09-03

## Context

The project grew with detailed code comments, a narrative README and specific
instructions in `CLAUDE.md`. Important decisions and invariants existed, but
finding them required knowing the right file beforehand. Another agent or person
could repeat an investigation, create an incompatible abstraction or change a
rule without noticing its reason.

Large instructions loaded in every session also compete with the task's context.
Keeping copies in each agent's specific format increases drift.

## Options considered

1. Keep only comments and the README.
2. Keep a wiki or external documents.
3. Use one large instruction file per tool.
4. Keep structured technical knowledge versioned next to the code, with short
   entry files for agents.

## Decision

The `docs/` directory is the technical source of truth and the repository's OKF
knowledge bundle. Each concept has one cohesive responsibility and a YAML
envelope with `type`, `title`, `description` and `status`. Existing architecture
documents and large contracts remain intact; this format does not require
splitting them. Folder `README.md` files are indexes, exempt from the envelope.

`docs/README.md` is a short area map. Each area's index lists its concepts by
title and one-line description. Start there when the location is unknown, then
open relevant concepts and follow their dependencies; known paths can be opened
directly. No second catalog or duplicate `index.md` files are needed.

Root `ARCHITECTURE.md`, `README.md`, `CONTRIBUTING.md`, `AGENTS.md` and `CLAUDE.md`
stay outside the bundle. `ARCHITECTURE.md` maps the system, `README.md` presents
the product and `AGENTS.md` remains the short standing-instructions file with
invariants and essential commands. Component/fixture READMEs, GitHub templates
and `.claude/commands/` are also outside the bundle.

Tool-specific files must import or point to `AGENTS.md` and contain only the
real differences of that tool. Contracts, decisions, quality and operations
have their own areas in `docs/`.

Documentation goes through the same versioning and review as the code. A change
in behavior, contract or decision updates the corresponding documentation in the
same change.

New and substantially changed concepts include the envelope and a line in their
folder index. `stable` means guidance in force, not complete feature parity or
proof of deployment. Proposals use `draft` and are listed separately. The
[decision index](README.md) defines the format and lifecycle vocabulary.
Finished implementation plans become contracts or ADRs, or leave the tree;
Git retains their history. Human-written pages need no generation, verification
or expiry timestamps, and no `log.md` duplicates Git.

`scripts/check-docs.mjs` checks local links, index coverage and the four metadata
fields, while retaining the agent-map line limit and Claude import check.
Search/RAG, routing, graph tools, an OKF gem/validator, a Knowledge Catalog and
a `.okf/` directory are outside this decision. No Cloud or relay knowledge
store is introduced.

The checked-out documentation describes the current system. Obsolete decisions
are removed after their remaining guarantees are consolidated; Git retains the
history. [ADR 0044](0044-current-documentation.md) defines this lifecycle.

## Consequences

Positive:

- humans and agents consult the same source;
- decisions gain history and an explicit status;
- the initial context stays small;
- documents can be verified by links, tests and CI;
- onboarding does not depend on the memory of whoever wrote the code.

Negative:

- every architectural change gains a small documentation maintenance cost;
- incorrect documents can give false confidence;
- links and compatibility with the code will need periodic review.

## Evidence

- `AGENTS.md` points to the documentation tree.
- `CLAUDE.md` imports the shared instructions.
- `docs/README.md` keeps the navigable index.
- `scripts/check-docs.test.mjs` covers index reachability, metadata and the
  existing standing-instruction checks. No application data, IPC or wire format
  changes; compatibility here concerns the documentation navigation and checks.
- contracts distinguish the current state from proposals not yet implemented.

## References for this approach

- [OpenAI — Harness engineering](https://openai.com/index/harness-engineering/):
  a short `AGENTS.md` as the map and a versioned `docs/` as the source of truth.
- [AGENTS.md](https://agents.md/): common instruction format for agents.
- [Claude Code — project memory](https://code.claude.com/docs/en/memory):
  concise instructions scoped to the repository.
- [C4 Model](https://c4model.com/diagrams): context and containers for the
  architectural map.
- [AWS — Architecture Decision Records](https://docs.aws.amazon.com/prescriptive-guidance/latest/architectural-decision-records/adr-process.html):
  context, decision, consequences and ADR lifecycle.
