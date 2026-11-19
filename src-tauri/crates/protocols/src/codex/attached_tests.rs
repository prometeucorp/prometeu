use super::*;

#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<Value>>>);
impl Write for Out {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).push(serde_json::from_slice(bytes).unwrap());
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn attached() -> (Link, Out) {
    let out = Out::default();
    let start = Start {
        permission: None,
        instructions: String::new(),
        cwd: "/workspace".into(),
        resume: None,
        model: "fixture-model".into(),
        effort: "low".into(),
        plugin_ids: vec![],
        plugin_hook_ids: vec![],
    };
    (
        Link::attached(
            Box::new(out.clone()),
            start,
            "native-thread".into(),
            "incarnation-one".into(),
            |_, en| en.into(),
        ),
        out,
    )
}
fn event(link: &mut Link, method: &str, params: Value) -> Vec<Value> {
    link.on_line(&json!({"method":method,"params":params}).to_string())
}

#[test]
fn attached_input_never_initializes_or_submits_another_native_turn() {
    let (mut link, out) = attached();
    assert!(lock(&out.0).is_empty());
    for frame in [
        json!({"v":1,"type":"message.send","text":"Native composer input"}),
        json!({"v":1,"type":"message.send","text":"/plan"}),
        json!({"v":1,"type":"commands.list"}),
        json!({"v":1,"type":"permission.mode.set","mode":"bypass"}),
    ] {
        assert!(link.write(&frame).is_err());
    }
    assert!(lock(&out.0).is_empty());
    event(
        &mut link,
        "turn/started",
        json!({"threadId":"native-thread","turn":{"id":"native-turn"}}),
    );
    link.write(&json!({"v":1,"type":"turn.interrupt"})).unwrap();
    link.refresh_usage().unwrap();
    let sent = lock(&out.0);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["method"], "turn/interrupt");
    assert_eq!(
        sent[0]["params"],
        json!({"threadId":"native-thread","turnId":"native-turn"})
    );
    assert_eq!(sent[1]["method"], "account/rateLimits/read");
}

#[test]
fn native_tui_resolutions_close_requests_without_guessing_the_decision() {
    let (mut link, out) = attached();
    for (method, thread) in [
        ("native/newRequest", "native-thread"),
        ("item/commandExecution/requestApproval", "other-thread"),
    ] {
        assert!(link
            .on_line(&json!({"id":7,"method":method,"params":{"threadId":thread}}).to_string())
            .is_empty());
    }
    assert!(lock(&out.0).is_empty());
    let opened = link.on_line(
        &json!({"id":7,"method":"item/commandExecution/requestApproval",
        "params":{"threadId":"native-thread","itemId":"native-tool","command":"echo fixture"}})
        .to_string(),
    );
    assert_eq!(opened[0]["type"], "request.opened");
    assert!(lock(&out.0).is_empty());
    let closed = event(
        &mut link,
        "serverRequest/resolved",
        json!({"threadId":"native-thread","requestId":7}),
    );
    assert_eq!(closed[0]["type"], "request.closed");
    assert_eq!(closed[0]["requestId"], opened[0]["requestId"]);
    assert_eq!(closed[0]["outcome"], "resolved");
    assert!(event(
        &mut link,
        "serverRequest/resolved",
        json!({"threadId":"native-thread","requestId":7})
    )
    .is_empty());
    assert!(link
        .write(
            &json!({"v":1,"type":"request.respond","requestId":opened[0]["requestId"],"response":{"outcome":"allow"}})
        )
        .is_err());
    assert!(lock(&out.0).is_empty());
}

#[test]
fn attached_assistant_blocks_keep_native_item_identity_across_stream_and_final() {
    let (mut link, _) = attached();
    event(
        &mut link,
        "turn/started",
        json!({"threadId":"native-thread","turn":{"id":"native-turn"}}),
    );
    for (id, text) in [
        ("native-message-one", "First answer"),
        ("native-message-two", "Second answer"),
    ] {
        let item = json!({"id":id,"type":"agentMessage","text":text});
        let started = event(
            &mut link,
            "item/started",
            json!({"threadId":"native-thread","item":item}),
        );
        assert!(started.iter().all(|frame| frame["messageId"] == id));
        let deltas = event(
            &mut link,
            "item/agentMessage/delta",
            json!({"threadId":"native-thread","itemId":id,"delta":text}),
        );
        assert_eq!(deltas[0]["messageId"], id);
        let final_frames = event(
            &mut link,
            "item/completed",
            json!({"threadId":"native-thread","item":item}),
        );
        assert_eq!(final_frames[0]["messageId"], id);
        assert_eq!(final_frames[0]["index"], 0);
        assert_eq!(final_frames[0]["block"]["text"], text);
    }
}

