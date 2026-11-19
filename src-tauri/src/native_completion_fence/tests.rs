use super::*;
use serde_json::json;

fn fence(provider: ProviderId) -> NativeCompletionFence {
    let version = match provider {
        ProviderId::Claude => "2.1.326 (Claude Code)",
        ProviderId::Antigravity => "1.5.10",
        _ => unreachable!(),
    };
    let mut fence = NativeCompletionFence::verified(provider, version, "process").unwrap();
    assert!(fence.begin(1, "native", watermark(provider)));
    if provider == ProviderId::Antigravity {
        let mut event = hook(provider, NativeHookKind::ModelInvocationStarted);
        event.initial_num_steps = Some(5);
        fence.observe_hook(1, &event);
    }
    fence
}

fn watermark(provider: ProviderId) -> SourceWatermark {
    match provider {
        ProviderId::Claude => SourceWatermark::Claude {
            revision: "revision".into(),
            next_offset: 100,
        },
        ProviderId::Antigravity => SourceWatermark::Antigravity { next_step_index: 5 },
        _ => unreachable!(),
    }
}

fn status(state: ProgramStatusState) -> ProgramStatusReport {
    ProgramStatusReport {
        id: None,
        state,
        kind: None,
        message: None,
        app: None,
        title: None,
        progress: None,
    }
}

fn hook(provider: ProviderId, kind: NativeHookKind) -> NativeHookEvent {
    serde_json::from_value(json!({
        "provider":provider, "session":"app", "process_nonce":"process", "native_session":"native",
        "kind":kind, "text":"Same response", "fully_idle":true, "reason":"NO_TOOL_CALL", "execution_number":0
    }))
    .unwrap()
}

