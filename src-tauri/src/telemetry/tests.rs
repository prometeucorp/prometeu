use super::*;
use serde_json::json;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("prometeu-telemetry-{}", id())))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn scope() -> Scope {
    Scope {
        workspace_id: Some(id()),
        conversation_id: Some(id()),
        provider: Some("claude".into()),
        ..Scope::default()
    }
}

#[test]
fn change_notification_follows_committed_capture_and_releases_its_locks() {
    let dir = Temp::new();
    let service = Mutex::new(Service::new(dir.0.clone()));
    for (index, answer) in [Some("PRIVATE GENERATED TITLE"), None]
        .into_iter()
        .enumerate()
    {
        let mut notifications = 0;
        let returned = after_capture(
            || {
                let mut service = lock(&service);
                for _ in 0..2 {
                    service.capture(0, &event(&scope(), 1, Fact::ConversationCreated {}));
                }
                answer
            },
            |name, payload| {
                notifications += 1;
                assert_eq!(name, "telemetry-changed");
                assert_eq!(
                    serde_json::to_value(payload).unwrap(),
                    serde_json::Value::Null
                );
                let queries = service
                    .try_lock()
                    .expect("capture lock must be released before notification")
                    .queries();
                // The callback may immediately reopen SQLite, just as a view refresh does.
                assert_eq!(
                    queries.summary(&Filter::default()).unwrap().events,
                    ((index + 1) * 2) as u64
                );
            },
        );
        assert_eq!(returned, answer);
        assert_eq!(
            notifications, 1,
            "one notification follows the whole capture, including a missing title"
        );
    }
}

#[test]
fn completed_replies_keep_a_private_lookup_anchor() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let mut capture = Capture::default();
    let scope = scope();
    let conversation = scope.conversation_id.clone().unwrap();
    capture.accepted(&mut service, scope, None);
    capture.observe(
        &mut service,
        &json!({
            "type":"turn.completed", "outcome":"ok", "messageId":"PRIVATE NATIVE REPLY",
            "message":"PRIVATE RESPONSE", "providerDurationMs":123,
            "telemetry":{"usageScope":"mainAgent","complete":true,
                "selectedModel":null,"observedModels":null,"usageByModel":null,
                "usage":{"inputTokens":100,"outputTokens":20}}
        }),
    );
    let exported = service.queries().export(&Filter::default()).unwrap();
    let completion = exported
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["type"] == "turn.completed")
        .unwrap();
    assert_eq!(
        completion["payload"]["messageKey"].as_str().map(str::len),
        Some(64)
    );
    assert!(!exported.contains("PRIVATE"));
    let rows = service
        .queries()
        .turns(
            &conversation,
            &[
                "unknown".into(),
                "PRIVATE NATIVE REPLY".into(),
                "PRIVATE NATIVE REPLY".into(),
            ],
        )
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].duration_ms, Some(123));
    assert_eq!(rows[0].usage.usage.input_tokens, Some(100));
    assert!(service
        .queries()
        .turns(&id(), &["PRIVATE NATIVE REPLY".into()])
        .unwrap()
        .is_empty());
    assert!(service
        .queries()
        .turns(&conversation, &vec!["id".into(); 501])
        .is_err());
    service.clear().unwrap();
    assert!(service
        .queries()
        .turns(&conversation, &["PRIVATE NATIVE REPLY".into()])
        .unwrap()
        .is_empty());
}

