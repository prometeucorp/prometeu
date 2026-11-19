use super::*;

fn projector() -> ExecutionProjector {
    ExecutionProjector::new("incarnation").unwrap()
}
fn report(id: Option<&str>, state: ProgramStatusState) -> ProgramStatusReport {
    ProgramStatusReport {
        id: id.map(str::to_owned),
        state,
        app: None,
        kind: None,
        progress: None,
        message: None,
        title: None,
    }
}
fn observe(
    projector: &mut ExecutionProjector,
    id: Option<&str>,
    state: ProgramStatusState,
) -> ExecutionProjection {
    projector.observe(report(id, state), 10)
}

#[test]
fn native_maintenance_settles_only_with_current_status_and_durable_source_without_completion() {
    let mut p = projector();
    // The PTY may report activity before the source identifies the maintenance input.
    observe(&mut p, None, ProgramStatusState::Working);
    let prior = observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .unwrap();
    p.accepted_maintenance();
    assert!(p.pending_completion().is_none());
    assert!(p
        .preview_completion(&prior, TranscriptBarrier::CaughtUp, 11)
        .is_none());
    assert!(p
        .settle_maintenance(TranscriptBarrier::Incomplete, 11)
        .is_none());
    let events = p
        .settle_maintenance(TranscriptBarrier::CaughtUp, 12)
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "execution.observed");
    assert_eq!(events[0]["state"], "idle");
    assert!(p
        .settle_maintenance(TranscriptBarrier::CaughtUp, 13)
        .is_none());
    p.accepted_maintenance();
    observe(&mut p, None, ProgramStatusState::Working);
    assert!(p
        .settle_maintenance(TranscriptBarrier::CaughtUp, 14)
        .is_none());
    observe(&mut p, Some("child"), ProgramStatusState::Working);
    assert!(observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .is_none());
    assert!(p
        .settle_maintenance(TranscriptBarrier::CaughtUp, 15)
        .is_none());
    observe(&mut p, Some("child"), ProgramStatusState::Done);
    p.accepted_input();
    assert!(p
        .settle_maintenance(TranscriptBarrier::CaughtUp, 16)
        .is_none());
    observe(&mut p, None, ProgramStatusState::Working);
    assert!(observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .is_some());
}

#[test]
fn observed_native_input_reopens_a_settled_cycle_and_source_claims_it_once() {
    for late_tool in [false, true] {
        let mut p = projector();
        p.accepted_input();
        observe(&mut p, None, ProgramStatusState::Working);
        p.observe_canonical(&json!({"type":"turn.completed","outcome":"interrupted"}));
        if late_tool {
            assert!(observe(&mut p, None, ProgramStatusState::Working)
                .events
                .is_empty());
        }
        let start = p.observed_input(11);
        assert_eq!(start.len(), 1);
        assert_eq!(start[0]["state"], "working");
        let activity = start[0]["activityId"].clone();
        assert!(p.captured_input(12).is_empty());
        let blocked = observe(&mut p, None, ProgramStatusState::Blocked);
        assert_eq!(blocked.events[0]["activityId"], activity);
        assert!(p.snapshot().has_active_work());
        assert!(observe(&mut p, None, ProgramStatusState::Done)
            .completion
            .is_some());
    }
}

#[test]
fn terminal_execution_interruption_is_authoritative_after_loss_but_not_repeated() {
    let mut p = projector();
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    observe(&mut p, None, ProgramStatusState::Clear);
    let interrupted = json!({"type":"turn.completed","outcome":"interrupted"});
    assert!(matches!(
        p.observe_canonical(&interrupted),
        CanonicalDisposition::Forward
    ));
    assert!(matches!(
        p.observe_canonical(&interrupted),
        CanonicalDisposition::DuplicateCompletion
    ));
    assert!(matches!(
        p.observe_canonical(&json!({"type":"turn.completed","outcome":"ok"})),
        CanonicalDisposition::DuplicateCompletion
    ));
    p.accepted_input();
    assert!(matches!(
        p.observe_canonical(&interrupted),
        CanonicalDisposition::Forward
    ));
}

