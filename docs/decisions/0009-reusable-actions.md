---
type: Decision
title: "ADR 0009 — reusable commands and agents responsible for tasks"
description: "Commands and profiles that a person can use again, with one session that is responsible for each task."
status: stable
---

# ADR 0009 — reusable commands and agents responsible for tasks

Date: 2026-09-05

## Context

The Open PR button sends a request to the current conversation. Persons must be
able to use prompts and configurable agents again, without a connection between
the task and one button. One delivery can do a review, open a PR, and answer
comments and CI in the same session.

## Options considered

1. Only skills: they let the agent use instructions again. But they do not show
   the model selection, the tools, the stored execution and the tracking by the
   software.
2. One agent for each phase and a general workflow editor: they make
   coordination and handoffs necessary. An independent review is not necessary
   yet.
3. Commands with two behaviors and profiles that persons can use again. They
   connect the chat, the menu and the PR button through the current sessions.

## Decision

Use the third option. A command adds its prompt to the message, or it starts
an agent in a different tab. Profiles have a prompt, a model, a provider, an
effort, MCP, plugins, skills and permissions. A project can replace a profile.
Executions store the resolved configuration. The session continues to be the
transcript.

One session is responsible for the delivery. The review, the publication and
the fixes are instructions in the profile. They are not deterministic gates
of a workflow engine. A text result from the agent does not prove the approval of a
review. The example profile tells the agent to examine the published code. It
does not let the agent merge.

The application does not follow the PR of a task without the person. It does
not poll comments or CI, and it does not start an automatic agent turn. The
project removed a previous optional PR monitor together with the versioned
automation workflows. Thus, the project can design the automation again from
the start. The application ignores the stored fields of that monitor (see the
contract). The adapters continue to make the provider differences into
configuration.

## Consequences

The person can start with a prompt or a task and use it again in different
projects. It is not necessary to make three agents for one PR. A
workflow editor and handoffs between sessions stay possible changes for the
future. The project does not add structures for them at this time.

Feedback from PR comments and CI goes to a task only when the person sends it.
The skill selection is an instruction. It does not isolate a capability.

The contract is in [actions](../contracts/actions.md). The storage only adds
fields. Previous boards get the defaults from
[ADR 0010](0010-default-code-review.md) one time, and the application does not
make tasks for them.

## Evidence

- [Commands and expansion](../../src/actions.test.ts).
- [Storage, validation and removal of duplicates](../../src-tauri/src/actions.rs).
- [GitHub reads and filters](../../src-tauri/src/github.rs).
- [Registration, expansion and the start in a different tab](../../e2e/actions.spec.ts).
