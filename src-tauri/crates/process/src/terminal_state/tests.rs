use super::*;

fn key() -> Key {
    Key {
        session: "session".into(),
        generation: "generation".into(),
    }
}

fn shell(script: &str, timeout: Duration) -> Client {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    Client::launch(command, timeout).unwrap()
}

fn size() -> Size {
    Size { cols: 20, rows: 4 }
}

#[test]
fn viewport_character_columns_reject_wrong_scalar_counts_and_impossible_positions() {
    let lines = vec!["e\u{0301}界🐣".into(), "".into()];
    assert!(valid_viewport_characters(
        &lines,
        &[vec![0, 0, 1, 3], vec![]],
        12,
        2
    ));
    for columns in [
        vec![vec![0, 0, 1], vec![]],
        vec![vec![0, 0, 1, 3, 3], vec![]], // UTF-16 units are not Unicode scalars.
        vec![vec![0, 0, 3, 1], vec![]],
        vec![vec![0, 0, 1, 12], vec![]],
        vec![vec![0, 0, 1, 3]],
    ] {
        assert!(!valid_viewport_characters(&lines, &columns, 12, 2));
    }
    let oversized = protocol::MAX_VIEWPORT_CHARACTERS + 1;
    assert!(!valid_viewport_characters(
        &["x".repeat(oversized)],
        &[vec![0; oversized]],
        12,
        1
    ));
}

#[test]
fn owned_helper_is_reaped_on_drop() {
    let helper = shell("sleep 30", Duration::from_secs(1));
    let pid = helper.system_id().unwrap();
    drop(helper);
    let status = Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .unwrap();
    assert!(!status.status.success());
}

#[test]
fn timeout_retires_helper_and_later_calls_do_not_replay() {
    let mut helper = shell("sleep 30", Duration::from_millis(40));
    assert_eq!(helper.create(&key(), size(), 100), Err(Error::Timeout));
    assert_eq!(helper.system_id(), None);
    assert_eq!(helper.create(&key(), size(), 100), Err(Error::Closed));
}

#[test]
fn mismatched_ids_versions_and_extra_frames_retire_the_channel() {
    for line in [
        r#"{"v":1,"id":"different","ok":true,"result":{"seq":0}}"#,
        r#"{"v":2,"id":"request-1","ok":true,"result":{"seq":0}}"#,
        r#"{"v":1,"id":"request-1","ok":true,"result":{"seq":0},"error":{"code":"limit"}}"#,
        "not json",
        "{}\n{}",
    ] {
        let script = format!("read request; printf '%s\\n' '{line}'; sleep 30");
        let mut helper = shell(&script, Duration::from_secs(1));
        assert_eq!(
            helper.create(&key(), size(), 100),
            Err(Error::InvalidResponse)
        );
        assert_eq!(helper.system_id(), None);
    }
}

#[test]
fn remote_rejection_keeps_valid_channel_and_never_includes_terminal_content() {
    let mut helper = shell(
        "read request; printf '%s\\n' '{\"v\":1,\"id\":\"request-1\",\"ok\":false,\"error\":{\"code\":\"limit\"}}'; \
         read request; printf '%s\\n' '{\"v\":1,\"id\":\"request-2\",\"ok\":true,\"result\":{\"seq\":0}}'; \
         while read request; do :; done",
        Duration::from_secs(1),
    );
    assert_eq!(
        helper.create(&key(), size(), 100),
        Err(Error::Rejected(Rejection::Limit))
    );
    assert!(helper.system_id().is_some());
    helper.create(&key(), size(), 100).unwrap();
    helper.shutdown().unwrap();
}

#[test]
fn invalid_write_ack_or_base64_replies_retire_the_helper() {
    for result in [
        r#"{"seq":2,"replies":[]}"#,
        r#"{"seq":1,"replies":["%%%"]}"#,
    ] {
        let script = format!("read request; printf '%s\\n' '{{\"v\":1,\"id\":\"request-1\",\"ok\":true,\"result\":{result}}}'; sleep 30");
        let mut helper = shell(&script, Duration::from_secs(1));
        assert_eq!(helper.write(&key(), 1, b"x"), Err(Error::InvalidResponse));
        assert_eq!(helper.system_id(), None);
    }
}