#[test]
fn terminal_execution_late_first_input_claims_activity_without_losing_completion_evidence() {
    for input_after_done in [false, true] {
        let mut p = projector();
        let first = observe(&mut p, None, ProgramStatusState::Working);
        let activity = first.events[0]["activityId"].clone();
        if !input_after_done {
            p.observe_canonical(&json!({"type":"user.message"}));
        }
        let ticket = observe(&mut p, None, ProgramStatusState::Done)
            .completion
            .unwrap();
        if input_after_done {
            p.observe_canonical(&json!({"type":"user.message"}));
        }
        assert_eq!(p.activity(), activity.as_str().unwrap());
        assert_eq!(
            p.commit_completion(&ticket, TranscriptBarrier::CaughtUp, 20)
                .len(),
            1
        );
        p.observe_canonical(&json!({"type":"user.message"}));
        assert_ne!(p.activity(), activity.as_str().unwrap());
    }
}

#[test]
fn terminal_execution_resumed_activity_revokes_old_flush_ticket_even_in_same_cycle() {
    let mut p = projector();
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    let old = observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .unwrap();
    observe(&mut p, None, ProgramStatusState::Working);
    assert!(p.pending_completion().is_none());
    assert!(p
        .commit_completion(&old, TranscriptBarrier::CaughtUp, 12)
        .is_empty());
    let current = observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .unwrap();
    assert_ne!(old, current);
    assert!(p
        .commit_completion(&old, TranscriptBarrier::CaughtUp, 13)
        .is_empty());
    assert_eq!(
        p.commit_completion(&current, TranscriptBarrier::CaughtUp, 14)
            .len(),
        1
    );
}

#[test]
fn terminal_execution_active_root_idle_stops_quietly_until_new_input() {
    for active in [ProgramStatusState::Working, ProgramStatusState::Blocked] {
        let mut p = projector();
        observe(&mut p, None, ProgramStatusState::Idle);
        assert!(
            !p.root_activity_stopped(),
            "startup idle has no stopped activity"
        );
        p.accepted_input();
        observe(&mut p, None, active);
        let idle = observe(&mut p, None, ProgramStatusState::Idle);
        assert_eq!(idle.events.len(), 1);
        assert_eq!(idle.events[0]["type"], "execution.observed");
        assert_eq!(idle.events[0]["state"], "idle");
        assert!(idle.completion.is_none());
        assert!(p.root_activity_stopped());
        assert!(observe(&mut p, None, ProgramStatusState::Working)
            .events
            .is_empty());
        assert!(observe(&mut p, None, ProgramStatusState::Done)
            .completion
            .is_none());
        p.accepted_input();
        assert!(!p.root_activity_stopped());
        assert_eq!(
            observe(&mut p, None, ProgramStatusState::Working).events[0]["state"],
            "working"
        );
        assert!(observe(&mut p, None, ProgramStatusState::Done)
            .completion
            .is_some());
    }
}

#[test]
fn terminal_execution_done_then_idle_preserves_completion_awaiting_source_flush() {
    let mut p = projector();
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    let ticket = observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .unwrap();
    observe(&mut p, None, ProgramStatusState::Idle);
    assert!(!p.root_activity_stopped());
    assert!(p.root_content_idle());
    assert_eq!(p.pending_completion(), Some(ticket.clone()));
    assert!(p
        .commit_completion(&ticket, TranscriptBarrier::Incomplete, 11)
        .is_empty());
    let done = p.commit_completion(&ticket, TranscriptBarrier::CaughtUp, 12);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0]["outcome"], "ok");
}

#[test]
fn terminal_execution_done_idle_claims_delayed_source_but_new_input_opens_live_cycle() {
    for captured in [false, true] {
        let mut p = projector();
        observe(&mut p, None, ProgramStatusState::Working);
        let ticket = observe(&mut p, None, ProgramStatusState::Done)
            .completion
            .unwrap();
        observe(&mut p, None, ProgramStatusState::Idle);
        assert!(p.root_content_idle());
        if captured {
            let echoed = p.captured_input(11);
            assert_eq!(echoed[0]["state"], "idle");
            assert!(p.root_content_idle());
            assert_eq!(p.pending_completion(), Some(ticket.clone()));
        }
        p.accepted_input();
        assert!(
            !p.root_content_idle(),
            "a new app submission must not claim the idle provisional cycle"
        );
        assert!(p
            .preview_completion(&ticket, TranscriptBarrier::CaughtUp, 12)
            .is_none());
        assert_eq!(
            observe(&mut p, None, ProgramStatusState::Working).events[0]["state"],
            "working"
        );
    }
    let mut p = projector();
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    let ticket = observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .unwrap();
    observe(&mut p, None, ProgramStatusState::Idle);
    observe(&mut p, None, ProgramStatusState::Working);
    assert!(
        !p.root_content_idle(),
        "authoritative native continuation restores live content"
    );
    assert!(p
        .preview_completion(&ticket, TranscriptBarrier::CaughtUp, 12)
        .is_none());
}

