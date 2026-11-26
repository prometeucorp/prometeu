# Cursor ACP fixtures

`tool.ndjson` is a recording of Cursor CLI `2026.10.22-27dcf5f` from 2026-10-25.
The recording used the current authenticated account and Composer 2.5. The
prompt ran `pwd`, wrote `probe.txt` with the text `CURSOR_OK`, and kept a
synthetic marker in memory.

We replaced the session identifiers, the tool identifiers and the temporary
folder. We joined the streamed text that contains the folder path, redacted it and
divided it again. Thus, the fixture does not keep parts of a private path. The
fixture does not include model catalogs, inherited command lists, the
initialization or unrelated notifications.

The tests for the handshake, the terminal responses to the prompt, the failure
and the extension use synthetic data. `tests.rs` identifies them. The permission
request and the tool frames and text frames in this folder are native.

The production adapter makes `canonical-events.json` from this recording. The timestamps, the message IDs and the time values are constants. To make the
file again, run these commands:

```sh
cd src-tauri
PROMETEU_UPDATE_CURSOR_FIXTURE=1 cargo test -p prometeu-protocols cursor::tests::canonical_fixture_matches_recorded_cursor_output
```

A different desktop smoke test examines the
installed CLI through the production adapter. Cargo ignores this test by
default. This test includes the load of a
native session and the cancellation. The fixtures alone do not prove that the
service is available at this time, or that an interactive native window
operates.
