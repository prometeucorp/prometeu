# ADR 0056 — A turn only completes when its background tasks drain

Date: 2026-09-23

Status: Accepted

## Context

Claude's `Agent` with `run_in_background`, Codex's `collabAgentToolCall` and
`subAgentActivity` and similar provider features start children that outlive the
turn that spawned them. The adapters already normalize them into
`background.changed`, but every consumer of `turn.completed` treated that single
event as the end of all work: the tab turned ready, the Stop button disappeared,
a queued message was pushed into a process still running its children, the
delegation execution reported `completed`, and the completion notice was dropped
whenever it arrived while tasks were pending.

The resulting contradiction was observable from outside: `get_execution`
answered `completed` while `send_message` on the same conversation answered
`conversation_busy`.

## Decision

Treat a conversation as working until both conditions hold: the main agent has
ended its turn and no background task is still running. `turn.completed` and
`background.changed` are two halves of one settlement, in every consumer.

- `chat.rs` keeps the observed tasks per conversation in runtime state and holds
  the terminal that arrives while they run. The tab stays `Rodando`, and a
  queued message waits for the same settlement instead of the raw terminal.
- `delegation.rs` records the outcome on the running execution when the turn
  ends and only moves it to `completed` when the tasks drain, so a caller never
  reads a finished execution from a conversation that still rejects its sends.
- `timeline.ts` exposes `working`, the union of the turn and the tasks, which
  drives the Stop button and the busy composer.
- `alert.ts` holds the completion notice and evaluates it when the tasks drain,
  instead of losing it.

The main agent answering again discards a held terminal, in every consumer:
`conversation::agent_activity` is the one definition of that, mirrored in
`alert.ts`. Without it, a drain that lands during the resumed turn would settle
the tab, release queued input and report the previous outcome for work still
running.

Background work also counts as work when deciding whether a conversation is
busy. `Chat::working` covers the main turn and the children that outlived one,
so switching accounts queues the message instead of restarting the process out
from under running children, a credential refresh waits, and an idle-only send
is rejected exactly when a delegated send would be.

An interruption settles everything immediately in all four places. It ends the
children too, and no provider owes us a drain report afterwards; waiting for one
would leave the conversation visibly stuck.

Draining alone still never completes a turn: a terminal from the main agent is
always required, and the main agent answering again discards a held one.

## Trade-offs

Background visibility remains exactly what each adapter reports. A provider that
never drains a task, or one whose children Prometeu cannot see — Antigravity
reports no child sessions today — keeps its previous behavior: the first case
holds the conversation open until the process stops or the person interrupts,
the second settles on the terminal as before. Correct visible state is worth
that dependency; the alternative, guessing that silence means the children
finished, is the bug this replaces.

`Execution.outcome` is now set while the state is still `running`, meaning "the
main turn ended, children are still working". Completion remains the `completed`
state alone. The contract in
[embedded MCP](../contracts/embedded-mcp.md) states it, so callers do not read
the outcome as a completion signal.

Runtime background state dies with the process, like readiness. A restart
reports no tasks rather than reconstructing them from the transcript.

## Consequences

The rule lives in `Work::observe`, over canonical events, so it is one
implementation for every provider rather than one per adapter.
`SessionReactions` now forwards resumed assistant activity to that rule; its
regression in `crates/core/src/session/tests.rs` proves a later child drain
cannot release a terminal invalidated by new main-agent activity.

Tests: `crates/core/src/conversation/work.rs::work_tests` including the resumed turn and the busy check, `delegation.rs` held-completion and interruption
tests, `src/timeline.test.ts`, `src/alert.test.ts`, the Stop control in
`e2e/composer.spec.ts`. Claude and Codex each drive `Work::observe` with what
their adapter really emits, in `claude.rs` and `codex.rs`, so the shared rule is
proven per provider instead of against handwritten events. Contracts updated in
[embedded MCP](../contracts/embedded-mcp.md) and
[conversation events V1](../contracts/conversation-events-v1.md); the flow is in
[conversation flow](../architecture/conversation-flow.md).