#[test]
fn insights_group_final_measurements_without_inventing_models_or_context_totals() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let store = service.store.as_mut().unwrap();
    let mut scope = scope();
    let conversation = scope.conversation_id.clone().unwrap();
    for (index, provider, input, output) in [
        (0, "claude", Some(100), Some(20)),
        (1, "codex", None, Some(30)),
    ] {
        scope.turn_id = Some(id());
        scope.provider = Some(provider.into());
        let at = 10 + index * 10;
        append(
            store,
            &scope,
            at,
            Fact::TurnStarted {
                origin: Origin {
                    action_id: (index == 0).then(id),
                    delegated_by: (index == 0).then(id),
                    ..Default::default()
                },
                measurement: Measurement {
                    selected_model: Some("selected-only".into()),
                    ..Default::default()
                },
            },
        );
        append(
            store,
            &scope,
            at + 1,
            Fact::UsageObserved {
                measurement: Measurement {
                    usage: Usage {
                        input_tokens: Some(10),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        );
        append(
            store,
            &scope,
            at + 2,
            Fact::TurnCompleted {
                outcome: Outcome::Ok,
                elapsed_ms: 2,
                provider_duration_ms: None,
                message_key: None,
                measurement: Measurement {
                    complete: input.is_some(),
                    selected_model: Some("selected-only".into()),
                    usage: Usage {
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_tokens: input.map(|_| 80),
                        context_used: Some(800),
                        context_window: Some(1000),
                        peak_context: Some(90 + index * 30),
                        cost_usd: input.map(|_| 0.25),
                        ..Default::default()
                    },
                    usage_by_model: input.map(|_| {
                        vec![ModelUsage {
                            model: "observed".into(),
                            usage: Usage {
                                input_tokens: Some(100),
                                ..Default::default()
                            },
                        }]
                    }),
                    ..Default::default()
                },
            },
        );
    }
    scope.turn_id = None;
    scope.conversation_id = None;
    scope.provider = Some("claude".into());
    let call = id();
    append(
        store,
        &scope,
        15,
        Fact::AppCallStarted {
            call_id: call.clone(),
            source: AppSource::Naming,
            measurement: Measurement::default(),
        },
    );
    append(
        store,
        &scope,
        100,
        Fact::AppCallCompleted {
            call_id: call,
            source: AppSource::Naming,
            outcome: Outcome::Ok,
            elapsed_ms: 85,
            measurement: Measurement {
                usage: Usage {
                    input_tokens: Some(4),
                    output_tokens: Some(2),
                    cost_usd: Some(0.0),
                    ..Default::default()
                },
                ..Default::default()
            },
        },
    );
    let result = service
        .queries()
        .insights(&Filter {
            workspace_id: scope.workspace_id.clone(),
            from: Some(0),
            to: Some(30),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result.summary.turns, 2);
    assert_eq!(result.summary.input_tokens, Some(100));
    assert_eq!(
        (
            result.usage.input_tokens,
            result.usage.output_tokens,
            result.usage.cache_read_tokens
        ),
        (Some(104), Some(52), Some(80))
    );
    assert_eq!(result.usage.cost_usd, Some(0.25));
    assert_eq!(
        (
            result.usage.context_used,
            result.usage.context_window,
            result.usage.peak_context
        ),
        (None, None, Some(120))
    );
    assert_eq!(result.conversations.len(), 1);
    assert_eq!(result.conversations[0].id, conversation);
    assert_eq!(result.conversations[0].provider, None);
    assert_eq!(result.conversations[0].turns, 2);
    assert_eq!(result.models.len(), 1);
    assert_eq!(result.models[0].id, "observed");
    assert_eq!(result.models[0].usage.output_tokens, None);
    assert_eq!(
        result
            .sources
            .iter()
            .find(|g| g.id == "naming")
            .unwrap()
            .turns,
        1
    );
    assert_eq!(result.origins.len(), 2);
    assert!(result
        .origins
        .iter()
        .all(|group| group.turns == 1 && group.usage.input_tokens == Some(100)));
    assert_eq!(
        service
            .queries()
            .insights(&Filter {
                from: Some(200),
                ..Default::default()
            })
            .unwrap()
            .usage
            .input_tokens,
        None
    );
}

#[test]
fn app_overhead_is_global_or_scoped_and_late_callbacks_cannot_restore_erased_history() {
    let dir = Temp::new();
    let service = Mutex::new(Service::new(dir.0.clone()));
    let mut capture = AppCapture::start(
        &service,
        0,
        Scope {
            provider: Some("claude".into()),
            ..Default::default()
        },
        AppSource::PluginMaker,
        Some("sonnet".into()),
    );
    let mut adapter = crate::claude::Adapter::fresh();
    for frame in adapter.translate(&json!({"type":"result","uuid":"PRIVATE RESULT","session_id":"PRIVATE SESSION", "result":"PRIVATE REPLY", "total_cost_usd":0.02,
        "usage":{"input_tokens":2,"cache_read_input_tokens":8,"cache_creation_input_tokens":0,"output_tokens":4}})) { capture.observe(&frame); }
    capture.finish(&service, true);
    let queries = lock(&service).queries();
    let result = queries.insights(&Filter::default()).unwrap();
    assert_eq!(result.summary.turns, 0);
    assert_eq!(result.usage.input_tokens, Some(10));
    assert_eq!(result.usage.cost_usd, Some(0.02));
    assert_eq!(result.sources[0].id, "plugin-maker");
    assert!(result.conversations.is_empty());
    assert!(!queries
        .export(&Filter::default())
        .unwrap()
        .contains("PRIVATE"));
    assert_eq!(
        queries
            .insights(&Filter {
                workspace_id: Some(id()),
                ..Default::default()
            })
            .unwrap()
            .usage
            .input_tokens,
        None
    );
    let pending = AppCapture::start(&service, 0, Scope::default(), AppSource::Naming, None);
    lock(&service).clear().unwrap();
    pending.finish(&service, true);
    assert_eq!(
        lock(&service)
            .queries()
            .insights(&Filter::default())
            .unwrap()
            .summary
            .events,
        0
    );
}

#[test]
fn old_turn_payloads_default_new_attribution_and_reply_fields() {
    let start:Fact=serde_json::from_value(json!({"type":"turn.started","payload":{"measurement":{"usageScope":"mainAgent","usage":{}}}})).unwrap();
    assert!(
        matches!(start,Fact::TurnStarted{origin:Origin{action_id:None,delegated_by:None,repositories},..} if repositories.is_empty())
    );
    let end:Fact=serde_json::from_value(json!({"type":"turn.completed","payload":{"outcome":"ok","elapsedMs":0,"measurement":{"usageScope":"mainAgent","usage":{}}}})).unwrap();
    assert!(matches!(
        end,
        Fact::TurnCompleted {
            message_key: None,
            ..
        }
    ));
}

#[test]
fn tenure_uses_complete_lifecycle_evidence_and_keeps_unknown_relations_unallocated() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let store = service.store.as_mut().unwrap();
    let mut scope = scope();
    let repository = id();
    let branch = id();
    for at in [10, 50, 100] {
        scope.turn_id = Some(id());
        append(
            store,
            &scope,
            at,
            Fact::TurnStarted {
                measurement: Measurement::default(),
                origin: Origin {
                    repositories: vec![RepositoryBranch {
                        repository_id: repository.clone(),
                        branch_id: branch.clone(),
                    }],
                    ..Default::default()
                },
            },
        );
        append(
            store,
            &scope,
            at + 1,
            Fact::TurnCompleted {
                outcome: Outcome::Ok,
                elapsed_ms: 1,
                provider_duration_ms: None,
                message_key: None,
                measurement: Measurement {
                    usage: Usage {
                        input_tokens: Some(10),
                        output_tokens: Some(0),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        );
    }
    let snapshot = id();
    for (number, created, closed, state) in [
        (1, 20, Some(50), PullRequestState::Closed),
        (2, 80, None, PullRequestState::Open),
    ] {
        append(
            store,
            &scope,
            200,
            Fact::PullRequestObserved {
                repository_id: repository.clone(),
                branch_id: branch.clone(),
                pull_request: number,
                state,
                created_at: Some(created),
                closed_at: closed,
                merged_at: None,
                history_complete: true,
                snapshot_id: snapshot.clone(),
                history_size: 2,
                observed_after: 150,
            },
        );
    }
    let result = service.queries().insights(&Filter::default()).unwrap();
    assert_eq!(result.pull_requests[0].turns, Some(1));
    assert_eq!(result.pull_requests[1].turns, Some(2));
    assert_eq!(result.pull_requests[1].usage.input_tokens, Some(20));
    // One persisted row of a two-candidate snapshot cannot make a partial write authoritative.
    let store = service.store.as_mut().unwrap();
    append(
        store,
        &scope,
        300,
        Fact::PullRequestObserved {
            repository_id: repository.clone(),
            branch_id: branch.clone(),
            pull_request: 2,
            state: PullRequestState::Open,
            created_at: Some(80),
            closed_at: None,
            merged_at: None,
            history_complete: true,
            snapshot_id: id(),
            history_size: 2,
            observed_after: 250,
        },
    );
    let result = service.queries().insights(&Filter::default()).unwrap();
    assert!(result
        .pull_requests
        .iter()
        .all(|pr| pr.attribution == "related"
            && pr.turns.is_none()
            && pr.usage.input_tokens.is_none()));
}

#[test]
fn tenure_never_extrapolates_open_state_or_missing_branch_history() {
    for (label, complete, created, closed, observed_after, correct_branch, state) in [
        (
            "truncated",
            false,
            Some(20),
            None,
            100,
            true,
            PullRequestState::Open,
        ),
        (
            "missing creation",
            true,
            None,
            None,
            100,
            true,
            PullRequestState::Open,
        ),
        (
            "missing closure",
            true,
            Some(20),
            None,
            100,
            true,
            PullRequestState::Closed,
        ),
        (
            "stale open state",
            true,
            Some(20),
            None,
            49,
            true,
            PullRequestState::Open,
        ),
        (
            "different branch",
            true,
            Some(20),
            None,
            100,
            false,
            PullRequestState::Open,
        ),
        (
            "closed at start",
            true,
            Some(20),
            Some(50),
            100,
            true,
            PullRequestState::Closed,
        ),
    ] {
        let dir = Temp::new();
        let mut service = Service::new(dir.0.clone());
        let store = service.store.as_mut().unwrap();
        let mut scope = scope();
        scope.turn_id = Some(id());
        let repository = id();
        let branch = id();
        append(
            store,
            &scope,
            50,
            Fact::TurnStarted {
                measurement: Measurement::default(),
                origin: Origin {
                    repositories: vec![RepositoryBranch {
                        repository_id: repository.clone(),
                        branch_id: branch.clone(),
                    }],
                    ..Default::default()
                },
            },
        );
        append(
            store,
            &scope,
            200,
            Fact::PullRequestObserved {
                repository_id: repository,
                branch_id: if correct_branch { branch } else { id() },
                pull_request: 1,
                state,
                created_at: created,
                closed_at: closed,
                merged_at: None,
                history_complete: complete,
                snapshot_id: id(),
                history_size: 1,
                observed_after,
            },
        );
        let result = service.queries().insights(&Filter::default()).unwrap();
        assert_eq!(result.pull_requests[0].attribution, "related", "{label}");
        assert_eq!(result.pull_requests[0].turns, None, "{label}");
    }
}
fn event(scope: &Scope, at: u64, fact: Fact) -> Event {
    let mut e = Event::new(scope.clone(), id(), fact);
    e.occurred_at = at;
    e
}
fn append(store: &mut Store, scope: &Scope, at: u64, fact: Fact) {
    store.append(&event(scope, at, fact)).unwrap();
}
#[test]
fn commits_deduplicates_reopens_and_preserves_future_versions() {
    let dir = Temp::new();
    let path = dir.0.join("telemetry.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let e = event(&scope(), 10, Fact::ConversationCreated {});
    store.append(&e).unwrap();
    store.append(&e).unwrap();
    assert_eq!(
        serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(),
        e
    );
    let mut conflict = e.clone();
    conflict.fact = Fact::WorkspaceArchived {};
    assert!(store.append(&conflict).is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .page(&Filter::default(), None, Health::default())
            .unwrap()
            .events
            .len(),
        1
    );
    store.db.pragma_update(None, "user_version", 99).unwrap();
    drop(store);
    assert!(Store::open(&path).is_err());
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
#[test]
fn cohort_usage_overlap_and_incomplete_time_have_explicit_coverage() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let store = service.store.as_mut().unwrap();
    let mut scope = scope();
    scope.turn_id = Some(id());
    let main = id();
    let child = id();
    append(
        store,
        &scope,
        1000,
        Fact::TurnStarted {
            origin: Origin::default(),
            measurement: Measurement::default(),
        },
    );
    for execution_id in [&main, &child] {
        append(
            store,
            &scope,
            1000,
            Fact::ExecutionStarted {
                execution_id: execution_id.clone(),
            },
        );
        append(
            store,
            &scope,
            11000,
            Fact::ExecutionCompleted {
                execution_id: execution_id.clone(),
                elapsed_ms: 10000,
            },
        );
    }
    append(
        store,
        &scope,
        11000,
        Fact::TurnCompleted {
            message_key: None,
            outcome: Outcome::Ok,
            elapsed_ms: 10000,
            provider_duration_ms: None,
            measurement: Measurement {
                usage: Usage {
                    input_tokens: Some(10000),
                    cache_read_tokens: Some(8000),
                    output_tokens: Some(2000),
                    reasoning_tokens: Some(500),
                    ..Usage::default()
                },
                ..Measurement::default()
            },
        },
    );
    append(
        store,
        &scope,
        2000,
        Fact::ExecutionStarted { execution_id: id() },
    );
    let s = service.queries().summary(&Filter::default()).unwrap();
    assert_eq!((s.input_tokens, s.output_tokens), (Some(10000), Some(2000)));
    assert_eq!(
        (s.execution_sum_ms, s.active_agent_ms),
        (Some(20000), Some(10000))
    );
    assert_eq!(s.incomplete_executions, 1);
    let s = service
        .queries()
        .summary(&Filter {
            from: Some(1000),
            to: Some(6000),
            ..Filter::default()
        })
        .unwrap();
    assert_eq!(s.input_tokens, Some(10000));
    assert_eq!(s.active_agent_ms, Some(5000));
    let s = service
        .queries()
        .summary(&Filter {
            from: Some(6000),
            to: Some(11000),
            ..Filter::default()
        })
        .unwrap();
    assert_eq!(s.turns, 0);
    assert_eq!(s.input_tokens, None);
    assert_eq!(s.active_agent_ms, Some(5000));
}
#[test]
fn capture_keeps_requests_in_turn_and_children_outlive_completion_without_content() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let mut capture = Capture::default();
    let scope = scope();
    capture.accepted(&mut service, scope, Some("model-a".into()));
    capture.observe(&mut service,&json!({"type":"request.opened","requestId":"secret-request","kind":"approval","input":{"command":"PRIVATE COMMAND"}}));
    capture.observe(&mut service,&json!({"type":"request.closed","requestId":"secret-request","outcome":"allowed","answer":"PRIVATE ANSWER"}));
    capture.observe(&mut service,&json!({"type":"background.changed","tasks":[{"id":"secret-child","description":"PRIVATE TASK"}]}));
    capture.observe(
        &mut service,
        &json!({"type":"turn.completed","outcome":"ok","message":"PRIVATE RESPONSE"}),
    );
    capture.observe(
        &mut service,
        &json!({"type":"turn.completed","outcome":"ok"}),
    );
    let s = service.queries().summary(&Filter::default()).unwrap();
    assert_eq!(s.turns, 1);
    assert_eq!(s.completed_turns, 1);
    assert_eq!(s.incomplete_executions, 1);
    assert_eq!(s.responded_waits, 1);
    capture.observe(
        &mut service,
        &json!({"type":"background.changed","tasks":[]}),
    );
    assert_eq!(
        service
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .incomplete_executions,
        0
    );
    let export = service.queries().export(&Filter::default()).unwrap();
    for secret in ["PRIVATE", "secret-child", "secret-request"] {
        assert!(!export.contains(secret));
    }
}
#[test]
fn erase_invalidates_late_callbacks_new_activity_starts_new_history() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let mut capture = Capture::default();
    let scope = scope();
    capture.accepted(&mut service, scope.clone(), None);
    service.clear().unwrap();
    capture.observe(
        &mut service,
        &json!({"type":"turn.completed","outcome":"ok"}),
    );
    capture.observe(
        &mut service,
        &json!({"type":"background.changed","tasks":[{"id":"late"}]}),
    );
    assert_eq!(
        service
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .events,
        0
    );
    capture.accepted(&mut service, scope, None);
    assert_eq!(
        service.queries().summary(&Filter::default()).unwrap().turns,
        1
    );
    service.clear().unwrap();
    drop(service);
    let service = Service::new(dir.0.clone());
    assert_eq!(
        service
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .events,
        0
    );
}
#[test]
fn missing_store_does_not_fail_capture_and_health_survives_restart() {
    let dir = Temp::new();
    paths::ensure_private_dir(&dir.0).unwrap();
    std::fs::create_dir(dir.0.join("telemetry.sqlite3")).unwrap();
    let mut service = Service::new(dir.0.clone());
    service.capture(0, &event(&scope(), 1, Fact::ConversationCreated {}));
    assert!(
        service
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .health
            .unavailable
    );
    drop(service);
    let service = Service::new(dir.0.clone());
    assert!(service.health.failures >= 2);
}
#[test]
fn late_pr_relations_deduplicate_workspaces_and_do_not_allocate_cost() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let store = service.store.as_mut().unwrap();
    let mut scope = scope();
    scope.turn_id = Some(id());
    append(
        store,
        &scope,
        1,
        Fact::TurnStarted {
            origin: Origin::default(),
            measurement: Measurement::default(),
        },
    );
    let repo = id();
    for number in [1, 2] {
        append(
            store,
            &scope,
            20,
            Fact::PullRequestAssociated {
                repository_id: repo.clone(),
                branch_id: Some(id()),
                pull_request: number,
            },
        );
    }
    for number in [1, 2] {
        let s = service
            .queries()
            .summary(&Filter {
                repository_id: Some(repo.clone()),
                pull_request: Some(number),
                from: Some(0),
                to: Some(10),
                ..Filter::default()
            })
            .unwrap();
        assert_eq!(s.turns, 1);
    }
}
#[test]
fn malformed_measurements_and_payload_fields_are_rejected() {
    assert!(serde_json::from_value::<Usage>(json!({"prompt":"secret"})).is_err());
    let dir = Temp::new();
    let mut store = Store::open(&dir.0.join("telemetry.sqlite3")).unwrap();
    let mut scope = scope();
    scope.turn_id = Some(id());
    let mut measurement = Measurement::default();
    measurement.usage.cost_usd = Some(-1.);
    assert!(store
        .append(&event(&scope, 1, Fact::UsageObserved { measurement }))
        .is_err());
    scope.project_id = Some("/private/project".into());
    assert!(store
        .append(&event(&scope, 1, Fact::ConversationCreated {}))
        .is_err());
}
#[test]
fn legacy_board_identities_are_opaque_stable_and_distinct_from_paths() {
    let mut board = crate::state::Board::default();
    board.projects.push(crate::state::Project {
        id: "/private/repo".into(),
        path: "/private/repo".into(),
        name: "secret".into(),
    });
    board.prepare_telemetry_ids();
    let before = board.telemetry_ids.get("project", "/private/repo").unwrap();
    assert!(uuid::Uuid::parse_str(&before).is_ok());
    let mut restored: crate::state::Board =
        serde_json::from_str(&serde_json::to_string(&board).unwrap()).unwrap();
    restored.prepare_telemetry_ids();
    assert_eq!(
        restored.telemetry_ids.get("project", "/private/repo"),
        Some(before)
    );
}

#[test]
fn overlapping_input_preserves_unknown_attribution_instead_of_charging_the_new_message() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let mut capture = Capture::default();
    let scope = scope();
    capture.accepted(&mut service, scope.clone(), None);
    capture.accepted(&mut service, scope, None);
    capture.observe(
        &mut service,
        &json!({"type":"turn.completed","outcome":"ok"}),
    );
    let summary = service.queries().summary(&Filter::default()).unwrap();
    assert_eq!(summary.turns, 2);
    assert_eq!(summary.completed_turns, 0);
    assert!(summary.health.failures > 0);
}

#[test]
fn request_cancellations_union_time_and_clock_jumps_are_not_measured_as_work() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let store = service.store.as_mut().unwrap();
    let scope = scope();
    for responded in [true, false] {
        let request = id();
        append(
            store,
            &scope,
            1000,
            Fact::HumanRequested {
                request_id: request.clone(),
                kind: RequestKind::Question,
            },
        );
        let fact = if responded {
            Fact::HumanReceived {
                request_id: request,
                elapsed_ms: 10000,
            }
        } else {
            Fact::HumanCancelled {
                request_id: request,
                elapsed_ms: 10000,
            }
        };
        append(store, &scope, 11000, fact);
    }
    let execution = id();
    append(
        store,
        &scope,
        1000,
        Fact::ExecutionStarted {
            execution_id: execution.clone(),
        },
    );
    append(
        store,
        &scope,
        12000,
        Fact::ExecutionCompleted {
            execution_id: execution,
            elapsed_ms: 1000,
        },
    );
    let summary = service.queries().summary(&Filter::default()).unwrap();
    assert_eq!(summary.responded_waits, 1);
    assert_eq!(summary.cancelled_waits, 1);
    assert_eq!(summary.human_wait_ms, Some(10000));
    assert_eq!(summary.clock_anomalies, 1);
    assert_eq!(summary.active_agent_ms, None);
    let boundary = service
        .queries()
        .summary(&Filter {
            from: Some(11000),
            to: Some(15000),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(boundary.human_wait_ms, None);
}

#[test]
fn latest_snapshot_and_final_measurement_never_add_together_and_clear_removes_pages() {
    let dir = Temp::new();
    let path = dir.0.join("telemetry.sqlite3");
    let mut service = Service::new(dir.0.clone());
    let store = service.store.as_mut().unwrap();
    let mut scope = scope();
    scope.turn_id = Some(id());
    append(
        store,
        &scope,
        1,
        Fact::TurnStarted {
            origin: Origin::default(),
            measurement: Measurement::default(),
        },
    );
    let m = |input| Measurement {
        usage: Usage {
            input_tokens: Some(input),
            ..Default::default()
        },
        ..Default::default()
    };
    append(store, &scope, 2, Fact::UsageObserved { measurement: m(3) });
    append(store, &scope, 3, Fact::UsageObserved { measurement: m(4) });
    assert_eq!(
        service
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .input_tokens,
        Some(4)
    );
    append(
        service.store.as_mut().unwrap(),
        &scope,
        4,
        Fact::TurnCompleted {
            message_key: None,
            outcome: Outcome::Ok,
            elapsed_ms: 3,
            provider_duration_ms: None,
            measurement: m(5),
        },
    );
    assert_eq!(
        service
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .input_tokens,
        Some(5)
    );
    let before = service.queries().page(&Filter::default(), None).unwrap();
    let needle = before.events[0].id.clone();
    let next = service
        .queries()
        .page(
            &Filter::default(),
            Some(query::Cursor {
                occurred_at: 1,
                sequence: before.events[0].sequence,
            }),
        )
        .unwrap();
    assert_eq!(next.events.len(), 3);
    service.clear().unwrap();
    assert!(!String::from_utf8_lossy(&std::fs::read(path).unwrap()).contains(&needle));
    assert!(!dir.0.join("telemetry.sqlite3-journal").exists());
    // A stale background callback is ignored even if it constructs its event after erasure.
    let event = event(&scope, 10, Fact::ConversationCreated {});
    service.capture(0, &event);
    assert!(service
        .queries()
        .page(&Filter::default(), None)
        .unwrap()
        .events
        .is_empty());
}

#[test]
fn export_is_private_without_changing_the_destination_directory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Temp::new();
    std::fs::create_dir(&dir.0).unwrap();
    std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.0.join("history.jsonl");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_export(&path, |writer| {
        writer
            .write_all(b"{\"exportVersion\":1}\n")
            .map_err(|_| FAILURE.into())
    })
    .unwrap();
    assert_eq!(
        std::fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\"exportVersion\":1}\n"
    );
    assert!(write_export(&dir.0.join("telemetry.sqlite3"), |_| Ok(())).is_err());
}

#[test]
fn failed_commit_reports_coverage_and_a_later_retry_still_commits_once() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    service
        .store
        .as_ref()
        .unwrap()
        .db
        .pragma_update(None, "query_only", true)
        .unwrap();
    let e = event(&scope(), 1, Fact::ConversationCreated {});
    service.capture(0, &e);
    let s = service.queries().summary(&Filter::default()).unwrap();
    assert_eq!(s.events, 0);
    assert!(s.health.failures > 0);
    service
        .store
        .as_ref()
        .unwrap()
        .db
        .pragma_update(None, "query_only", false)
        .unwrap();
    service.capture(0, &e);
    service.capture(0, &e);
    assert_eq!(
        service
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .events,
        1
    );
}

#[test]
fn erased_run_cannot_complete_or_measure_a_new_overlapping_message() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let mut capture = Capture::default();
    let scope = scope();
    capture.accepted(&mut service, scope.clone(), None);
    service.clear().unwrap();
    capture.accepted(&mut service, scope.clone(), None);
    let measurement = Measurement {
        complete: true,
        usage: Usage {
            input_tokens: Some(99),
            output_tokens: Some(20),
            ..Default::default()
        },
        ..Default::default()
    };
    capture.observe(
        &mut service,
        &json!({"type":"telemetry.usage","measurement":measurement}),
    );
    capture.observe(
        &mut service,
        &json!({"type":"background.changed","tasks":[{"id":"old-child"}]}),
    );
    capture.observe(
        &mut service,
        &json!({"type":"turn.completed","outcome":"ok","telemetry":measurement}),
    );
    let summary = service.queries().summary(&Filter::default()).unwrap();
    assert_eq!(summary.turns, 1);
    assert_eq!(summary.completed_turns, 0);
    assert_eq!(summary.input_tokens, None);
    assert_eq!(summary.complete_executions, 0);
    assert_eq!(summary.incomplete_executions, 0);
    assert!(summary.health.failures > 0);
    capture.accepted(&mut service, scope, None);
    capture.observe(
        &mut service,
        &json!({"type":"background.changed","tasks":[{"id":"old-child"}]}),
    );
    capture.observe(
        &mut service,
        &json!({"type":"turn.completed","outcome":"ok"}),
    );
    let summary = service.queries().summary(&Filter::default()).unwrap();
    assert_eq!(summary.turns, 2);
    assert_eq!(summary.completed_turns, 1);
    assert_eq!(summary.incomplete_executions, 0);
}

