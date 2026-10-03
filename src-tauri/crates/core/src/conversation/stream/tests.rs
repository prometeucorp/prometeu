use super::*;
use crate::lock::lock;
use std::sync::Mutex;

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> u64 {
        42
    }
}

#[derive(Default)]
struct Memory(Mutex<String>);
impl TranscriptStore for Memory {
    fn load(&self) -> Result<String, String> {
        Ok(lock(&self.0).clone())
    }
    fn append(&self, line: &str) -> Result<(), String> {
        let mut text = lock(&self.0);
        text.push_str(line);
        text.push('\n');
        Ok(())
    }
}

#[test]
fn canonical_usage_survives_while_internal_capture_fields_stay_private() {
    let frame = json!({"v":1,"type":"turn.completed","at":1,"outcome":"ok","message":"answer","durationMs":10,"costUsd":null,
            "providerDurationMs":10,"telemetry":{"usage":{"inputTokens":123}},
            "messageId":"reply","usage":{"complete":true,"usage":{"inputTokens":123}}});
    let serialized = frame.to_string();
    let public: Value = serde_json::from_str(&public_text(&serialized, &frame)).unwrap();
    assert!(public.get("telemetry").is_none());
    assert!(public.get("providerDurationMs").is_none());
    assert_eq!(public["message"], "answer");
    assert_eq!(public["durationMs"], 10);
    assert_eq!(public["messageId"], "reply");
    assert_eq!(public["usage"], frame["usage"]);
    assert_eq!(frame["telemetry"]["usage"]["inputTokens"], 123);
}

#[test]
fn failed_command_does_not_record_busy_or_user_message() {
    let mut lines = Lines::default();
    let error = lines.command(
        &json!({ "v": 1, "type": "message.send", "text": "retry me" }),
        &FixedClock,
        &mut |_: &Value, _: &str| Err("broken pipe".to_string()),
        |_, _| panic!("failed commands must not emit"),
    );
    assert_eq!(error.unwrap_err(), "broken pipe");
    assert!(lines.text.is_empty());
    assert_eq!(lines.seq, 0);
}

#[test]
fn answering_a_request_does_not_start_another_notification_cycle() {
    let mut lines = Lines::default();
    let events = lines
        .command(
            &json!({
                "v": 1,
                "type": "request.respond",
                "requestId": "ask-1",
                "response": { "outcome": "allow" },
            }),
            &FixedClock,
            &mut |_: &Value, _: &str| Ok(vec![]),
            |lines, event| {
                lines.record(
                    &event.to_string(),
                    event,
                    &Memory::default(),
                    &|_: &str, _| Ok(()),
                );
            },
        )
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "request.closed");
    assert_eq!(lines.seq, 1);
}

#[test]
fn numbers_lines_whether_retained_or_not() {
    let mut l = Lines::default();
    assert_eq!(l.absorb("a"), 1);
    assert_eq!(l.skip(), 2);
    assert_eq!(l.absorb("b"), 3);
    assert_eq!(l.text, "a\nb\n");
}

#[test]
fn retention_limit_drops_whole_lines() {
    let mut l = Lines::default();
    let fat = "x".repeat(KEEP);
    l.absorb(&fat);
    l.absorb("end");
    assert_eq!(l.text, "end\n");
    assert_eq!(l.seq, 2);
}

