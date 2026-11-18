use super::*;

// Synthetic data only. Shapes follow openai/codex's protocol and rollout recorder sources;
// Antigravity's installed CLI transcript-format description; and Cursor CLI 2026.11.13's
// agent-transcript serializer (role/message.content and explicit turn_ended). No private logs.
// Codex: github.com/openai/codex/blob/main/codex-rs/history/src/rollout_payload.rs and
// codex-rs/protocol/src/protocol.rs. AGY's public hooks bind transcriptPath:
// antigravity.google/docs/hooks. Cursor: installed 7923.index.js, agent-transcript/dist/index.js.
fn observer(format: TranscriptFormat) -> NativeTranscript {
    NativeTranscript::new(TranscriptBinding {
        format,
        native_session: "fixture-session".into(),
        transcript_path: PathBuf::from("/fixture/native.jsonl"),
    })
    .unwrap()
}

fn lines(values: &[Value]) -> Vec<u8> {
    values
        .iter()
        .map(|value| format!("{value}\n"))
        .collect::<String>()
        .into_bytes()
}

fn cursor(role: &str, text: &str) -> Value {
    json!({"role":role,"message":{"content":[{"type":"text","text":text}]}})
}

fn image_row(format: InlineImageFormat, data: &str) -> Value {
    match format {
        InlineImageFormat::Claude => {
            json!({"type":"user","sessionId":"fixture-session","message":{"content":[
            {"type":"text","text":"Read the image 🦀"},
            {"type":"image","source":{"data":data,"media_type":"image/png","type":"base64"}}
        ]},"uuid":"user-identity"})
        }
        InlineImageFormat::Codex => {
            json!({"type":"event_msg","payload":{"type":"user_message","message":"Read the image 🦀","images":[format!("data:image/png;base64,{data}")],"image_details":["original"],"client_id":"user-identity"}})
        }
    }
}

#[test]
fn terminal_transcript_inline_images_survive_every_framing_boundary() {
    for format in [InlineImageFormat::Claude, InlineImageFormat::Codex] {
        let raw = lines(&[image_row(format, "AAEC/w==")]);
        // Escaped key spelling, escaped slash and ASCII Unicode escapes are valid native JSON.
        let raw = String::from_utf8(raw)
            .unwrap()
            .replace("\"data\"", "\"d\\u0061ta\"")
            .replace("/w==", "\\/w==")
            .replace("AAEC", "\\u0041AEC")
            .into_bytes();
        for split in 0..=raw.len() {
            let mut tail = JsonlTail::with_inline_images(format);
            let mut records = tail
                .feed(0, &raw[..split], TranscriptPhase::Replay)
                .unwrap()
                .records;
            let batch = tail
                .feed(split as u64, &raw[split..], TranscriptPhase::Live)
                .unwrap();
            assert!(!batch.loss && !batch.partial, "split {split}");
            records.extend(batch.records);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].omitted_media, 1);
            assert_eq!(records[0].offset, 0);
            assert_eq!(tail.next_offset(), raw.len() as u64);
            match format {
                InlineImageFormat::Claude => {
                    assert_eq!(
                        records[0].value["message"]["content"][0]["text"],
                        "Read the image 🦀"
                    );
                    assert!(records[0].value["message"]["content"][1]["source"]["data"].is_null());
                    assert_eq!(records[0].value["uuid"], "user-identity");
                }
                InlineImageFormat::Codex => {
                    assert_eq!(records[0].value["payload"]["message"], "Read the image 🦀");
                    assert!(records[0].value["payload"]["images"][0].is_null());
                    assert_eq!(records[0].value["payload"]["image_details"][0], "original");
                }
            }
        }
    }
}