#[test]
fn export_rejects_symlink_without_touching_target_or_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = Temp::new();
    std::fs::create_dir(&dir.0).unwrap();
    let target = dir.0.join("private.txt");
    let destination = dir.0.join("selected.jsonl");
    std::fs::write(&target, "Keep this content").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&target, &destination).unwrap();
    assert!(write_export(&destination, |_| panic!("symlink must not be opened")).is_err());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "Keep this content"
    );
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    std::fs::remove_file(&target).unwrap();
    assert!(write_export(&destination, |_| Ok(())).is_err());
    assert!(!target.exists());
}

#[test]
fn snapshot_queries_allow_commits_and_erasure_truncates_wal_after_readers_finish() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let first = event(&scope(), 1, Fact::ConversationCreated {});
    service.capture(0, &first);
    let shared = Mutex::new(service);
    let queries = lock(&shared).queries();
    queries
        .read(|store| {
            assert_eq!(
                store.summary(&Filter::default(), Health::default())?.events,
                1
            );
            let mut writer = shared
                .try_lock()
                .expect("query must not hold the capture mutex");
            writer.capture(0, &event(&scope(), 2, Fact::ConversationCreated {}));
            assert_eq!(
                writer.health.failures, 0,
                "reader must not block the SQLite commit"
            );
            assert!(
                writer.readers.try_write().is_err(),
                "erasure must wait for this snapshot"
            );
            assert_eq!(
                store.summary(&Filter::default(), Health::default())?.events,
                1
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        lock(&shared)
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .events,
        2
    );
    lock(&shared).clear().unwrap();
    for name in [
        "telemetry.sqlite3",
        "telemetry.sqlite3-wal",
        "telemetry.sqlite3-shm",
    ] {
        let bytes = std::fs::read(dir.0.join(name)).unwrap_or_default();
        assert!(!String::from_utf8_lossy(&bytes).contains(&first.id));
        if name.ends_with("-wal") {
            assert!(bytes.is_empty());
        }
    }
}

#[test]
fn workspace_choices_include_other_histories_while_summary_stays_filtered() {
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let first = scope();
    let second = scope();
    service.capture(0, &event(&first, 1, Fact::ConversationCreated {}));
    service.capture(0, &event(&second, 2, Fact::ConversationCreated {}));
    let summary = service
        .queries()
        .summary(&Filter {
            workspace_id: first.workspace_id.clone(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(summary.events, 1);
    assert!(summary
        .workspace_ids
        .contains(first.workspace_id.as_ref().unwrap()));
    assert!(summary
        .workspace_ids
        .contains(second.workspace_id.as_ref().unwrap()));
}

#[test]
fn streamed_export_uses_one_snapshot_while_new_capture_commits() {
    struct Writer<'a> {
        service: &'a Mutex<Service>,
        captured: bool,
        lines: usize,
    }
    impl std::io::Write for Writer<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if !self.captured {
                let mut service = self
                    .service
                    .try_lock()
                    .expect("export must release capture mutex");
                service.capture(0, &event(&scope(), 999, Fact::ConversationCreated {}));
                assert_eq!(service.health.failures, 0);
                self.captured = true;
            }
            self.lines += bytes.iter().filter(|&&b| b == b'\n').count();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let dir = Temp::new();
    let mut service = Service::new(dir.0.clone());
    let scope = scope();
    for at in 0..600 {
        service.capture(0, &event(&scope, at, Fact::ConversationCreated {}));
    }
    let shared = Mutex::new(service);
    let queries = lock(&shared).queries();
    queries
        .read(|store| {
            let summary = store.summary(&Filter::default(), Health::default())?;
            assert_eq!(summary.events, 600);
            let mut writer = Writer {
                service: &shared,
                captured: false,
                lines: 0,
            };
            store.export(&Filter::default(), &summary, &mut writer)?;
            assert_eq!(
                writer.lines, 601,
                "metadata plus the original snapshot, without late capture"
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        lock(&shared)
            .queries()
            .summary(&Filter::default())
            .unwrap()
            .events,
        601
    );
    let destination = dir.0.join("export.jsonl");
    queries.export_to(&Filter::default(), &destination).unwrap();
    assert_eq!(
        std::fs::read_to_string(destination)
            .unwrap()
            .lines()
            .count(),
        602
    );
}

#[test]
fn existing_delete_journal_database_reopens_in_wal_without_losing_events() {
    let dir = Temp::new();
    let path = dir.0.join("telemetry.sqlite3");
    let mut store = Store::open(&path).unwrap();
    let original = event(&scope(), 1, Fact::ConversationCreated {});
    store.append(&original).unwrap();
    store
        .db
        .execute_batch(
            "DROP INDEX events_execution; DROP INDEX events_request; PRAGMA journal_mode=DELETE;",
        )
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .db
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        store
            .page(&Filter::default(), None, Health::default())
            .unwrap()
            .events[0]
            .id,
        original.id
    );
}

#[test]
fn failed_row_decode_preserves_existing_export_and_removes_temporary_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Temp::new();
    let mut service = Service::new(dir.0.join("state"));
    let scope = scope();
    service.capture(0, &event(&scope, 1, Fact::ConversationCreated {}));
    let malformed = event(
        &scope,
        2,
        Fact::ContextCompacted {
            before: None,
            after: None,
        },
    );
    service.capture(0, &malformed);
    service
        .store
        .as_ref()
        .unwrap()
        .db
        .execute(
            "UPDATE events SET payload=?1 WHERE id=?2",
            params![r#"{"before":"invalid","after":null}"#, malformed.id],
        )
        .unwrap();
    let destination = dir.0.join("history.jsonl");
    std::fs::write(&destination, "Previous export\n").unwrap();
    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o640)).unwrap();
    let queries = service.queries();
    assert!(queries.export_to(&Filter::default(), &destination).is_err());
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "Previous export\n"
    );
    assert_eq!(
        std::fs::metadata(&destination)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    let absent = dir.0.join("new.jsonl");
    assert!(queries.export_to(&Filter::default(), &absent).is_err());
    assert!(!absent.exists());
    assert_eq!(
        std::fs::read_dir(&dir.0).unwrap().count(),
        2,
        "only state and the original export remain"
    );
}

#[test]
fn export_publishes_only_after_successful_stream_and_rejects_swapped_symlink() {
    use std::os::unix::fs::symlink;
    let dir = Temp::new();
    std::fs::create_dir(&dir.0).unwrap();
    let destination = dir.0.join("history.jsonl");
    std::fs::write(&destination, "Previous export").unwrap();
    assert!(write_export(&destination, |writer| {
        writer.write_all(&[b'x'; 16384]).unwrap();
        writer.flush().unwrap();
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "Previous export"
        );
        Err("err.telemetry.export".into())
    })
    .is_err());
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "Previous export"
    );
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    let target = dir.0.join("other.txt");
    std::fs::write(&target, "Keep this content").unwrap();
    assert!(write_export(&destination, |writer| {
        writer.write_all(b"Complete export").unwrap();
        std::fs::remove_file(&destination).unwrap();
        symlink(&target, &destination).unwrap();
        Ok(())
    })
    .is_err());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "Keep this content"
    );
    assert!(std::fs::symlink_metadata(&destination)
        .unwrap()
        .is_symlink());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
}
