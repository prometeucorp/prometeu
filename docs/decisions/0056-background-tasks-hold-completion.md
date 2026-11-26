---
type: Decision
title: "ADR 0056 — A turn only completes when its background tasks drain"
description: "A turn completes only when all its background tasks stop."
status: stable
---

# ADR 0056 — A turn only completes when its background tasks drain

Date: 2026-09-23


## Context

Some provider features start child tasks that continue after the turn that
started them. Examples are the Claude `Agent` with `run_in_background`, and the
Codex `collabAgentToolCall` and `subAgentActivity`. The adapters change these
tasks into `background.changed` at this time. But each consumer of
`turn.completed` used that one event as the end of all work. These problems
occurred:

- The tab showed that the agent waited for input.
- The Stop button went off the screen.
- Prometeu sent a queued message to a process while the children of that
  process continued to operate.
- The delegation execution reported `completed`.
- If the completion notice came while tasks were not complete, Prometeu
  discarded it.

A client of Prometeu was able to see the contradiction. `get_execution`
answered `completed`, but `send_message` on the same conversation answered
`conversation_busy`.

## Decision

A conversation continues to work until these two conditions occur: the primary
agent completed its turn, and no background task operates. In each consumer,
`turn.completed` and `background.changed` are the two parts of one completion
of the work.

- `chat.rs` keeps the tasks that it sees for each conversation in the runtime
  state. It holds the terminal event that comes while the tasks operate. The
  tab stays `Rodando`. A queued message waits for the same completion of the work,
  not for the terminal event alone.
- `delegation.rs` records the outcome on the execution when the turn stops. It
  moves the execution to `completed` only when all the tasks stop. Thus a caller
  does not read a completed execution from a conversation that continues to
  reject its messages.
- `timeline.ts` gives `working`. This value is on while the turn or a task
  operates. It controls the Stop button and the busy composer.
- `alert.ts` holds the completion notice. It examines the notice when all the
  tasks stop, and it does not discard the notice.

When the primary agent answers again, each consumer discards a held terminal
event. `conversation::agent_activity` is the one definition of this rule, and
`alert.ts` has a copy of it. Without this rule, the end of the tasks can occur
during the resumed turn. Then Prometeu can complete the work of the tab, release
queued input and report the previous outcome for work that continues.

Background work also counts as work when Prometeu examines if a conversation is
busy. `Chat::working` includes the primary turn and the children that continue
after it. Thus these behaviors occur:

- When the person changes the account, Prometeu puts the message in the queue.
  It does not start the process again while its children operate.
- A credential refresh waits.
- Prometeu rejects a message for a target with no work in the same conditions where it
  rejects a delegated message.

An interruption stops all the work immediately in all four modules. It also
stops the children, and after that no provider sends a report that the tasks
stopped. If
Prometeu waits for such a report, the conversation stays busy on the screen
and does not stop.

The end of the tasks alone does not complete a turn. A terminal event from the
primary agent is always necessary. When the primary agent answers again, Prometeu
discards a held terminal event.

## Trade-offs

Prometeu shows the background work only as each adapter reports it. Some
providers do not report the end of a task. For other providers, Prometeu cannot
see the children. For example, Antigravity does not report child sessions at
this time. These providers keep their previous behavior:

- If the provider does not report the end of a task, the conversation stays
  open until the process stops or the person stops it.
- If Prometeu cannot see the children, the work stops on the terminal event, as
  before.

A correct state on the screen is more important than this dependency. The
alternative is to guess that the children completed when there are no events.
That guess is the bug that this decision removes.

At this time, Prometeu sets `Execution.outcome` while the state continues to be
`running`. This tells that the primary turn stopped and that the children continue
to work. Only the `completed` state tells that the work is complete. The
contract in [embedded MCP](../contracts/embedded-mcp.md) gives this rule. Thus
callers do not use the outcome as a completion signal.

The runtime background state stops with the process, as the state that shows that the agent waits for input.
After a new start, Prometeu reports no tasks. It does not make them again from
the transcript.

## Consequences

The rule is in `Work::observe`, and it uses canonical events. Thus there is one
implementation for all providers, not one for each adapter.
`SessionReactions` also sends resumed assistant activity to that rule. Its
regression test is in `crates/core/src/session/tests.rs`. It proves that a
child task that stops after new primary-agent activity cannot release an
expired terminal event.

Tests: `crates/core/src/conversation/work.rs::work_tests`, which includes the
resumed turn and the busy check. The `delegation.rs` tests for held completion
and interruption. `src/timeline.test.ts`, `src/alert.test.ts`, and the Stop
control in `e2e/composer.spec.ts`. The Claude and Codex tests send to
`Work::observe` the events that their adapter really sends, in `claude.rs` and
`codex.rs`. Thus the tests prove the shared rule for each provider, not with
events that a person wrote manually. This change updated the contracts in
[embedded MCP](../contracts/embedded-mcp.md) and
[conversation events V1](../contracts/conversation-events-v1.md). The
[conversation flow](../architecture/conversation-flow.md) gives the flow.