#[test]
fn attached_explicit_request_response_preserves_rpc_id_and_payload() {
    let (mut link, out) = attached();
    let opened = link.on_line(&json!({"id":"native-request","method":"item/tool/requestUserInput", "params":{
        "threadId":"native-thread","questions":[{"id":"q-one","question":"Which fixture?","options":[]}]}}).to_string());
    link.write(
        &json!({"v":1,"type":"request.respond","requestId":opened[0]["requestId"],
        "response":{"outcome":"answer","answers":{"Which fixture?":"First"}}}),
    )
    .unwrap();
    assert_eq!(
        lock(&out.0)[0],
        json!({"jsonrpc":"2.0","id":"native-request","result":{"answers":{"q-one":{"answers":["First"]}}}})
    );
}

#[test]
fn attached_answer_waits_for_native_resolution_and_cannot_be_repeated() {
    let (mut link, out) = attached();
    let request = json!({"id":"question","method":"item/tool/requestUserInput", "params":{
        "threadId":"native-thread","questions":[{"id":"q-one","question":"Which fixture?","options":[]}]}}).to_string();
    let opened = link.on_line(&request);
    let answer = json!({"v":1,"type":"request.respond","requestId":opened[0]["requestId"],
        "response":{"outcome":"answer","answers":{"Which fixture?":"First"}}});
    assert!(link.write(&answer).unwrap().is_empty());
    assert!(link.write(&answer).is_err());
    assert!(link.on_line(&request).is_empty());
    assert!(link.write(&answer).is_err());
    assert_eq!(lock(&out.0).len(), 1);
    let resolution = json!({"method":"serverRequest/resolved","params":{"threadId":"native-thread","requestId":"question"}}).to_string();
    let closed = link.on_line(&resolution);
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["type"], "request.closed");
    assert_eq!(closed[0]["requestId"], opened[0]["requestId"]);
    assert_eq!(closed[0]["outcome"], "resolved");
    assert!(link.on_line(&resolution).is_empty());
    assert!(link.write(&answer).is_err());
}

#[test]
fn resumed_native_engine_cannot_accept_a_stale_remote_answer_with_a_reused_rpc_id() {
    let (mut first, _) = attached();
    let (mut second, out) = attached();
    second.request_namespace = Some("incarnation-two".into());
    let request = json!({"id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":"native-thread","itemId":"tool","command":"echo fixture"}}).to_string();
    let previous = first.on_line(&request)[0]["requestId"].clone();
    let current = second.on_line(&request)[0]["requestId"].clone();
    assert_ne!(previous, current);
    assert!(second.write(&json!({"v":1,"type":"request.respond","requestId":previous,"response":{"outcome":"allow"}})).is_err());
    assert!(lock(&out.0).is_empty());
    second.write(&json!({"v":1,"type":"request.respond","requestId":current,"response":{"outcome":"deny"}})).unwrap();
    assert_eq!(lock(&out.0)[0]["id"], 7);
    assert_eq!(lock(&out.0)[0]["result"]["decision"], "decline");
    assert_ne!(first.request_key(&json!(7)), first.request_key(&json!("7")));
}

#[test]
fn attached_settings_and_account_quota_updates_keep_the_existing_canonical_routes() {
    let (mut link, out) = attached();
    link.refresh_usage().unwrap();
    let id = lock(&out.0)[0]["id"].clone();
    let quotas=link.on_line(&json!({"id":id,"result":{"rateLimits":{"limitId":"codex"},"rateLimitsByLimitId":{"codex":{"limitId":"codex"},"spark":{"limitId":"spark"}}}}).to_string());
    assert_eq!(quotas[0]["type"], "usage.updated");
    assert_eq!(quotas[0]["provider"], "codex");
    assert_eq!(
        quotas[0]["usage"]["rateLimitsByLimitId"]["spark"]["limitId"],
        "spark"
    );
    event(
        &mut link,
        "thread/settings/updated",
        json!({"threadId":"native-thread","threadSettings":{"collaborationMode":{"mode":"plan","settings":{"model":"new-model","reasoning_effort":"high"}}}}),
    );
    assert_eq!(link.model, "new-model");
    assert_eq!(link.start.effort, "high");
    let updated = event(
        &mut link,
        "account/rateLimits/updated",
        json!({"rateLimits":{"limitId":"codex","primary":{"usedPercent":25}}}),
    );
    assert_eq!(updated[0]["type"], "usage.updated");
    assert_eq!(updated[0]["usage"]["primary"]["usedPercent"], 25);
}