#[test]
fn terminal_transcript_large_inline_images_keep_text_metadata_and_bounded_memory() {
    let data = "A".repeat(2 * TRANSCRIPT_LINE_LIMIT);
    for format in [InlineImageFormat::Claude, InlineImageFormat::Codex] {
        let raw = lines(&[
            image_row(format, &data),
            json!({"type":"following","identity":"kept"}),
        ]);
        let mut tail = JsonlTail::with_inline_images(format);
        let mut records = Vec::new();
        for chunk in raw.chunks(8191) {
            let batch = tail
                .feed(tail.next_offset(), chunk, TranscriptPhase::Replay)
                .unwrap();
            assert!(!batch.loss);
            assert!(tail.pending.len() <= TRANSCRIPT_LINE_LIMIT);
            records.extend(batch.records);
        }
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].omitted_media, 1);
        assert_eq!(records[1].ordinal, 1);
        assert_eq!(records[1].value["identity"], "kept");
        assert!(records[0].value.to_string().len() < 1024);
    }
    let row = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[
        {"type":"input_image","image_url":format!("data:image/png;base64,{data}"),"detail":"high"},
        {"type":"input_text","text":"Preserve this text"},
        {"type":"input_image","file_id":"file-opaque","detail":"low"}
    ]}});
    let mut tail = JsonlTail::with_inline_images(InlineImageFormat::Codex);
    let batch = tail.feed(0, &lines(&[row]), TranscriptPhase::Live).unwrap();
    assert!(!batch.loss);
    assert_eq!(batch.records[0].omitted_media, 1);
    let content = &batch.records[0].value["payload"]["content"];
    assert_eq!(content[0]["detail"], "high");
    assert_eq!(content[1]["text"], "Preserve this text");
    assert_eq!(content[2]["file_id"], "file-opaque");
}

#[test]
fn terminal_transcript_image_filter_preserves_other_strings_and_reports_real_loss() {
    let ordinary = json!({"message":{"content":[{"type":"text","source":{"data":"ordinary 🦀 text"}}]},"data":"AAAA","image_url":"AAAA"});
    let mut tail = JsonlTail::with_inline_images(InlineImageFormat::Claude);
    let batch = tail
        .feed(0, &lines(&[ordinary.clone()]), TranscriptPhase::Replay)
        .unwrap();
    assert!(!batch.loss);
    assert_eq!(batch.records[0].value, ordinary);
    assert_eq!(batch.records[0].omitted_media, 0);
    let url = json!({"type":"event_msg","payload":{"type":"user_message","message":"Keep URL","images":["https://example.invalid/image.png"]}});
    let mut tail = JsonlTail::with_inline_images(InlineImageFormat::Codex);
    let batch = tail
        .feed(0, &lines(&[url.clone()]), TranscriptPhase::Replay)
        .unwrap();
    assert!(!batch.loss);
    assert_eq!(batch.records[0].value, url);

    for format in [InlineImageFormat::Claude, InlineImageFormat::Codex] {
        let mut tail = JsonlTail::with_inline_images(format);
        let batch = tail
            .feed(
                0,
                &lines(&[
                    json!({"text":"x".repeat(TRANSCRIPT_LINE_LIMIT + 1)}),
                    json!({"kept":true}),
                ]),
                TranscriptPhase::Live,
            )
            .unwrap();
        assert!(batch.loss);
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].value["kept"], true);
        for invalid in ["AA==AAAA", "A===", "AB==", "A", "AAAA!", "AA🦀AA"] {
            let mut tail = JsonlTail::with_inline_images(format);
            let batch = tail
                .feed(
                    0,
                    &lines(&[image_row(format, invalid)]),
                    TranscriptPhase::Live,
                )
                .unwrap();
            assert!(batch.loss && batch.records.is_empty(), "invalid {invalid}");
        }
        let raw = String::from_utf8(lines(&[image_row(format, "AAAA")]))
            .unwrap()
            .replace("AAAA", "AA\\u00xzAA");
        let mut tail = JsonlTail::with_inline_images(format);
        let batch = tail.feed(0, raw.as_bytes(), TranscriptPhase::Live).unwrap();
        assert!(batch.loss && batch.records.is_empty());
    }
    let raw = format!("{}0{}\n", "{\"nested\":".repeat(65), "}".repeat(65));
    let mut tail = JsonlTail::with_inline_images(InlineImageFormat::Claude);
    let batch = tail.feed(0, raw.as_bytes(), TranscriptPhase::Live).unwrap();
    assert!(batch.loss && batch.records.is_empty());
}

