# Completion evidence

The pure fence accepts less than the observation of native activity. This
difference is intentional. A Stop hook runs before the other hooks make the decision to continue the
turn or to stop it. PSP Done alone does not tell which native transcript records are
on the disk. Thus, the fence uses these items together:

- the Stop hook
- PSP Done
- the source watermark of the current submission
- a transcript poll that completes correctly and that started after the two
  signals

For Claude Code `2.1.326`, the installed Stop builder selects the last assistant
message. It connects the text blocks of that message with LF, and then applies the
JavaScript trim. It does not include the UUID of that message. Thus, the observer
must have these items from the source:

- the append revision
- the byte offset
- the UUID
- the API message ID
- the content
- the terminal stop reason

A previous response with the same text before the watermark cannot satisfy this
proof. A continuation with the same text must have a different API response.
Many stored blocks from one API response count only one time.

The managed invocation enables `CLAUDE_CODE_EAGER_FLUSH=1`. In this build, the
result branch of the engine waits for the transcript record and the local write
queue. Then it gives its result to the UI, and the UI publishes PSP Done after
that. This order makes sure that a reader can see the writes. It does not make them
durable with fsync. The stored content continues to be necessary, because a native writer can
stop an error and not send it forward.

For Antigravity `1.5.10`, the installed `NewStopHook` gets `finalModelOutput`
internally from the last planner response. But the external command hook does
not include that field, and this is intentional. A controlled native smoke test
recorded the stdin schema and showed this difference. Its exported schema
includes `executionNum`, `fullyIdle`, and `continuationActive`.
The last field means that a previous Stop continued the current turn. It
does not mean that this Stop will continue the turn. The embedded hook
documentation and the builder give the name `NO_TOOL_CALL` to a usual
completion. The `model_stop` example in the public web documentation does not
agree with this installed build.

In Antigravity, `PreInvocation.initialNumSteps` identifies the planner step of
the new invocation. The authenticated `lastUserInput` comes from the initial
user protobuf. The JSONL user content also includes XML tags and metadata for
the model. The observer connects the two items only when that invocation starts
immediately after the same `USER_INPUT` step. A source poll can occur before the
hook delivery. Thus, one pair of a user step and an assistant step, with a
limit, can wait for the connection to that index.

The last source step must agree with all these conditions:

- Its index is the same as the latest invocation index.
- Its type is `PLANNER_RESPONSE`, its source is `MODEL` and its status is
  `DONE`.
- It has no truncation and no tool calls that wait.
- It has a pair with the `fullyIdle=true`, `NO_TOOL_CALL` Stop of the current
  execution and with the PSP Done cycle after that Stop.

When an output text is available, it must agree fully. The external hook can
send no text. Then, the observer does not use a guess from the text in place of
the native proof of index and status. A snapshot that writes the source again
can replay this step. The native index of the step, not its projection phase,
shows that the step is new. A source that is unknown or not complete does not become a
correct completion only because the file stopped its changes.

This is the primary evidence from the installed sources. We examined it, and we
did not read unrelated histories:

| Build | SHA-256 | Relevant code |
| --- | --- | --- |
| Claude Code 2.1.326 | `e214ad76dcc0e846b03be28ed1c2734aacd4e27acb527cb7f8a9e2bed644e837` | Embedded JS Stop builder at byte 296993307. Engine eager-flush selection at 325647569. Transcript queue flush at 293438561 |
| Antigravity 1.5.10 | `780ee763a9938a37b45c7f9d6ae31747d7c574358fd10c64d3a1cdc36faf2798` | `hooks.NewStopHook.func1`, `hookutils.GetLastModelOutputAndThinking`, `chatconverters.StepToSummaryJSON`. Embedded hook contract near byte 67294200 |

The tests examine these conditions:

- previous content with the same text
- polls that are not complete
- native continuation
- replayed snapshots of the trajectory
- background work
- cancellation
- an expired process identity
- the replacement of the source
- newer input that cancels the evidence that waits

The selected managed Claude account also completed a controlled new native turn
through the production observer. Working, Stop, Done, the current assistant UUID,
the terminal stop reason and a poll after the two signals gave the last proof.
The test harness used private native UI preferences with the current credential
folder. It did not log in or copy credentials. Native tool inheritance was not
a part of that smoke test.

Synthetic tests alone do not prove the runtime composition. The host must do
these steps:

1. Keep the raw source metadata.
2. Connect the correct native submission.
3. Read all the statuses before it commits.
4. Cancel the fence when the transcript is missing or the identity has a
   failure.

Error results and native terminal reasons that Prometeu cannot use must have
their own evidence. This proof is only for a correct completion.