#[test]
fn terminal_execution_root_idle_keeps_children_and_loss_remains_unknown() {
    let mut p = projector();
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    observe(&mut p, Some("worker"), ProgramStatusState::Working);
    let idle = observe(&mut p, None, ProgramStatusState::Idle);
    assert_eq!(idle.events[0]["state"], "idle");
    assert_eq!(idle.events[0]["scope"], "root");
    assert_eq!(idle.events[1]["state"], "working");
    assert_eq!(idle.events[1]["scope"], "background");
    assert!(p.snapshot().has_active_work());
    let lost = observe(&mut p, Some("worker"), ProgramStatusState::Clear);
    assert_eq!(lost.events[0]["state"], "unknown");
    assert!(
        p.root_activity_stopped(),
        "child loss cannot revive the stopped root's late content"
    );
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    let lost = observe(&mut p, None, ProgramStatusState::Clear);
    assert_eq!(lost.events[0]["state"], "unknown");
    assert!(!p.root_activity_stopped());
}

#[test]
fn terminal_execution_only_explicit_root_transition_and_flush_can_complete() {
    let mut p = projector();
    for state in [
        ProgramStatusState::Done,
        ProgramStatusState::Error,
        ProgramStatusState::Idle,
        ProgramStatusState::Clear,
    ] {
        assert!(observe(&mut p, None, state).completion.is_none());
    }
    p.accepted_input();
    let working = observe(&mut p, None, ProgramStatusState::Working);
    assert_eq!(working.events[0]["type"], "execution.observed");
    assert_eq!(working.events[0]["state"], "working");
    assert!(working
        .events
        .iter()
        .all(|frame| frame["type"] != "assistant.started"));
    let done = observe(&mut p, None, ProgramStatusState::Done);
    let ticket = done.completion.unwrap();
    assert!(done
        .events
        .iter()
        .all(|frame| frame["type"] != "turn.completed"));
    assert!(p
        .commit_completion(&ticket, TranscriptBarrier::Incomplete, 11)
        .is_empty());
    let completed = p.commit_completion(&ticket, TranscriptBarrier::CaughtUp, 12);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0]["outcome"], "ok");
    assert!(completed[0].get("messageId").is_none());
    assert!(p
        .commit_completion(&ticket, TranscriptBarrier::CaughtUp, 13)
        .is_empty());
}

#[test]
fn terminal_execution_canonical_completion_cancels_ticket_and_stale_status_cannot_rearm() {
    let mut p = projector();
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    let ticket = observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .unwrap();
    let canonical = json!({"type":"turn.completed","outcome":"ok"});
    assert!(matches!(
        p.observe_canonical(&canonical),
        CanonicalDisposition::Forward
    ));
    assert!(p
        .commit_completion(&ticket, TranscriptBarrier::CaughtUp, 12)
        .is_empty());
    assert!(observe(&mut p, None, ProgramStatusState::Working)
        .events
        .is_empty());
    assert!(observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .is_none());
    assert!(matches!(
        p.observe_canonical(&canonical),
        CanonicalDisposition::DuplicateCompletion
    ));
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    assert!(observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .is_some());
}