fn append(observer: &mut NativeTranscript, values: &[Value]) -> TranscriptBatch {
    observer
        .append(
            observer.next_offset(),
            &lines(values),
            TranscriptPhase::Live,
            10,
        )
        .unwrap()
}

#[test]
fn terminal_transcript_framing_survives_every_utf8_and_json_boundary() {
    let bytes = lines(&[cursor("user", "A crab 🦀"), cursor("assistant", "Ready")]);
    for split in 0..=bytes.len() {
        let mut tail = JsonlTail::default();
        let mut records = tail
            .feed(0, &bytes[..split], TranscriptPhase::Live)
            .unwrap()
            .records;
        records.extend(
            tail.feed(split as u64, &bytes[split..], TranscriptPhase::Live)
                .unwrap()
                .records,
        );
        assert_eq!(records.len(), 2, "split {split}");
        assert_eq!(
            records[0].value["message"]["content"][0]["text"],
            "A crab 🦀"
        );
        assert_eq!(records[0].ordinal, 0);
        assert_eq!(records[1].ordinal, 1);
    }
    let mut tail = JsonlTail::default();
    assert!(
        tail.feed(0, b"{\"role\":", TranscriptPhase::Replay)
            .unwrap()
            .partial
    );
    let next = tail.next_offset();
    let batch = tail
        .feed(next, b"\"user\"}\n", TranscriptPhase::Live)
        .unwrap();
    assert_eq!(batch.records[0].phase, TranscriptPhase::Replay);
}

#[test]
fn terminal_transcript_gaps_fail_without_advancing_and_oversize_stays_bounded() {
    let mut tail = JsonlTail::default();
    assert!(matches!(
        tail.feed(3, b"{}\n", TranscriptPhase::Live),
        Err(TranscriptError::OffsetMismatch {
            expected: 0,
            received: 3
        })
    ));
    assert_eq!(tail.next_offset(), 0);
    let oversized = vec![b'x'; TRANSCRIPT_LINE_LIMIT + 1];
    let batch = tail.feed(0, &oversized, TranscriptPhase::Live).unwrap();
    assert!(batch.loss && batch.partial);
    assert!(tail.pending.len() <= TRANSCRIPT_LINE_LIMIT);
    let batch = tail
        .feed(
            tail.next_offset(),
            b"\nnot json\n[]\n{\"valid\":true}\n",
            TranscriptPhase::Live,
        )
        .unwrap();
    assert!(batch.loss);
    assert!(!batch.partial);
    assert_eq!(batch.records.len(), 1);
    assert_eq!(batch.records[0].ordinal, 3);
}

#[test]
fn terminal_transcript_cursor_replay_and_duplicates_never_complete_a_live_turn() {
    let mut observer = observer(TranscriptFormat::Cursor);
    let initial = lines(&[
        cursor("user", "Old question"),
        cursor("assistant", "Old answer"),
        json!({"type":"turn_ended","status":"success"}),
    ]);
    let batch = observer
        .snapshot(&initial, TranscriptPhase::Live, 0)
        .unwrap();
    assert_eq!(batch.replay.len(), 2);
    assert!(batch.live.is_empty());
    assert!(!batch.replace);
    assert_eq!(
        observer
            .snapshot(&initial, TranscriptPhase::Live, 0)
            .unwrap(),
        TranscriptBatch::default()
    );
    let mut next = initial.clone();
    next.extend(lines(&[
        cursor("user", "New question"),
        cursor("assistant", "New answer"),
        json!({"type":"turn_ended","status":"success"}),
    ]));
    let batch = observer.snapshot(&next, TranscriptPhase::Live, 1).unwrap();
    assert_eq!(batch.live.len(), 3);
    assert_eq!(batch.live[2]["type"], "turn.completed");
    assert_eq!(batch.live[2]["outcome"], "ok");
    assert!(observer
        .snapshot(&next, TranscriptPhase::Live, 1)
        .unwrap()
        .live
        .is_empty());
}

