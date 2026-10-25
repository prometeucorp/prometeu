# Cursor ACP fixtures

`tool.ndjson` records Cursor CLI `2026.10.22-27dcf5f` on 2026-10-25 using the
existing authenticated account and Composer 2.5. The prompt ran `pwd`, wrote
`probe.txt` containing `CURSOR_OK`, and remembered a synthetic marker.

Session/tool identifiers and the temporary directory are replaced. Streamed
text containing the directory is combined, redacted and split again to avoid
retaining fragments of a private path. Model catalogs, inherited command lists,
initialization and unrelated notifications are omitted. Handshake, prompt
terminal responses, failure and extension tests are synthetic and identified in
`tests.rs`; the permission request and tool/text frames here are native.

`canonical-events.json` is produced by the real adapter from this recording.
Timestamps, message IDs and durations are normalized. Regenerate with:

```sh
cd src-tauri
PROMETEU_UPDATE_CURSOR_FIXTURE=1 cargo test -p prometeu-protocols cursor::tests::canonical_fixture_matches_recorded_cursor_output
```

The separate ignored desktop smoke test verifies the installed CLI through the
production adapter, including native session loading and cancellation. Fixtures
alone do not prove current service availability or an interactive native window.
