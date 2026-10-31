use super::*;

#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Output {
    fn take(&self) -> Vec<Value> {
        String::from_utf8(std::mem::take(&mut *lock(&self.0)))
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
fn start(resume: bool) -> Start {
    Start {
        cwd: "/fixture/worktree".into(),
        resume: resume.then(|| "fixture-session".into()),
        model: "composer-2.5[fast=true]".into(),
        plan: false,
        bypass: false,
        instructions: "Follow project instructions.".into(),
        mcp_servers: vec![],
    }
}
fn frame(link: &mut Link, value: Value) -> Vec<Value> {
    link.on_line(&value.to_string())
}
fn result(link: &mut Link, id: u64, value: Value) -> Vec<Value> {
    frame(link, json!({"jsonrpc":"2.0","id":id,"result":value}))
}
fn send(link: &mut Link) {
    link.write(&json!({"v":1,"type":"message.send","text":"Check workspace"}))
        .unwrap();
}
fn ready(resume: bool) -> (Link, Output) {
    let output = Output::default();
    let mut link = Link::new(Box::new(output.clone()), start(resume));
    assert_eq!(output.take()[0]["method"], "initialize");
    send(&mut link);
    result(
        &mut link,
        1,
        json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
    );
    assert_eq!(output.take()[0]["method"], "authenticate");
    result(&mut link, 2, json!({}));
    let command = output.take().remove(0);
    assert_eq!(
        command["method"],
        if resume {
            "session/load"
        } else {
            "session/new"
        }
    );
    assert_eq!(command["params"]["cwd"], "/fixture/worktree");
    assert_eq!(command["params"]["mcpServers"], json!([]));
    assert!(frame(&mut link, json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"replayed history"}}}})).is_empty());
    result(
        &mut link,
        3,
        if resume {
            json!({})
        } else {
            json!({"sessionId":"fixture-session"})
        },
    );
    assert_eq!(
        output.take()[0]["params"]["modelId"],
        "composer-2.5[fast=true]"
    );
    result(&mut link, 4, json!({}));
    assert_eq!(output.take()[0]["params"]["modeId"], "agent");
    result(&mut link, 5, json!({}));
    let prompt = output.take().remove(0);
    assert_eq!(prompt["method"], "session/prompt");
    assert!(prompt["params"]["prompt"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("Follow project instructions."));
    (link, output)
}
fn permission(link: &mut Link, id: Value) -> Vec<Value> {
    frame(
        link,
        json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission","params":{"sessionId":"fixture-session","toolCall":{"toolCallId":"tool","kind":"execute","title":"pwd"},"options":[{"kind":"allow_once","optionId":"yes"},{"kind":"reject_once","optionId":"no"}]}}),
    )
}
#[test]
fn recorded_tools_streaming_and_turns_survive_native_resume_without_replay_duplication() {
    for resume in [false, true] {
        let (mut link, output) = ready(resume);
        let mut events = vec![];
        for line in include_str!("fixtures/tool.ndjson").lines() {
            let emitted = link.on_line(line);
            for event in &emitted {
                if event["type"] == "request.opened" {
                    let response = link.write(&json!({"v":1,"type":"request.respond","requestId":event["requestId"],"response":{"outcome":"allow"}})).unwrap();
                    assert_eq!(response[0]["type"], "request.closed");
                    assert_eq!(
                        output.take()[0]["result"]["outcome"]["optionId"],
                        "allow-once"
                    );
                }
            }
            events.extend(emitted);
        }
        events.extend(result(&mut link, 6, json!({"stopReason":"end_turn"})));
        assert!(events.iter().any(|e| e["type"] == "tool.completed"
            && e["output"]
                .as_str()
                .unwrap()
                .contains("/fixture/worktree\nCURSOR_OK")));
        assert!(events.iter().any(|e| e["type"] == "assistant.block"
            && e["block"]["kind"] == "text"
            && e["block"]["text"].as_str().unwrap().contains("CURSOR_OK")));
        assert_eq!(events.last().unwrap()["outcome"], "ok");
        assert!(result(&mut link, 6, json!({"stopReason":"end_turn"})).is_empty());
        let previous = link.message.clone();
        send(&mut link);
        assert_ne!(link.message, previous);
        assert_eq!(
            output.take()[0]["params"]["prompt"][0]["text"],
            "Check workspace"
        );
        assert!(link
            .write(&json!({"v":1,"type":"message.send","text":"Busy"}))
            .is_err());
    }
}
#[test]
fn approval_denial_bypass_and_cancellation_preserve_wire_ids() {
    let (mut link, output) = ready(false);
    let opened = permission(&mut link, json!(0));
    link.write(&json!({"v":1,"type":"request.respond","requestId":opened[0]["requestId"],"response":{"outcome":"deny","message":"No"}})).unwrap();
    let reply = output.take().remove(0);
    assert_eq!(reply["id"], 0);
    assert_eq!(reply["result"]["outcome"]["optionId"], "no");
    link.write(&json!({"v":1,"type":"permission.mode.set","mode":"bypass"}))
        .unwrap();
    assert!(permission(&mut link, json!("approval")).is_empty());
    assert_eq!(output.take()[0]["result"]["outcome"]["optionId"], "yes");
    link.start.bypass = false;
    permission(&mut link, json!("cancel-me"));
    let events = link.write(&json!({"v":1,"type":"turn.interrupt"})).unwrap();
    assert_eq!(events[0]["outcome"], "cancelled");
    let sent = output.take();
    assert_eq!(sent[0]["method"], "session/cancel");
    assert_eq!(sent[1]["result"]["outcome"]["outcome"], "cancelled");
    link.start.bypass = true;
    assert!(permission(&mut link, json!("late")).is_empty());
    assert_eq!(
        output.take()[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert_eq!(
        result(&mut link, 6, json!({"stopReason":"cancelled"}))
            .last()
            .unwrap()["outcome"],
        "interrupted"
    );
}
#[test]
fn questions_and_plan_approvals_use_existing_request_contract() {
    let (mut link, output) = ready(false);
    let opened = frame(
        &mut link,
        json!({"jsonrpc":"2.0","id":"q","method":"cursor/ask_question","params":{"questions":[{"id":"color","prompt":"Which color?","options":[{"id":"blue","label":"Blue"}],"allowMultiple":false}]}}),
    );
    assert_eq!(opened[0]["kind"], "question");
    link.write(&json!({"v":1,"type":"request.respond","requestId":opened[0]["requestId"],"response":{"outcome":"answer","answers":{"Which color?":"Blue"}}})).unwrap();
    assert_eq!(
        output.take()[0]["result"]["outcome"]["answers"][0]["selectedOptionIds"],
        json!(["blue"])
    );
    let plan = frame(
        &mut link,
        json!({"jsonrpc":"2.0","id":7,"method":"cursor/create_plan","params":{"plan":"Build a widget"}}),
    );
    assert!(plan
        .iter()
        .any(|e| e["type"] == "assistant.block" && e["block"]["text"] == "Build a widget"));
    link.write(&json!({"v":1,"type":"request.respond","requestId":plan.last().unwrap()["requestId"],"response":{"outcome":"deny","message":"Keep existing widget"}})).unwrap();
    assert_eq!(
        output.take()[0]["result"],
        json!({"outcome":{"outcome":"rejected","reason":"Keep existing widget"}})
    );
    let answer = question_answer(&json!({"questions":[{"id":"q","prompt":"Color?","options":[{"id":"blue","label":"Blue"}]}]}), &json!({"outcome":"answer","answers":{"Color?":"Use green"}})).unwrap();
    assert_eq!(
        answer,
        json!({"outcome":{"outcome":"skipped","reason":"Color?: Use green"}})
    );
}
#[test]
fn startup_errors_never_leak_diagnostics_or_replace_lost_history() {
    for (id, resume, code) in [
        (2, false, "err.cursor.auth"),
        (3, true, "err.cursor.resume"),
    ] {
        let output = Output::default();
        let mut link = Link::new(Box::new(output.clone()), start(resume));
        send(&mut link);
        result(
            &mut link,
            1,
            json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
        );
        if id == 3 {
            result(&mut link, 2, json!({}));
        }
        output.take();
        let events = frame(
            &mut link,
            json!({"jsonrpc":"2.0","id":id,"error":{"message":"secret-token"}}),
        );
        assert!(events
            .iter()
            .any(|e| e["type"] == "turn.completed" && e["outcome"] == "error"));
        assert!(!serde_json::to_string(&events)
            .unwrap()
            .contains("secret-token"));
        assert!(serde_json::to_string(&events).unwrap().contains(code));
        assert!(output.take().is_empty());
        assert!(link.out.is_none());
        assert!(link
            .write(&json!({"v":1,"type":"message.send","text":"retry"}))
            .is_err());
    }
}

#[test]
fn failed_handshake_disconnects_host_input_and_releases_the_child_pipe() {
    let output = Output::default();
    let mut protocol = connect(Box::new(output.clone()), start(true));
    assert!(protocol.input.is_connected());
    protocol
        .input
        .send(
            &json!({"v":1,"type":"message.send","text":"Check workspace"}),
            "",
        )
        .unwrap();
    (protocol.translate)(&json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}).to_string());
    (protocol.translate)(
        &json!({"jsonrpc":"2.0","id":2,"error":{"message":"Login required"}}).to_string(),
    );
    assert!(!protocol.input.is_connected());
    assert_eq!(
        Arc::strong_count(&output.0),
        1,
        "stdin must be dropped while the reader still exists"
    );

    // The host replaces a disconnected conversation on the next input. Its new connection must
    // authenticate again and retain the requested native session, not silently create another.
    let (mut resumed, output) = ready(true);
    assert_eq!(resumed.session.as_deref(), Some("fixture-session"));
    assert_eq!(
        result(&mut resumed, 6, json!({"stopReason":"end_turn"}))
            .last()
            .unwrap()["outcome"],
        "ok"
    );
    assert!(output.take().is_empty());
}