#[test]
fn terminal_transcript_rewrites_replace_the_projection_including_same_length_edits() {
    let mut observer = observer(TranscriptFormat::Cursor);
    let old = lines(&[cursor("user", "Old"), cursor("assistant", "First")]);
    observer.snapshot(&old, TranscriptPhase::Replay, 0).unwrap();
    let replacement = lines(&[cursor("user", "New"), cursor("assistant", "Other")]);
    assert_eq!(old.len(), replacement.len());
    let batch = observer
        .snapshot(&replacement, TranscriptPhase::Live, 1)
        .unwrap();
    assert!(batch.replace);
    assert!(batch.live.is_empty());
    assert_eq!(batch.replay[0]["content"][0]["text"], "New");
    assert_eq!(batch.replay[1]["block"]["text"], "Other");
    observer.expect_prompt("Later").unwrap();
    let truncated = observer.snapshot(&[], TranscriptPhase::Live, 2).unwrap();
    assert!(truncated.replace);
    assert!(truncated.replay.is_empty() && truncated.live.is_empty());
    assert_eq!(
        append(&mut observer, &[cursor("user", "Later")]).live.len(),
        1
    );
}

#[test]
fn terminal_transcript_exact_prompt_echo_is_consumed_once_and_mismatch_expires_it() {
    let mut observer = observer(TranscriptFormat::Cursor);
    observer.expect_prompt("Repeat me").unwrap();
    assert!(append(&mut observer, &[cursor("user", "Repeat me")])
        .live
        .is_empty());
    assert_eq!(
        append(&mut observer, &[cursor("user", "Repeat me")])
            .live
            .len(),
        1
    );
    observer.expect_prompt("Later").unwrap();
    assert_eq!(
        append(&mut observer, &[cursor("user", "Typed in terminal")])
            .live
            .len(),
        1
    );
    assert_eq!(
        append(&mut observer, &[cursor("user", "Later")]).live.len(),
        1
    );
}

#[test]
fn terminal_transcript_codex_checks_native_identity_and_uses_explicit_turn_ids() {
    let mut observer = observer(TranscriptFormat::Codex);
    let batch = append(
        &mut observer,
        &[
            json!({"type":"session_meta","payload":{"id":"fixture-session"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1"}}),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"Question"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","id":"answer-1","content":[{"type":"output_text","text":"Answer"}]}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"Answer"}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"other"}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1","duration_ms":35}}),
        ],
    );
    assert_eq!(batch.live.len(), 4);
    assert_eq!(batch.live[0]["type"], "assistant.started");
    assert_eq!(batch.live[2]["block"]["text"], "Answer");
    assert_eq!(batch.live[3]["durationMs"], 35);
    assert!(append(
        &mut observer,
        &[json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}})]
    )
    .live
    .is_empty());

    let mut other = observer_for_mismatch();
    assert_eq!(
        other.append(
            0,
            &lines(&[json!({"type":"session_meta","payload":{"id":"wrong-session"}})]),
            TranscriptPhase::Live,
            0
        ),
        Err(TranscriptError::IdentityMismatch)
    );
    assert_eq!(
        other.append(
            other.next_offset(),
            &lines(&[
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Wrong file"}})
            ]),
            TranscriptPhase::Live,
            0
        ),
        Err(TranscriptError::IdentityMismatch)
    );
}

fn observer_for_mismatch() -> NativeTranscript {
    observer(TranscriptFormat::Codex)
}

#[test]
fn terminal_transcript_codex_tool_results_use_reported_outcomes_not_output_words() {
    let mut observer = observer(TranscriptFormat::Codex);
    let batch = append(
        &mut observer,
        &[
            json!({"type":"response_item","payload":{"type":"function_call","call_id":"call-1","name":"exec_command","arguments":"{\"cmd\":\"test\"}"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"call-1","output":"error: just a quoted word"}}),
            json!({"type":"event_msg","payload":{"type":"exec_command_end","call_id":"call-1","aggregated_output":"error: just a quoted word","exit_code":0}}),
            json!({"type":"event_msg","payload":{"type":"patch_apply_end","call_id":"patch-1","stdout":"","stderr":"Rejected patch","success":false}}),
            json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":150,"cached_input_tokens":100},"model_context_window":1000}}}),
        ],
    );
    assert_eq!(batch.live.len(), 4);
    assert_eq!(batch.live[0]["block"]["id"], "call-1");
    assert_eq!(batch.live[1]["toolId"], "call-1");
    assert_eq!(batch.live[1]["error"], false);
    assert_eq!(batch.live[2]["error"], true);
    assert_eq!(batch.live[3]["type"], "context.updated");
    assert_eq!(batch.live[3]["used"], 150);
}

