use super::*;

fn request(bridge: &HookBridge, hook: &str) -> Vec<u8> {
    let event = normalize(
        &bridge.identity,
        &json!({
            "hook_event_name":hook,"session_id":"native","tool_name":"Bash",
            "tool_input":{"command":"printf safe"}
        }),
    )
    .unwrap();
    let mut bytes = serde_json::to_vec(&Envelope {
        version: 1,
        token: bridge.token.clone(),
        event,
    })
    .unwrap();
    bytes.push(b'\n');
    bytes
}

fn send(bridge: &HookBridge, hook: &str) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut socket = connect(&bridge.directory.join("socket"), deadline).unwrap();
    write_until(&mut socket, &request(bridge, hook), deadline).unwrap();
    socket
}

#[test]
fn pending_decision_does_not_block_observations_and_only_explicit_response_reaches_its_socket() {
    let (mut bridge, hooks) =
        HookBridge::start(ProviderId::Claude, "tab", "generation", Some("native")).unwrap();
    let decisions = bridge.enable_claude_decisions().unwrap();
    let mut socket = send(&bridge, "PermissionRequest");
    assert_eq!(
        hooks.recv_timeout(DEADLINE).unwrap().kind,
        NativeHookKind::PermissionRequested
    );
    let opened = decisions.drain().pop().unwrap();
    let ordinary = send(&bridge, "PreToolUse");
    assert_eq!(
        read_fd(
            ordinary.as_raw_fd(),
            16,
            Instant::now() + DEADLINE,
            None,
            true
        )
        .unwrap(),
        b"ok\n"
    );
    assert_eq!(
        hooks.recv_timeout(DEADLINE).unwrap().kind,
        NativeHookKind::ToolStarted
    );
    let responder = decisions.clone();
    let worker = std::thread::spawn(move || {
        responder.respond(&json!({"type":"request.respond","requestId":opened["requestId"],"response":{"outcome":"deny","message":"Revise the command"}}))
    });
    let bytes = read_fd(
        socket.as_raw_fd(),
        WIRE_LIMIT,
        Instant::now() + DEADLINE,
        None,
        true,
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        reply["hookSpecificOutput"]["decision"],
        json!({"behavior":"deny","message":"Revise the command"})
    );
    write_until(&mut socket, b"written\n", Instant::now() + DEADLINE).unwrap();
    assert!(worker.join().unwrap().is_ok());
    assert_eq!(decisions.drain()[0]["outcome"], "resolved");
    bridge.close();
}

#[test]
fn native_completion_and_bridge_shutdown_release_pending_helpers_without_any_decision() {
    let (mut bridge, hooks) =
        HookBridge::start(ProviderId::Claude, "tab", "generation", Some("native")).unwrap();
    let decisions = bridge.enable_claude_decisions().unwrap();
    let permission = send(&bridge, "PermissionRequest");
    hooks.recv_timeout(DEADLINE).unwrap();
    let stale = decisions.drain().pop().unwrap();
    let stop = send(&bridge, "Stop");
    assert_eq!(
        read_fd(stop.as_raw_fd(), 16, Instant::now() + DEADLINE, None, true).unwrap(),
        b"ok\n"
    );
    assert_eq!(
        read_fd(
            permission.as_raw_fd(),
            16,
            Instant::now() + DEADLINE,
            None,
            true
        )
        .unwrap(),
        b"ok\n"
    );
    hooks.recv_timeout(DEADLINE).unwrap();
    assert_eq!(decisions.drain()[0]["outcome"], "cancelled");
    assert!(decisions.respond(&json!({"type":"request.respond","requestId":stale["requestId"],"response":{"outcome":"allow"}})).is_err());
    let pending = send(&bridge, "PermissionRequest");
    hooks.recv_timeout(DEADLINE).unwrap();
    decisions.drain();
    bridge.close();
    assert_eq!(
        read_fd(
            pending.as_raw_fd(),
            16,
            Instant::now() + DEADLINE,
            None,
            true
        )
        .unwrap(),
        b"ok\n"
    );
    assert_eq!(decisions.drain()[0]["outcome"], "cancelled");
}

#[test]
fn helper_disconnection_withdraws_the_card_and_does_not_affect_the_next_invocation() {
    let (mut bridge, hooks) =
        HookBridge::start(ProviderId::Claude, "tab", "generation", Some("native")).unwrap();
    let decisions = bridge.enable_claude_decisions().unwrap();
    let permission = send(&bridge, "PermissionRequest");
    hooks.recv_timeout(DEADLINE).unwrap();
    let id = decisions.drain().pop().unwrap()["requestId"].clone();
    drop(permission);
    let deadline = Instant::now() + DEADLINE;
    loop {
        let events = decisions.drain();
        if let Some(event) = events.first() {
            assert_eq!(event["requestId"], id);
            assert_eq!(event["outcome"], "cancelled");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let _next = send(&bridge, "PermissionRequest");
    hooks.recv_timeout(DEADLINE).unwrap();
    assert_ne!(decisions.drain()[0]["requestId"], id);
    bridge.close();
}