fn record(provider: ProviderId, position: u64, id: &str) -> TranscriptRecord {
    let value = match provider {
        ProviderId::Claude => json!({"type":"assistant","sessionId":"native","uuid":id,
            "message":{"id":format!("api-{id}"),"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"Same response"}]}}),
        ProviderId::Antigravity => {
            json!({"step_index":position,"source":"MODEL","type":"PLANNER_RESPONSE",
            "status":"DONE","content":"Same response","truncated_fields":[],"tool_calls":[]})
        }
        _ => unreachable!(),
    };
    TranscriptRecord {
        ordinal: 0,
        offset: position,
        phase: TranscriptPhase::Live,
        value,
        omitted_media: 0,
    }
}

fn caught_up(fence: &mut NativeCompletionFence, generation: u64) {
    let poll = fence.start_poll(generation);
    fence.finish_poll(poll, true);
}

#[test]
fn completion_requires_current_persisted_response_stop_and_post_done_poll() {
    let mut fence = fence(ProviderId::Claude);
    fence.observe_psp(1, &status(ProgramStatusState::Working));
    fence.observe_hook(1, &hook(ProviderId::Claude, NativeHookKind::StopObserved));
    fence.observe_record(1, "revision", &record(ProviderId::Claude, 100, "message-1"));
    let early = fence.start_poll(1);
    fence.observe_psp(1, &status(ProgramStatusState::Done));
    fence.finish_poll(early, true);
    assert!(fence.take_completion(1).is_none());
    let partial = fence.start_poll(1);
    fence.finish_poll(partial, false);
    assert!(fence.take_completion(1).is_none());
    caught_up(&mut fence, 1);
    assert_eq!(
        fence.take_completion(1),
        Some(PersistedCompletion {
            generation: 1,
            native_source: "message-1".into()
        })
    );
    assert!(fence.take_completion(1).is_none());
}

#[test]
fn repeated_identical_response_before_watermark_or_from_replay_cannot_complete() {
    let mut fence = fence(ProviderId::Claude);
    fence.observe_psp(1, &status(ProgramStatusState::Working));
    fence.observe_hook(1, &hook(ProviderId::Claude, NativeHookKind::StopObserved));
    fence.observe_record(
        1,
        "revision",
        &record(ProviderId::Claude, 99, "old-message"),
    );
    let mut replay = record(ProviderId::Claude, 100, "replayed-message");
    replay.phase = TranscriptPhase::Replay;
    fence.observe_record(1, "revision", &replay);
    fence.observe_psp(1, &status(ProgramStatusState::Done));
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    fence.observe_record(
        1,
        "revision",
        &record(ProviderId::Claude, 100, "new-message"),
    );
    assert!(fence.take_completion(1).is_none());
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_some());
}

#[test]
fn stop_continuation_needs_another_new_source_even_when_response_is_identical() {
    let mut fence = fence(ProviderId::Claude);
    let stopped = hook(ProviderId::Claude, NativeHookKind::StopObserved);
    fence.observe_psp(1, &status(ProgramStatusState::Working));
    fence.observe_record(1, "revision", &record(ProviderId::Claude, 100, "first"));
    fence.observe_hook(1, &stopped);
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    fence.observe_hook(1, &stopped);
    fence.observe_psp(1, &status(ProgramStatusState::Done));
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    let mut another_block = record(ProviderId::Claude, 140, "another-block");
    another_block.value["message"]["id"] = json!("api-first");
    fence.observe_record(1, "revision", &another_block);
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    fence.observe_record(1, "revision", &record(ProviderId::Claude, 150, "second"));
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_some());
}

#[test]
fn cancel_new_input_and_source_replacement_invalidate_pending_proofs() {
    for final_state in [
        ProgramStatusState::Idle,
        ProgramStatusState::Error,
        ProgramStatusState::Clear,
    ] {
        let mut fence = fence(ProviderId::Claude);
        fence.observe_psp(1, &status(ProgramStatusState::Working));
        fence.observe_hook(1, &hook(ProviderId::Claude, NativeHookKind::StopObserved));
        fence.observe_record(1, "revision", &record(ProviderId::Claude, 100, "first"));
        fence.observe_psp(1, &status(final_state));
        fence.observe_psp(1, &status(ProgramStatusState::Done));
        caught_up(&mut fence, 1);
        assert!(fence.take_completion(1).is_none());
    }
    let mut fence = fence(ProviderId::Claude);
    fence.observe_psp(1, &status(ProgramStatusState::Working));
    fence.observe_hook(1, &hook(ProviderId::Claude, NativeHookKind::StopObserved));
    fence.observe_psp(1, &status(ProgramStatusState::Done));
    let stale_poll = fence.start_poll(1);
    assert!(fence.begin(
        2,
        "native",
        SourceWatermark::Claude {
            revision: "revision".into(),
            next_offset: 200
        }
    ));
    fence.observe_record(1, "revision", &record(ProviderId::Claude, 100, "old"));
    fence.finish_poll(stale_poll, true);
    assert!(fence.take_completion(1).is_none());
    assert!(fence.take_completion(2).is_none());
    fence.observe_record(2, "replaced", &record(ProviderId::Claude, 220, "new"));
    fence.observe_psp(2, &status(ProgramStatusState::Working));
    fence.observe_psp(2, &status(ProgramStatusState::Done));
    fence.observe_hook(2, &hook(ProviderId::Claude, NativeHookKind::StopObserved));
    caught_up(&mut fence, 2);
    assert!(fence.take_completion(2).is_none());
}

#[test]
fn antigravity_uses_native_step_floor_and_rejects_partial_or_truncated_content() {
    let mut fence = fence(ProviderId::Antigravity);
    let mut invocation = hook(
        ProviderId::Antigravity,
        NativeHookKind::ModelInvocationStarted,
    );
    invocation.initial_num_steps = Some(8);
    fence.observe_hook(1, &invocation);
    fence.observe_psp(1, &status(ProgramStatusState::Working));
    fence.observe_hook(
        1,
        &hook(ProviderId::Antigravity, NativeHookKind::StopObserved),
    );
    fence.observe_psp(1, &status(ProgramStatusState::Done));
    fence.observe_record(1, "revision", &record(ProviderId::Antigravity, 7, "unused"));
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    let mut response = record(ProviderId::Antigravity, 8, "unused");
    response.value["truncated_fields"] = json!(["thinking"]);
    fence.observe_record(1, "revision", &response);
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    response.value["truncated_fields"] = json!([]);
    response.value["status"] = json!("IN_PROGRESS");
    fence.observe_record(1, "revision", &response);
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    response.value["status"] = json!("DONE");
    // Native snapshot rewriting changes projection phase, not the trajectory step identity.
    response.phase = TranscriptPhase::Replay;
    fence.observe_record(1, "revision", &response);
    caught_up(&mut fence, 1);
    assert_eq!(fence.take_completion(1).unwrap().native_source, "step:8");
}

#[test]
fn child_stale_or_incomplete_stop_cannot_supply_root_success() {
    for mode in ["child", "process", "session", "background", "error"] {
        let mut fence = fence(ProviderId::Antigravity);
        fence.observe_psp(1, &status(ProgramStatusState::Working));
        fence.observe_record(1, "revision", &record(ProviderId::Antigravity, 5, "unused"));
        let mut stopped = hook(ProviderId::Antigravity, NativeHookKind::StopObserved);
        match mode {
            "child" => stopped.agent_id = Some("child".into()),
            "process" => stopped.process_nonce = "old-process".into(),
            "session" => stopped.native_session = "other-native".into(),
            "background" => stopped.fully_idle = Some(false),
            "error" => stopped.error = Some(json!("Native error")),
            _ => unreachable!(),
        }
        fence.observe_hook(1, &stopped);
        fence.observe_psp(1, &status(ProgramStatusState::Done));
        caught_up(&mut fence, 1);
        assert!(fence.take_completion(1).is_none(), "{mode}");
    }
}

#[test]
fn exact_native_text_normalization_and_verified_versions_are_required() {
    let mut fence = fence(ProviderId::Claude);
    let mut response = record(ProviderId::Claude, 100, "exact");
    response.value["message"]["content"] = json!([
        {"type":"text","text":"\u{feff} First"}, {"type":"thinking","thinking":"private"},
        {"type":"text","text":"Second\u{00a0}"}
    ]);
    let mut stopped = hook(ProviderId::Claude, NativeHookKind::StopObserved);
    stopped.text = Some("First\nSecond".into());
    stopped.stop_hook_active = Some(true);
    fence.observe_psp(1, &status(ProgramStatusState::Working));
    fence.observe_hook(1, &stopped);
    fence.observe_record(1, "revision", &response);
    fence.observe_psp(1, &status(ProgramStatusState::Done));
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_some());
    assert!(NativeCompletionFence::verified(ProviderId::Claude, "unknown", "process").is_none());
    assert!(NativeCompletionFence::verified(ProviderId::Antigravity, "1.5.9", "process").is_none());
    assert!(
        NativeCompletionFence::verified(ProviderId::Cursor, "2026.11.13-eacade1", "process")
            .is_none()
    );
}

