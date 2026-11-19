use super::*;

fn hook(nonce: &str, tool: &str, input: Value) -> NativeHookEvent {
    serde_json::from_value(json!({
        "provider":"claude", "session":"tab", "process_nonce":nonce,
        "native_session":"native", "kind":"permission_requested", "tool_name":tool,
        "tool_input":input
    }))
    .unwrap()
}

#[test]
fn explicit_reply_does_not_rewrite_input_or_persist_permissions_or_claim_native_acceptance() {
    let registry = Decisions::new("generation");
    let input = json!({"command":"printf safe", "description":"An explicit command"});
    let pending = registry
        .open(&hook("generation", "Bash", input.clone()))
        .unwrap();
    let opened = registry.drain().pop().unwrap();
    assert!(opened["toolId"].is_null());
    let command = json!({"type":"request.respond", "requestId":opened["requestId"], "response":{"outcome":"allow"}});
    let responding = registry.clone();
    let worker = std::thread::spawn(move || responding.respond(&command));
    let delivery = pending
        .receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        delivery.output,
        json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest", "decision":{"behavior":"allow"}}})
    );
    pending.deliver(delivery, true);
    assert!(worker.join().unwrap().is_ok());
    assert_eq!(registry.drain()[0]["outcome"], "resolved");
    drop(pending);
    assert!(registry.drain().is_empty());
}

#[test]
fn invocation_and_generation_prevent_stale_replies_and_no_timeout_is_an_allow() {
    let registry = Decisions::new("first");
    assert!(registry.open(&hook("second", "Bash", json!({}))).is_none());
    let pending = registry.open(&hook("first", "Bash", json!({}))).unwrap();
    let first = registry.drain().pop().unwrap();
    drop(pending);
    assert_eq!(registry.drain()[0]["outcome"], "cancelled");
    let _next = registry.open(&hook("first", "Bash", json!({}))).unwrap();
    let next = registry.drain().pop().unwrap();
    assert_ne!(first["requestId"], next["requestId"]);
    assert!(registry.respond(&json!({"type":"request.respond","requestId":first["requestId"],"response":{"outcome":"allow"}})).is_err());
    registry.close();
    assert!(registry.open(&hook("first", "Bash", json!({}))).is_none());
    assert_eq!(registry.drain()[0]["outcome"], "cancelled");
}

#[test]
fn questions_require_exact_answers_and_preserve_every_original_field() {
    let input = json!({"questions":[{"question":"Choose a label", "options":[{"label":"Blue","description":"Blue label"}]}], "metadata":{"source":"native"}});
    let answer = response(
        "AskUserQuestion",
        &input,
        &json!({"outcome":"answer","answers":{"Choose a label":"Custom"}}),
    )
    .unwrap();
    let mut expected = input.clone();
    expected["answers"] = json!({"Choose a label":"Custom"});
    assert_eq!(
        answer["hookSpecificOutput"]["decision"]["updatedInput"],
        expected
    );
    assert!(response("AskUserQuestion", &input, &json!({"outcome":"allow"})).is_err());
    assert!(response(
        "AskUserQuestion",
        &input,
        &json!({"outcome":"answer","answers":{"Wrong":"Blue"}})
    )
    .is_err());
    assert!(response(
        "Bash",
        &json!({}),
        &json!({"outcome":"answer","answers":{}})
    )
    .is_err());
    let plan = json!({"plan":"The exact plan", "allowedPrompts":[]});
    assert_eq!(
        response("ExitPlanMode", &plan, &json!({"outcome":"allow"})).unwrap()["hookSpecificOutput"]
            ["decision"]["updatedInput"],
        plan
    );
    assert_eq!(
        response(
            "Bash",
            &json!({}),
            &json!({"outcome":"deny","message":"Please revise"})
        )
        .unwrap()["hookSpecificOutput"]["decision"],
        json!({"behavior":"deny","message":"Please revise"})
    );
}

#[test]
fn bounded_pending_requests_do_not_block_native_fallback_and_withdrawal_never_denies() {
    let registry = Decisions::new("first");
    let pending: Vec<_> = (0..LIMIT)
        .map(|_| registry.open(&hook("first", "Bash", json!({}))).unwrap())
        .collect();
    assert!(registry.open(&hook("first", "Bash", json!({}))).is_none());
    assert_eq!(registry.drain().len(), LIMIT);
    registry.withdraw();
    assert!(pending.iter().all(|item| !item.active()));
    assert!(registry
        .drain()
        .iter()
        .all(|event| event["outcome"] == "cancelled"));
    assert!(registry.open(&hook("first", "Bash", json!({}))).is_some());
}
