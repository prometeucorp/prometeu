# Antigravity stream fixtures

`tool.ndjson`, `resume.ndjson`, and `interrupted.ndjson` are recordings from the
installed Antigravity CLI 1.2.7 on 2026-09-19. The CLI used its current external
account in an empty temporary workspace. We replaced only the conversation
identity and the work folder with fixture constants.

The first prompt told the agent to run only `echo AGY_TOOL_OK` and to send a
reply with the same marker. The second process resumed that native conversation
and gave the marker again. The third process got SIGINT after its first step.
Version 1.2.7 returned `ERROR` with `error: "interrupted"`. The adapter changes
this result into an interrupted turn.

`canonical-events.json` is the output of the adapter for the three live
recordings. Tests in different languages use it for the V1 validation. Each
recording uses a new adapter instance. The tool recording and the resume
recording share the initial session identity. The interrupted conversation is independent and has a different fixture
identity. The event timestamps and the turn times are zero. Thus, the comparison always
gives the same result.
To make the file again, run
`PROMETEU_UPDATE_ANTIGRAVITY_FIXTURE=1 cargo test antigravity::tests::canonical_events_match_cross_language_fixture`
from `src-tauri/`.

`permission-denied.ndjson` is a smaller version of a 1.2.7 recording from the CLI. The
recording used an empty diagnostic folder and the default launch flags of the
application. The fixture keeps the `ERROR` tool state and the `SUCCESS` result
with `denied_actions` that is not empty. The command did not run, and the agent
did not go around a denial. These events must show a permission failure to the
person. They must not show an empty turn that completed correctly.

`usage.tsv` is a recording of `agy -p /usage`. It contains the remaining quotas
and the reset timestamps for each model group. It does not contain an identity
or credentials. The tests change the remaining values into used values and keep
the groups. They reject incorrect data and do not replace the cache.