#[test]
fn partial_chunk_failure_invalidates_the_snapshot_association() {
    let mut helper = shell(
        "read request; printf '%s\\n' '{\"v\":1,\"id\":\"request-1\",\"ok\":true,\"result\":{\"seq\":0}}'; \
         read request; printf '%s\\n' '{\"v\":1,\"id\":\"request-2\",\"ok\":true,\"result\":{\"seq\":262144,\"replies\":[]}}'; \
         read request; exit 0",
        Duration::from_secs(2),
    );
    let mut stream = Stream::create(&mut helper, key(), size(), 100).unwrap();
    let bytes = vec![b'x'; protocol::MAX_WRITE_BYTES + 1];
    assert!(stream.write_chunk(&mut helper, 1, &bytes).is_err());
    assert_eq!(stream.chunk, 0);
    assert_eq!(stream.bytes, protocol::MAX_WRITE_BYTES as u64);
    assert!(matches!(stream.snapshot(&mut helper), Err(Error::Closed)));
    assert!(matches!(
        stream.write_chunk(&mut helper, 1, b"retry"),
        Err(Error::Closed)
    ));
}

#[test]
fn process_start_and_frame_limits_are_explicit() {
    let missing = Client::launch(
        Command::new("/prometeu-missing-terminal-helper"),
        Duration::from_secs(1),
    );
    assert!(matches!(missing, Err(Error::Unavailable)));
    let mut helper = shell("while read request; do :; done", Duration::from_secs(1));
    assert_eq!(
        helper.write(&key(), 1, &vec![b'x'; protocol::MAX_WRITE_BYTES + 1]),
        Err(Error::Limit)
    );
    assert_eq!(
        helper.write(&key(), protocol::MAX_SAFE_SEQUENCE + 1, b"x"),
        Err(Error::Limit)
    );
    assert!(helper.system_id().is_some());
}

#[test]
#[ignore = "requires the built SEA helper or npm ci --prefix packages/terminal-state and Node"]
fn real_helper_preserves_wire_snapshot_and_maps_byte_offsets_to_pty_chunks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let command = match std::env::var_os("PROMETEU_TEST_TERMINAL_STATE_HELPER") {
        Some(path) => Command::new(path),
        None => {
            let mut command = Command::new("node");
            command.arg(root.join("packages/terminal-state/src/cli.ts"));
            command
        }
    };
    let mut helper = Client::launch(command, Duration::from_secs(5)).unwrap();
    let mut stream = Stream::create(&mut helper, key(), size(), 100).unwrap();
    let first = "normal\r\n\x1b[?1049h\x1b[?2004h\x1b]8;;https://example.test\x1b\\é".as_bytes();
    let receipt = stream.write_chunk(&mut helper, 1, first).unwrap();
    assert_eq!(receipt.chunk_seq, 1);
    assert_eq!(receipt.byte_seq, first.len() as u64);
    assert!(matches!(
        stream.write_chunk(&mut helper, 1, b"duplicate"),
        Err(Error::Sequence)
    ));
    let query = stream.write_chunk(&mut helper, 2, b"\x1b[6n").unwrap();
    assert_eq!(query.replies, vec![b"\x1b[2;2R".to_vec()]);
    let state = stream.snapshot(&mut helper).unwrap();
    assert_eq!(
        state.state.viewport_lines.len(),
        usize::from(state.state.rows)
    );
    assert_eq!(state.state.appearance.background, "#282c34");
    assert_eq!(state.state.viewport_character_columns[1], vec![0]);
    assert_eq!(state.chunk_seq, 2);
    assert_eq!(state.state.seq, (first.len() + 4) as u64);
    assert!(state.state.normal.ansi.contains("normal"));
    assert_eq!(state.state.active, protocol::BufferKind::Alternate);
    assert!(state.state.modes.bracketed_paste_mode);
    assert_eq!(state.state.links[0].uri, "https://example.test");
    let bytes = serde_json::to_vec(&state.state).unwrap();
    assert_eq!(
        serde_json::from_slice::<Snapshot>(&bytes).unwrap(),
        state.state
    );
    stream
        .resize(&mut helper, Size { cols: 30, rows: 6 })
        .unwrap();
    assert_eq!(stream.snapshot(&mut helper).unwrap().state.cols, 30);
    let large = vec![b'x'; protocol::MAX_WRITE_BYTES + 1];
    let applied = stream.write_chunk(&mut helper, 3, &large).unwrap();
    assert_eq!(applied.chunk_seq, 3);
    assert_eq!(stream.snapshot(&mut helper).unwrap().chunk_seq, 3);
    stream.write_chunk(&mut helper, 4, &[0xf0, 0x9f]).unwrap();
    let partial = stream.snapshot(&mut helper).unwrap();
    assert_eq!(partial.state.pending_utf8, "8J8=");
    stream.dispose(&mut helper).unwrap();
    let next = Key {
        session: "session".into(),
        generation: "replacement".into(),
    };
    helper.create(&next, size(), 100).unwrap();
    assert_eq!(
        helper.write(&key(), 1, b"stale"),
        Err(Error::Rejected(Rejection::StaleGeneration))
    );
    helper.dispose(&next).unwrap();
    helper.shutdown().unwrap();
}