#[test]
fn terminal_execution_child_activity_never_claims_primary_completion() {
    let mut p = projector();
    p.accepted_input();
    observe(&mut p, None, ProgramStatusState::Working);
    let child = observe(&mut p, Some("worker"), ProgramStatusState::Working);
    assert_eq!(child.events[0]["type"], "background.changed");
    let ticket = observe(&mut p, None, ProgramStatusState::Done)
        .completion
        .unwrap();
    let completed = p.commit_completion(&ticket, TranscriptBarrier::CaughtUp, 12);
    assert_eq!(completed.len(), 1);
    let blocked = p.observe_fact(
        ExecutionFact::Blocked {
            id: Some("worker".into()),
            kind: Some(ProgramStatusKind::Question),
            detail: Some("Choose a target".into()),
        },
        13,
    );
    let state = blocked
        .events
        .iter()
        .find(|frame| frame["type"] == "execution.observed")
        .unwrap();
    assert_eq!(state["scope"], "background");
    assert_eq!(state["state"], "blocked");
    assert!(state["blockId"].is_string());
    assert!(blocked
        .events
        .iter()
        .all(|frame| frame["type"] != "request.opened"));
    let working = observe(&mut p, Some("worker"), ProgramStatusState::Working);
    assert!(working
        .events
        .iter()
        .any(|frame| frame["state"] == "working" && frame["scope"] == "background"));
    let child_done = observe(&mut p, Some("worker"), ProgramStatusState::Done);
    assert!(child_done.completion.is_none());
    assert_eq!(child_done.events[0]["tasks"], json!([]));
}

#[test]
fn terminal_execution_unknown_precedes_ambiguous_child_drain_and_revokes_completion() {
    for state in [ProgramStatusState::Clear, ProgramStatusState::Idle] {
        let mut p = projector();
        p.accepted_input();
        observe(&mut p, None, ProgramStatusState::Working);
        observe(&mut p, Some("worker"), ProgramStatusState::Working);
        let ticket = observe(&mut p, None, ProgramStatusState::Done)
            .completion
            .unwrap();
        let cleared = observe(&mut p, Some("worker"), state);
        assert_eq!(cleared.events[0]["state"], "unknown");
        assert_eq!(cleared.events[1]["type"], "background.changed");
        assert!(p
            .commit_completion(&ticket, TranscriptBarrier::CaughtUp, 15)
            .is_empty());
        assert!(observe(&mut p, None, ProgramStatusState::Working)
            .events
            .is_empty());
    }
}

#[test]
fn terminal_execution_duplicate_blocked_reports_keep_one_episode_and_stop_is_not_final() {
    let mut p = projector();
    let blocked = || ExecutionFact::Blocked {
        id: None,
        kind: Some(ProgramStatusKind::Permission),
        detail: Some("Allow this action".into()),
    };
    let first = p.observe_fact(blocked(), 1);
    let id = first.events[0]["blockId"].clone();
    assert!(id.is_string());
    assert!(p.observe_fact(blocked(), 2).events.is_empty());
    assert!(p
        .observe_fact(ExecutionFact::StopObserved, 3)
        .completion
        .is_none());
    assert!(p.pending_completion().is_none());
    observe(&mut p, None, ProgramStatusState::Working);
    let second = p.observe_fact(blocked(), 4);
    assert_ne!(second.events[0]["blockId"], id);
}

#[test]
fn terminal_execution_merges_native_background_without_erasing_other_sources() {
    let mut p = projector();
    observe(&mut p, Some("worker"), ProgramStatusState::Working);
    let frame = json!({"v":1,"type":"background.changed","at":2,"tasks":[{"id":"provider-task","description":"Native task","toolId":null}]});
    let CanonicalDisposition::Replace(merged) = p.observe_canonical(&frame) else {
        panic!("merged snapshot");
    };
    assert_eq!(merged["tasks"].as_array().unwrap().len(), 2);
    let child_done = observe(&mut p, Some("worker"), ProgramStatusState::Done);
    assert_eq!(child_done.events[0]["tasks"], frame["tasks"]);
    let retired = p.retire(3);
    assert_eq!(retired[0]["state"], "unknown");
    assert_eq!(retired[1]["tasks"], json!([]));
    assert!(retired
        .iter()
        .all(|frame| frame["type"] != "turn.completed"));
}

#[test]
fn terminal_execution_bounds_records_and_active_eviction_is_unknown_not_completion() {
    assert!(ExecutionProjector::new("").is_none());
    let mut p = projector();
    for index in 0..256 {
        observe(
            &mut p,
            Some(&format!("child-{index}")),
            ProgramStatusState::Working,
        );
    }
    let overflow = observe(&mut p, Some("new-child"), ProgramStatusState::Working);
    assert_eq!(p.snapshot().children.len(), 256);
    assert_eq!(overflow.events[0]["state"], "unknown");
    assert!(overflow.completion.is_none());
}
