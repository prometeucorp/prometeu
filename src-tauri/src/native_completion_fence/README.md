# Completion evidence

The pure fence is deliberately narrower than native activity observation. A
Stop hook runs before other hooks have finished deciding whether to continue.
PSP Done alone does not identify which native transcript records reached disk.
The fence combines both with a current submission's source watermark and a
successful transcript poll started after both observations.

For Claude Code `2.1.326`, the installed Stop builder selects the last assistant
message and joins its text blocks with LF, then applies JavaScript trim. It
does not include that message's UUID. The observer therefore needs the original
append revision, byte offset, UUID, API message ID, content, and terminal stop
reason. An old identical response before the watermark cannot satisfy this
proof. A continuation with identical text needs another distinct API response;
multiple persisted blocks from one API response do not count twice.

The managed invocation enables `CLAUDE_CODE_EAGER_FLUSH=1`. In this build the
engine's result branch awaits transcript recording and the local write queue
before yielding its result to the UI, which later publishes PSP Done. This is
visibility after writes, not fsync durability. Persisted content is still
required because a native writer may handle an error without propagating it.

For Antigravity `1.5.10`, the installed `NewStopHook` obtains
`finalModelOutput` internally from the last planner response, but the external
command hook deliberately omits that field. A controlled native smoke captured
the stdin schema and confirmed this distinction. Its exported schema includes
`executionNum`, `fullyIdle`, and `continuationActive`; the latter means that a
previous Stop continued the current turn, not that this Stop will continue.
The embedded hook documentation and builder name normal completion
`NO_TOOL_CALL`. The public web documentation's `model_stop` example does not
match this installed build.

Antigravity's `PreInvocation.initialNumSteps` identifies the new invocation's
planner step. The authenticated `lastUserInput` comes from the original user
protobuf; the JSONL user content instead includes model-facing XML tags and
metadata. The observer binds them only when that invocation starts immediately
after the exact `USER_INPUT` step. A source poll may precede hook delivery, so
one bounded user/assistant pair can wait for that exact index binding.

A final source step must equal the latest invocation index, have type
`PLANNER_RESPONSE`, source `MODEL`, status `DONE`, no truncation or pending tool
calls, and be paired with the current execution's `fullyIdle=true`,
`NO_TOOL_CALL` Stop plus the subsequent PSP Done cycle. When an output text is
available it must match exactly; absence from the external hook does not replace
the native index/status proof with a text guess. A snapshot rewrite may replay this
step; its native index, rather than its projection phase, establishes novelty.
An unknown or incomplete source never becomes a successful completion merely
because the file stopped changing.

Primary installed-source evidence, inspected without reading unrelated
histories:

| Build | SHA-256 | Relevant code |
| --- | --- | --- |
| Claude Code 2.1.326 | `e214ad76dcc0e846b03be28ed1c2734aacd4e27acb527cb7f8a9e2bed644e837` | Embedded JS Stop builder at byte 296993307; engine eager-flush selection at 325647569; transcript queue flush at 293438561 |
| Antigravity 1.5.10 | `780ee763a9938a37b45c7f9d6ae31747d7c574358fd10c64d3a1cdc36faf2798` | `hooks.NewStopHook.func1`, `hookutils.GetLastModelOutputAndThinking`, `chatconverters.StepToSummaryJSON`; embedded hook contract near byte 67294200 |

The tests cover old identical content, partial polls, native continuation,
replayed trajectory snapshots, background work, cancellation, stale process
identity, source replacement, and newer input invalidating pending evidence.
The selected managed Claude account also passed a controlled fresh native turn
through the real observer: Working, Stop, Done, the current assistant UUID and
terminal stop reason, and a poll after both signals supplied the final proof.
The harness used private native UI preferences with the existing credential
directory, without login or copying credentials. Native tool inheritance was
not part of that smoke.

Synthetic tests alone do not certify runtime composition. The host must retain
raw source metadata, associate the correct native submission, drain statuses
before committing, and invalidate the fence on transcript loss or identity
failure. Error outcomes and unsupported native terminal reasons require their
own evidence rather than this successful-completion proof.