#[test]
fn multiple_choice_answers_ignore_click_order_but_preserve_free_text() {
    let (mut link, output) = ready(false);
    for (answer, outcome) in [
        ("Red, Blue", "answered"),
        ("Blue, Red", "answered"),
        ("Blue, Red, Green", "skipped"),
    ] {
        let opened = frame(
            &mut link,
            json!({"jsonrpc":"2.0","id":"q","method":"cursor/ask_question","params":{"questions":[{"id":"color","prompt":"Colors?","options":[{"id":"red","label":"Red"},{"id":"blue","label":"Blue"}],"allowMultiple":true}]}}),
        );
        link.write(&json!({"v":1,"type":"request.respond","requestId":opened[0]["requestId"],"response":{"outcome":"answer","answers":{"Colors?":answer}}})).unwrap();
        let reply = output.take().remove(0);
        assert_eq!(reply["result"]["outcome"]["outcome"], outcome);
        if outcome == "answered" {
            assert_eq!(
                reply["result"]["outcome"]["answers"][0]["selectedOptionIds"],
                json!(["red", "blue"])
            );
        } else {
            assert_eq!(
                reply["result"]["outcome"]["reason"],
                "Colors?: Blue, Red, Green"
            );
        }
    }
}
#[test]
fn foreign_sessions_and_unknown_client_calls_cannot_change_the_turn() {
    let (mut link, output) = ready(false);
    assert!(frame(&mut link, json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"child","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"private child"}}}})).is_empty());
    assert!(frame(
        &mut link,
        json!({"jsonrpc":"2.0","id":99,"method":"fs/read_text_file","params":{"path":"/secret"}})
    )
    .is_empty());
    assert_eq!(output.take()[0]["error"]["code"], -32601);
    assert!(link.on_line("not JSON").is_empty());
}
#[test]
fn catalog_preserves_acp_ids_and_rejects_invalid_models() {
    let value = json!({"models":{"availableModels":[{"modelId":"composer-2.5[fast=true]","name":"Composer"}]}});
    assert_eq!(
        parse_catalog(&value).unwrap()[0].id,
        "composer-2.5[fast=true]"
    );
    assert!(parse_catalog(&json!({})).is_err());
    assert!(
        parse_catalog(&json!({"models":{"availableModels":[{"modelId":"", "name":"Auto"}]}}))
            .is_err()
    );
}

#[test]
fn canonical_fixture_matches_recorded_cursor_output() {
    let (mut link, _) = ready(false);
    let mut events = vec![];
    for line in include_str!("fixtures/tool.ndjson").lines() {
        let batch = link.on_line(line);
        let requests: Vec<_> = batch
            .iter()
            .filter(|e| e["type"] == "request.opened")
            .map(|e| e["requestId"].clone())
            .collect();
        events.extend(batch);
        for id in requests {
            events.extend(link.write(&json!({"v":1,"type":"request.respond","requestId":id,"response":{"outcome":"allow"}})).unwrap());
        }
    }
    events.extend(result(&mut link, 6, json!({"stopReason":"end_turn"})));
    for event in &mut events {
        event["at"] = json!(0);
        if event.get("messageId").is_some() {
            event["messageId"] = json!("cursor-message");
        }
        if event.get("requestId").is_some() {
            event["requestId"] = json!("cursor-request");
        }
        if event.get("durationMs").is_some() {
            event["durationMs"] = json!(0);
        }
    }
    let serialized = serde_json::to_string_pretty(&json!({"events":events})).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/cursor/fixtures/canonical-events.json");
    if std::env::var_os("PROMETEU_UPDATE_CURSOR_FIXTURE").is_some() {
        std::fs::write(&path, &serialized).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), serialized);
}

#[test]
fn malformed_questions_and_empty_plans_never_leave_unanswerable_cards() {
    let (mut link, output) = ready(false);
    assert!(frame(&mut link, json!({"jsonrpc":"2.0","id":"bad","method":"cursor/ask_question","params":{"questions":[{"id":"q","prompt":null,"options":[]}]}})).is_empty());
    assert_eq!(output.take()[0]["result"]["outcome"]["outcome"], "skipped");
    assert!(frame(
        &mut link,
        json!({"jsonrpc":"2.0","id":"bad-plan","method":"cursor/create_plan","params":{}})
    )
    .is_empty());
    assert_eq!(output.take()[0]["result"]["outcome"]["outcome"], "rejected");
    assert!(link.asks.is_empty());
}

#[test]
fn session_sends_the_complete_client_mcp_set_on_new_and_load() {
    let servers = json!([
        {"name":"notes","command":"npx","args":["notes"],"env":[]},
        {"type":"http","name":"api","url":"https://example.test/mcp","headers":[]}
    ]);
    for resume in [false, true] {
        let output = Output::default();
        let mut settings = start(resume);
        settings.mcp_servers = servers.as_array().unwrap().clone();
        let mut link = Link::new(Box::new(output.clone()), settings);
        assert_eq!(output.take()[0]["method"], "initialize");
        result(
            &mut link,
            1,
            json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
        );
        assert_eq!(output.take()[0]["method"], "authenticate");
        result(&mut link, 2, json!({}));
        let command = output.take().remove(0);
        assert_eq!(
            command["method"],
            if resume {
                "session/load"
            } else {
                "session/new"
            }
        );
        assert_eq!(command["params"]["mcpServers"], servers);
    }
}

#[test]
fn resume_preserves_native_mode_after_plan_approval() {
    let output = Output::default();
    let mut settings = start(true);
    settings.plan = true;
    let mut link = Link::new(Box::new(output.clone()), settings);
    result(
        &mut link,
        1,
        json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
    );
    result(&mut link, 2, json!({}));
    result(&mut link, 3, json!({"modes":{"currentModeId":"agent"}}));
    output.take();
    result(&mut link, 4, json!({}));
    assert_eq!(output.take()[0]["params"]["modeId"], "agent");
}

#[test]
fn restarted_rpc_counters_do_not_hide_new_permission_cards_in_the_transcript() {
    let (mut first, _) = ready(false);
    let old = permission(&mut first, json!(0))[0]["requestId"].clone();
    let (mut resumed, output) = ready(true);
    let current = permission(&mut resumed, json!(0))[0]["requestId"].clone();
    assert_ne!(old, current);
    resumed.write(&json!({"v":1,"type":"request.respond","requestId":current,"response":{"outcome":"allow"}})).unwrap();
    assert_eq!(output.take()[0]["id"], 0);
}