#[test]
fn terminal_transcript_incomplete_or_lost_records_cannot_finish_a_turn() {
    let values = [
        cursor("assistant", "Working"),
        json!({"type":"turn_ended","status":"success"}),
    ];
    let mut partial = observer(TranscriptFormat::Cursor);
    let mut bytes = lines(&values);
    bytes.extend(b"{\"role\":");
    let batch = partial.append(0, &bytes, TranscriptPhase::Live, 0).unwrap();
    assert!(batch.partial);
    assert!(batch
        .live
        .iter()
        .all(|frame| frame["type"] != "turn.completed"));
    let mut damaged = observer(TranscriptFormat::Cursor);
    let mut bytes = b"not-json\n".to_vec();
    bytes.extend(lines(&values));
    let batch = damaged.append(0, &bytes, TranscriptPhase::Live, 0).unwrap();
    assert!(batch.loss);
    assert_eq!(batch.live.len(), 1);
    assert_eq!(batch.live[0]["type"], "assistant.block");
}

#[test]
fn terminal_transcript_antigravity_step_done_does_not_finish_the_conversation() {
    let mut observer = observer(TranscriptFormat::Antigravity);
    let batch = append(
        &mut observer,
        &[
            json!({"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"Inspect the file"}),
            json!({"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":"I will inspect it.","content":"Reading","tool_calls":[{"name":"read_file","args":{"path":"file.txt"}}]}),
            json!({"step_index":2,"source":"SYSTEM","type":"GENERIC","status":"DONE","content":"Tool output without a call id"}),
        ],
    );
    assert_eq!(batch.live.len(), 4);
    assert_eq!(batch.live[1]["block"]["kind"], "thinking");
    assert_eq!(batch.live[3]["block"]["name"], "read_file");
    assert!(batch
        .live
        .iter()
        .all(|frame| frame["type"] != "turn.completed" && frame["type"] != "tool.completed"));
    let truncated = append(
        &mut observer,
        &[
            json!({"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Truncated...","thinking":"Visible reasoning","truncated_fields":["content"]}),
        ],
    );
    assert!(truncated.loss);
    assert_eq!(truncated.live.len(), 1);
    assert_eq!(truncated.live[0]["block"]["text"], "Visible reasoning");
}

#[test]
fn terminal_transcript_cursor_reports_only_the_tool_evidence_present() {
    let mut observer = observer(TranscriptFormat::Cursor);
    let batch = append(
        &mut observer,
        &[
            json!({"role":"assistant","message":{"content":[
                {"type":"text","text":"Reading"},{"type":"tool_use","name":"Read","input":{"path":"file.txt"}}
            ]}}),
            json!({"role":"tool","message":{"content":[]}}),
        ],
    );
    assert_eq!(batch.live.len(), 2);
    assert_eq!(batch.live[1]["block"]["kind"], "tool");
    assert!(batch
        .live
        .iter()
        .all(|frame| frame["type"] != "tool.completed"));
}

#[test]
fn terminal_transcript_limits_are_explicit_and_do_not_leak_native_data_in_errors() {
    let mut observer = observer(TranscriptFormat::Cursor);
    assert_eq!(
        observer.expect_prompt(&"x".repeat(PROMPT_BYTES_LIMIT + 1)),
        Err(TranscriptError::PromptLimit)
    );
    assert!(observer.pending_prompts.is_empty());
    assert_eq!(
        observer.snapshot(
            &vec![0; TRANSCRIPT_SNAPSHOT_LIMIT + 1],
            TranscriptPhase::Live,
            0
        ),
        Err(TranscriptError::SizeLimit)
    );
    assert_eq!(observer.next_offset(), 0);
    let binding = TranscriptBinding {
        format: TranscriptFormat::Cursor,
        native_session: "fixture".into(),
        transcript_path: PathBuf::from("relative.jsonl"),
    };
    assert!(matches!(
        NativeTranscript::new(binding),
        Err(TranscriptError::InvalidBinding)
    ));
}