#[test]
fn antigravity_same_execution_can_be_observed_again_after_background_work_finishes() {
    let mut fence = fence(ProviderId::Antigravity);
    fence.observe_psp(1, &status(ProgramStatusState::Working));
    fence.observe_record(1, "revision", &record(ProviderId::Antigravity, 5, "unused"));
    let mut stopped = hook(ProviderId::Antigravity, NativeHookKind::StopObserved);
    stopped.fully_idle = Some(false);
    fence.observe_hook(1, &stopped);
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_none());
    stopped.fully_idle = Some(true);
    fence.observe_hook(1, &stopped);
    fence.observe_psp(1, &status(ProgramStatusState::Done));
    caught_up(&mut fence, 1);
    assert!(fence.take_completion(1).is_some());
}

#[test]
fn antigravity_external_stop_without_text_requires_the_exact_last_invocation_step() {
    for counterexample in ["old-step", "later-step", "tool-call", "continued", "valid"] {
        let mut fence = fence(ProviderId::Antigravity);
        fence.observe_psp(1, &status(ProgramStatusState::Working));
        let mut response = record(ProviderId::Antigravity, 5, "unused");
        match counterexample {
            "old-step" => response.value["step_index"] = json!(4),
            "later-step" => response.value["step_index"] = json!(6),
            "tool-call" => response.value["tool_calls"] = json!([{"name":"read_file","args":{}}]),
            _ => {}
        }
        fence.observe_record(1, "revision", &response);
        let mut stopped = hook(ProviderId::Antigravity, NativeHookKind::StopObserved);
        stopped.text = None;
        fence.observe_hook(1, &stopped);
        if counterexample == "continued" {
            let mut next = hook(
                ProviderId::Antigravity,
                NativeHookKind::ModelInvocationStarted,
            );
            next.initial_num_steps = Some(6);
            fence.observe_hook(1, &next);
        }
        fence.observe_psp(1, &status(ProgramStatusState::Done));
        caught_up(&mut fence, 1);
        assert_eq!(
            fence.take_completion(1).is_some(),
            counterexample == "valid"
        );
    }
}