#[test]
fn retains_the_data_needed_to_restore_the_view() {
    let f = |s: &str| serde_json::from_str::<Value>(s).unwrap();
    assert!(keep(&f(r#"{"v":1,"type":"assistant.block"}"#)));
    assert!(keep(&f(r#"{"v":1,"type":"user.message"}"#)));
    assert!(keep(&f(r#"{"v":1,"type":"tool.completed"}"#)));
    assert!(keep(&f(r#"{"v":1,"type":"request.opened"}"#)));
    assert!(keep(&f(r#"{"v":1,"type":"turn.completed"}"#)));
    assert!(keep(&f(r#"{"v":1,"type":"context.compacted"}"#)));
    assert!(keep(&f(r#"{"v":1,"type":"system.notice"}"#)));
    assert!(!keep(&f(r#"{"v":1,"type":"assistant.started"}"#)));
    assert!(!keep(&f(r#"{"v":1,"type":"assistant.block.started"}"#)));
    assert!(!keep(&f(r#"{"v":1,"type":"assistant.delta"}"#)));
    assert!(!keep(&f(r#"{"v":1,"type":"tool.input.delta"}"#)));
    assert!(!keep(&f(r#"{"v":1,"type":"context.updated"}"#)));
    assert!(!keep(&f(r#"{"v":1,"type":"session.identity"}"#)));
    assert!(!keep(&f(r#"{"v":1,"type":"usage.updated"}"#)));
}

#[test]
fn accepted_input_precedes_concurrent_output_and_snapshot() {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let store = Memory::default();
    let lines = Mutex::new(Lines::default());
    let delivered = Mutex::new(Vec::new());
    let events = |text: &str, seq| {
        lock(&delivered).push((seq, serde_json::from_str::<Value>(text).unwrap()));
        Ok(())
    };
    let (accepted, accepted_rx) = channel();
    let (waiting, waiting_rx) = channel();
    std::thread::scope(|scope| {
        let (lines, store, events) = (&lines, &store, &events);
        let response = scope.spawn(move || {
            accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(lines.try_lock().is_err());
            waiting.send(()).unwrap();
            let frame = event(
                "assistant.block",
                43,
                json!({ "block": { "kind": "text", "text": "response" } }),
            );
            let mut lines = lock(lines);
            lines.record(&frame.to_string(), &frame, store, events);
            lines.snapshot(true, &FixedClock)
        });
        lock(lines)
            .command(
                &json!({ "v": 1, "type": "message.send", "text": "hello" }),
                &FixedClock,
                &mut |command: &Value, history: &str| {
                    assert_eq!(command["text"], "hello");
                    assert!(history.is_empty());
                    accepted.send(()).unwrap();
                    waiting_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    Ok(vec![event("system.notice", 42, json!({"detail": "echo"}))])
                },
                |lines, frame| {
                    lines.record(&frame.to_string(), frame, store, events);
                },
            )
            .unwrap();
        let snapshot = response.join().unwrap();
        assert_eq!(snapshot.seq, 4);
        let history: Vec<Value> = snapshot
            .text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            history
                .iter()
                .map(|frame| frame["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "user.message",
                "system.notice",
                "assistant.block",
                "session.state"
            ]
        );
        assert_eq!(history[0]["at"], 42);
        assert_eq!(history[3]["state"], "busy");
        assert_eq!(lock(lines).text, store.load().unwrap());
    });
    let delivered = lock(&delivered);
    assert_eq!(
        delivered
            .iter()
            .map(|(seq, frame)| (*seq, frame["type"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        [
            (1, "session.state"),
            (2, "user.message"),
            (3, "system.notice"),
            (4, "assistant.block")
        ]
    );
}

#[test]
fn failed_storage_and_delivery_still_advance_memory_without_retry() {
    struct BrokenStore(Mutex<usize>);
    impl TranscriptStore for BrokenStore {
        fn load(&self) -> Result<String, String> {
            Err("unreadable".into())
        }
        fn append(&self, _: &str) -> Result<(), String> {
            *lock(&self.0) += 1;
            Err("disk full".into())
        }
    }
    let store = BrokenStore(Mutex::new(0));
    assert!(Lines::seeded(&store).is_err());
    let sent = Mutex::new(Vec::new());
    let events = |text: &str, seq| {
        lock(&sent).push((text.to_string(), seq));
        Err("disconnected".into())
    };
    let mut lines = Lines::default();
    let frame = event("turn.completed", 42, json!({"outcome": "ok"}));
    let text = frame.to_string();
    assert_eq!(
        lines.record(&text, &frame, &store, &events),
        Some(Delivery {
            seq: 1,
            storage_error: Some("disk full".into()),
            event_error: Some("disconnected".into()),
        })
    );
    assert_eq!(*lock(&store.0), 1);
    assert_eq!(*lock(&sent), [(text.clone(), 1)]);
    assert_eq!(lines.text, text + "\n");
    assert_eq!(lines.snapshot(false, &FixedClock).seq, 1);
}

#[test]
fn telemetry_is_filtered_at_the_injected_storage_and_delivery_boundary() {
    let store = Memory::default();
    let sent = Mutex::new(Vec::new());
    let events = |text: &str, seq| {
        lock(&sent).push((text.to_string(), seq));
        Ok(())
    };
    let mut lines = Lines::default();
    let private = event("telemetry.usage", 1, json!({"usage": {"inputTokens": 100}}));
    assert!(lines
        .record(&private.to_string(), &private, &store, &events)
        .is_none());
    assert_eq!(lines.seq, 0);
    assert!(store.load().unwrap().is_empty());
    assert!(lock(&sent).is_empty());
    let frame = event(
        "turn.completed",
        2,
        json!({"outcome": "ok", "durationMs": 10,
        "telemetry": {"private": true}, "providerDurationMs": 12}),
    );
    lines.record(&frame.to_string(), &frame, &store, &events);
    let public: Value = serde_json::from_str(lines.text.trim()).unwrap();
    assert_eq!(
        public,
        event(
            "turn.completed",
            2,
            json!({"outcome": "ok", "durationMs": 10})
        )
    );
    assert_eq!(store.load().unwrap(), lines.text);
    assert_eq!(lock(&sent)[0], (public.to_string(), 1));
    assert_eq!(frame["telemetry"]["private"], true);
}

#[test]
fn restored_mixed_history_preserves_bytes_and_resets_transport_sequence() {
    let original = "{\"type\":\"user\",\"message\":{\"content\":\"old\"}}\n{\"v\":1,\"type\":\"user.message\",\"at\":3,\"content\":[]}";
    let store = Memory(Mutex::new(original.into()));
    let lines = Lines::seeded(&store).unwrap();
    assert_eq!(lines.text, original.to_string() + "\n");
    assert_eq!(store.load().unwrap(), original);
    let snapshot = lines.snapshot(false, &FixedClock);
    assert_eq!(snapshot.seq, 0);
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap(),
        json!({
            "text": format!("{}{}\n", lines.text, event("session.state", 42, json!({"state":"ready"}))),
            "seq": 0,
        })
    );
    assert_eq!(
        serde_json::from_value::<Snapshot>(serde_json::to_value(&snapshot).unwrap()).unwrap(),
        snapshot
    );
}

#[test]
fn trimming_a_unicode_seed_keeps_whole_records_without_rewriting_storage() {
    let tail = "{\"v\":1,\"type\":\"system.notice\",\"detail\":\"Unicode: 日本語\"}\n";
    let seed = "é".repeat(KEEP) + "\n" + tail;
    let store = Memory(Mutex::new(seed.clone()));
    let mut lines = Lines::seeded(&store).unwrap();
    assert_eq!(lines.text, tail);
    assert_eq!(lines.seq, 0);
    assert_eq!(store.load().unwrap(), seed);
    lines.absorb("next");
    assert_eq!(lines.seq, 1);
    assert_eq!(lines.text, tail.to_owned() + "next\n");
}

#[test]
fn request_outcomes_keep_the_existing_v1_contract() {
    for (input, expected) in [
        ("allow", "allowed"),
        ("deny", "denied"),
        ("answer", "answered"),
        ("cancel", "cancelled"),
    ] {
        let mut lines = Lines::default();
        let command = json!({"v":1,"type":"request.respond","requestId":"request-1","response":{"outcome":input}});
        let accepted = lines
            .command(
                &command,
                &FixedClock,
                &mut |received: &Value, _: &str| {
                    assert_eq!(received, &command);
                    Ok(vec![])
                },
                |_, _| {},
            )
            .unwrap();
        assert_eq!(
            accepted,
            [event(
                "request.closed",
                42,
                json!({"requestId":"request-1","outcome":expected})
            )]
        );
    }
}

#[test]
fn disconnected_delivery_does_not_resend_accepted_input_or_lose_its_history() {
    struct Input(usize);
    impl ConversationInput for Input {
        fn send(&mut self, _: &Value, _: &str) -> Result<Vec<Value>, String> {
            self.0 += 1;
            Ok(vec![])
        }
    }
    let store = Memory::default();
    let mut lines = Lines::default();
    let mut input = Input(0);
    lines
        .command(
            &json!({"v":1,"type":"message.send","text":"accepted once"}),
            &FixedClock,
            &mut input,
            |lines, frame| {
                let delivery = lines
                    .record(&frame.to_string(), frame, &store, &|_: &str, _| {
                        Err("disconnected".into())
                    })
                    .unwrap();
                assert_eq!(delivery.storage_error, None);
                assert_eq!(delivery.event_error.as_deref(), Some("disconnected"));
            },
        )
        .unwrap();
    assert_eq!(input.0, 1);
    assert_eq!(lines.seq, 2);
    assert_eq!(lines.text, store.load().unwrap());
    assert_eq!(lines.text.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(lines.text.trim()).unwrap()["content"][0]["text"],
        "accepted once"
    );
}
