use super::reactions::{SessionActions, SessionContext, SessionReactions, SessionUsage};
use super::*;
use crate::conversation::work::Work;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

struct Fixture {
    board: Mutex<Board>,
    runtime: Mutex<RuntimeState>,
    account: Mutex<Result<AccountState, String>>,
    setup: AtomicBool,
    fail_send: AtomicBool,
    fail_revive: AtomicBool,
    fail_persist: AtomicBool,
    fail_after_dispatch: AtomicBool,
    dispatch_accepted: AtomicBool,
    durable: Mutex<Option<Board>>,
    effects: Mutex<Vec<String>>,
    looking: Mutex<Option<String>>,
}
impl Fixture {
    fn new() -> Self {
        Self {
            board: Mutex::new(serde_json::from_value(json!({"stages":[],"workspaces":[{
                "id":"w","title":"workspace","repo":"","repo_name":"","branch":"","worktree":"","stage":"review",
                "tabs":[{"id":"s","title":"session","status":"pronta"},{"id":"sibling","title":"other","status":"desligada"}]
            }]})).unwrap()),
            runtime: Mutex::new(RuntimeState { up: true, ready: true, waits_for_turn: false }),
            account: Mutex::new(Ok(AccountState::default())),
            setup: AtomicBool::new(false), fail_send: AtomicBool::new(false), fail_revive: AtomicBool::new(false),
            fail_persist: AtomicBool::new(false), fail_after_dispatch: AtomicBool::new(false), dispatch_accepted: AtomicBool::new(false), durable: Mutex::new(None),
            effects: Mutex::new(vec![]), looking: Mutex::new(None),
        }
    }
    fn service(&self) -> SessionService<'_> {
        SessionService {
            board: &self.board,
            runtime: self,
            accounts: self,
            publication: self,
            diagnostics: self,
        }
    }
    fn queue(&self, text: &str) {
        lock(&self.board).tab_mut("s").unwrap().pending_prompt = Some(text.into());
    }
    fn pending(&self) -> Option<String> {
        lock(&self.board)
            .tab_mut("s")
            .unwrap()
            .pending_prompt
            .clone()
    }
    fn effect(&self, text: String) {
        assert!(
            self.board.try_lock().is_ok(),
            "effect must release the board lock"
        );
        lock(&self.effects).push(text);
    }
}
impl SessionRuntime for Fixture {
    fn state(&self, _: &str) -> RuntimeState {
        *lock(&self.runtime)
    }
    fn setup_running(&self, _: &str) -> bool {
        self.setup.load(Ordering::Relaxed)
    }
    fn stop(&self, _: &str) {
        self.effect("stop".into());
        *lock(&self.runtime) = RuntimeState::default();
    }
    fn revive(&self, _: &str) -> Result<(), String> {
        self.effect("revive".into());
        if self.fail_revive.load(Ordering::Relaxed) {
            return Err("spawn failed".into());
        }
        Ok(())
    }
    fn mark_ready(&self, _: &str) {
        self.effect("ready".into());
        lock(&self.runtime).ready = true;
    }
    fn send(&self, _: &str, text: &str, idle: bool) -> Result<(), String> {
        self.effect(format!("send:{idle}:{text}"));
        if self.fail_send.load(Ordering::Relaxed) {
            self.queue("arrived during send");
            return Err("broken pipe".into());
        }
        Ok(())
    }
    fn send_dispatch(
        &self,
        session: &str,
        dispatch: &crate::board::NativeDispatch,
    ) -> Result<DispatchOutcome, String> {
        let mut durable = lock(&self.durable)
            .clone()
            .expect("dispatch must first persist");
        let tab = durable.tab_mut(session).unwrap();
        assert_eq!(tab.native_dispatch.as_ref().unwrap().id, dispatch.id);
        assert_eq!(tab.native_dispatch.as_ref().unwrap().text, dispatch.text);
        if self.dispatch_accepted.load(Ordering::Relaxed) {
            self.effect("already-accepted".into());
            return Ok(DispatchOutcome::AlreadyAccepted);
        }
        self.effect(format!("dispatch:{}:{}", dispatch.id, dispatch.text));
        if self.fail_send.load(Ordering::Relaxed) {
            return Err("uncertain acceptance".into());
        }
        if self.fail_after_dispatch.load(Ordering::Relaxed) {
            self.fail_persist.store(true, Ordering::Relaxed);
        }
        Ok(DispatchOutcome::Sent)
    }
}
impl SessionAccounts for Fixture {
    fn state(&self, _: &str) -> Result<AccountState, String> {
        lock(&self.account).clone()
    }
}
impl SessionPublication for Fixture {
    fn publish(&self) {
        self.effect("publish".into());
    }
    fn looking(&self) -> Option<String> {
        lock(&self.looking).clone()
    }
    fn persist(&self) -> Result<(), String> {
        self.effect("persist".into());
        if self.fail_persist.load(Ordering::Relaxed) {
            return Err("storage unavailable".into());
        }
        *lock(&self.durable) = Some(lock(&self.board).clone());
        Ok(())
    }
}
impl SessionDiagnostics for Fixture {
    fn account_error(&self, error: &str) {
        self.effect(format!("account:{error}"));
    }
    fn pending_error(&self, _: &str, error: &str) {
        self.effect(format!("pending:{error}"));
    }
}

#[test]
fn native_dispatch_is_durable_before_write_and_uncertainty_preserves_its_identity_and_later_queue()
{
    let fixture = Fixture::new();
    lock(&fixture.board).tab_mut("s").unwrap().conversation_mode =
        crate::board::ConversationMode::Terminal;
    fixture.queue("First prompt");
    fixture.fail_send.store(true, Ordering::Relaxed);
    fixture.service().send_prompt("s", None);
    let dispatch = lock(&fixture.board)
        .tab_mut("s")
        .unwrap()
        .native_dispatch
        .clone()
        .unwrap();
    assert_eq!(dispatch.text, "First prompt");
    assert!(fixture.pending().is_none());
    fixture.service().send("s", "Later prompt", false).unwrap();
    assert_eq!(fixture.pending().as_deref(), Some("Later prompt"));
    assert_eq!(
        lock(&fixture.board)
            .tab_mut("s")
            .unwrap()
            .native_dispatch
            .as_ref()
            .unwrap()
            .id,
        dispatch.id
    );
    let bytes = serde_json::to_vec(&*lock(&fixture.board)).unwrap();
    let restored = Fixture::new();
    *lock(&restored.board) = serde_json::from_slice(&bytes).unwrap();
    restored.dispatch_accepted.store(true, Ordering::Relaxed);
    restored.service().send_prompt("s", None);
    assert!(lock(&restored.board)
        .tab_mut("s")
        .unwrap()
        .native_dispatch
        .is_none());
    assert_eq!(restored.pending().as_deref(), Some("Later prompt"));
    assert!(lock(&restored.effects)
        .iter()
        .any(|effect| effect == "already-accepted"));
    assert!(!lock(&restored.effects)
        .iter()
        .any(|effect| effect.starts_with("dispatch:") || effect.starts_with("send:")));
    assert!(matches!(
        lock(&restored.board).tab_mut("s").unwrap().status,
        Status::Pronta
    ));
}

#[test]
fn native_dispatch_storage_failure_keeps_recoverable_text_without_calling_the_process() {
    let fixture = Fixture::new();
    lock(&fixture.board).tab_mut("s").unwrap().conversation_mode =
        crate::board::ConversationMode::Terminal;
    fixture.queue("Keep this input");
    fixture.fail_persist.store(true, Ordering::Relaxed);
    fixture.service().send_prompt("s", None);
    assert_eq!(
        lock(&fixture.board)
            .tab_mut("s")
            .unwrap()
            .native_dispatch
            .as_ref()
            .unwrap()
            .text,
        "Keep this input"
    );
    assert!(!lock(&fixture.effects)
        .iter()
        .any(|effect| effect.starts_with("dispatch:")));
}

#[test]
fn native_dispatch_accepted_before_board_clear_recovers_without_a_second_send() {
    let fixture = Fixture::new();
    lock(&fixture.board).tab_mut("s").unwrap().conversation_mode =
        crate::board::ConversationMode::Terminal;
    fixture.queue("Accepted once");
    fixture.fail_after_dispatch.store(true, Ordering::Relaxed);
    fixture.service().send_prompt("s", None);
    let saved = lock(&fixture.durable).clone().unwrap();
    let id = lock(&fixture.board)
        .tab_mut("s")
        .unwrap()
        .native_dispatch
        .as_ref()
        .unwrap()
        .id
        .clone();
    let restored = Fixture::new();
    *lock(&restored.board) = saved;
    assert_eq!(
        lock(&restored.board)
            .tab_mut("s")
            .unwrap()
            .native_dispatch
            .as_ref()
            .unwrap()
            .id,
        id
    );
    restored.dispatch_accepted.store(true, Ordering::Relaxed);
    restored.service().send_prompt("s", None);
    assert!(lock(&restored.board)
        .tab_mut("s")
        .unwrap()
        .native_dispatch
        .is_none());
    assert!(lock(&restored.effects)
        .iter()
        .any(|effect| effect == "already-accepted"));
    assert!(!lock(&restored.effects)
        .iter()
        .any(|effect| effect.starts_with("dispatch:") || effect.starts_with("send:")));
}
impl SessionContext for Fixture {
    fn tokens(&self, _: &str) -> Option<u64> {
        Some(123)
    }
}
impl SessionActions for Fixture {
    fn completed(&self, _: &str, failed: bool) {
        self.effect(format!("completed:{failed}"));
    }
}
impl SessionUsage for Fixture {
    fn observe(&self, _: &Value) {
        self.effect("usage".into());
    }
}

#[test]
fn input_gates_serialize_one_session_without_blocking_others() {
    let gates = InputGates::default();
    let gate = gates.session("s");
    let _held = lock(&gate);
    assert!(gates.session("s").try_lock().is_err());
    assert!(gates.session("other").try_lock().is_ok());
}

#[test]
fn immediate_input_uses_idle_admission_and_preserves_the_workspace_stage_and_siblings() {
    let f = Fixture::new();
    f.service().send("s", "  hello  ", true).unwrap();
    assert_eq!(*lock(&f.effects), ["send:true:hello", "publish"]);
    let board = lock(&f.board);
    assert_eq!(board.workspaces[0].stage, "review");
    assert!(matches!(
        board.workspaces[0].tabs[0].status,
        Status::Rodando
    ));
    assert!(matches!(
        board.workspaces[0].tabs[1].status,
        Status::Desligada
    ));
    assert!(!board.workspaces[0].unread);
}

#[test]
fn an_account_change_waits_for_work_and_retains_the_queued_message() {
    let f = Fixture::new();
    *lock(&f.account) = Ok(AccountState {
        changed: true,
        working: true,
    });
    f.service().send("s", "next", false).unwrap();
    assert_eq!(f.pending().as_deref(), Some("next"));
    assert_eq!(*lock(&f.effects), ["publish"]);
}

#[test]
fn a_changed_idle_account_stops_then_publishes_input_before_restart() {
    let f = Fixture::new();
    *lock(&f.account) = Ok(AccountState {
        changed: true,
        working: false,
    });
    f.service().send("s", "next", false).unwrap();
    assert_eq!(*lock(&f.effects), ["stop", "publish", "revive"]);
    assert_eq!(f.pending().as_deref(), Some("next"));
}

#[test]
fn stopped_sessions_recover_existing_queues_even_when_input_was_appended() {
    let f = Fixture::new();
    f.queue("first");
    lock(&f.runtime).up = false;
    f.fail_revive.store(true, Ordering::Relaxed);
    assert_eq!(
        f.service().send("s", "second", false),
        Err("spawn failed".into())
    );
    assert_eq!(f.pending().as_deref(), Some("first\n\nsecond"));
    assert_eq!(*lock(&f.effects), ["publish", "revive"]);
}

#[test]
fn readiness_recovery_waits_for_setup_then_sends_the_queue_once() {
    let f = Fixture::new();
    f.queue("queued");
    lock(&f.runtime).ready = false;
    f.setup.store(true, Ordering::Relaxed);
    f.service().flush_pending("s").unwrap();
    assert_eq!(*lock(&f.effects), ["ready"]);
    f.setup.store(false, Ordering::Relaxed);
    f.service().flush_pending("s").unwrap();
    f.service().flush_pending("s").unwrap();
    assert_eq!(*lock(&f.effects), ["ready", "send:false:queued", "publish"]);
    assert_eq!(f.pending(), None);
}

#[test]
fn provider_waiting_and_login_errors_leave_input_available() {
    let f = Fixture::new();
    f.queue("queued");
    lock(&f.runtime).waits_for_turn = true;
    f.service().send_prompt("s", Some("prefix:".into()));
    assert_eq!(f.pending().as_deref(), Some("queued"));
    lock(&f.runtime).waits_for_turn = false;
    *lock(&f.account) = Err(code("err.account.busy"));
    f.service().send_prompt("s", Some("prefix:".into()));
    assert_eq!(f.pending().as_deref(), Some("prefix:queued"));
    assert_eq!(
        *lock(&f.effects),
        [format!("account:{}", code("err.account.busy"))]
    );
}

#[test]
fn failed_input_is_restored_before_new_input_and_pauses_its_action() {
    let f = Fixture::new();
    f.queue("first");
    let mut board = lock(&f.board);
    board.tab_mut("s").unwrap().task = Some(serde_json::from_value(json!({
        "command":"test","profile":{"id":"p","name":"profile","prompt":"","choice":{"agent":"claude","model":"","effort":""},"skills":[],"permission":"ask"},
        "paused":false,"done":false,"turns":0,"checked_at":0
    })).unwrap());
    drop(board);
    f.fail_send.store(true, Ordering::Relaxed);
    f.service().send_prompt("s", None);
    assert_eq!(f.pending().as_deref(), Some("first\n\narrived during send"));
    let mut board = lock(&f.board);
    let task = board.tab_mut("s").unwrap().task.as_ref().unwrap();
    assert!(task.paused);
    assert_eq!(task.error.as_deref(), Some("broken pipe"));
}

#[test]
fn blank_input_has_no_effect_and_missing_tabs_keep_the_existing_error() {
    let f = Fixture::new();
    *lock(&f.account) = Err("no account".into());
    f.service().send("s", " \n ", false).unwrap();
    assert!(lock(&f.effects).is_empty());
    *lock(&f.account) = Ok(AccountState::default());
    lock(&f.runtime).up = false;
    assert_eq!(
        f.service().send("missing", "hello", false),
        Err(code("err.session.noTab"))
    );
}

#[test]
fn execution_observations_update_availability_without_completion_or_answer_routes() {
    let f = Fixture::new();
    let sessions = f.service();
    let work = Mutex::new(HashMap::<String, Work>::new());
    let reactions = SessionReactions {
        sessions: &sessions,
        work: &work,
        context: &f,
        actions: &f,
        usage: &f,
    };
    let ready = AtomicBool::new(false);
    let react = |state: &str, detail: Option<&str>| {
        reactions.react(
            "s",
            &json!({
                "type":"execution.observed", "state":state, "scope":"root", "detail":detail
            }),
            &ready,
        )
    };
    let status = || serde_json::to_value(lock(&f.board).tab_mut("s").unwrap().status).unwrap();
    assert!(!react("working", Some("Inspecting sources")));
    assert_eq!(status(), "rodando");
    assert!(!react("idle", None));
    assert_eq!(status(), "pronta");
    assert!(
        !lock(&f.board).workspaces[0].unread,
        "availability does not announce completion"
    );
    assert!(
        !ready.load(Ordering::Relaxed),
        "status is not native input readiness"
    );
    assert!(!react("blocked", Some("Native approval required")));
    assert_eq!(status(), "querendo");
    assert!(lock(&f.board).workspaces[0].unread);
    assert!(lock(&work)["s"].waiting());
    assert!(!react("unknown", None));
    assert_eq!(status(), "rodando");
    assert!(!lock(&work)["s"].waiting());
    assert!(!lock(&f.effects)
        .iter()
        .any(|effect| effect.starts_with("completed:")));
    reactions.react("s", &json!({"type":"request.opened","requestId":"real","kind":"question","input":{"questions":[{"question":"Which target?"}]}}), &ready);
    react("idle", None);
    assert_eq!(status(), "querendo");
    assert_eq!(
        lock(&f.board).tab_mut("s").unwrap().note.as_deref(),
        Some("Which target?")
    );
}

#[test]
fn resumed_activity_prevents_an_old_background_drain_from_settling_the_session() {
    let f = Fixture::new();
    let sessions = f.service();
    let work = Mutex::new(HashMap::<String, Work>::new());
    let reactions = SessionReactions {
        sessions: &sessions,
        work: &work,
        context: &f,
        actions: &f,
        usage: &f,
    };
    let ready = AtomicBool::new(false);
    assert!(!reactions.react(
        "s",
        &json!({"type":"background.changed","tasks":[{"id":"child"}]}),
        &ready
    ));
    assert!(!reactions.react(
        "s",
        &json!({"type":"turn.completed","outcome":"ok"}),
        &ready
    ));
    assert!(!reactions.react("s", &json!({"type":"assistant.started"}), &ready));
    assert!(!reactions.react(
        "s",
        &json!({"type":"background.changed","tasks":[]}),
        &ready
    ));
    assert!(matches!(
        lock(&f.board).tab_mut("s").unwrap().status,
        Status::Rodando
    ));
    assert!(reactions.react(
        "s",
        &json!({"type":"turn.completed","outcome":"ok"}),
        &ready
    ));
    assert!(matches!(
        lock(&f.board).tab_mut("s").unwrap().status,
        Status::Pronta
    ));
}

#[test]
fn canonical_reactions_preserve_request_notes_identity_and_account_usage_ports() {
    let f = Fixture::new();
    let sessions = f.service();
    let work = Mutex::new(HashMap::new());
    let reactions = SessionReactions {
        sessions: &sessions,
        work: &work,
        context: &f,
        actions: &f,
        usage: &f,
    };
    let ready = AtomicBool::new(false);
    for used in [90, 0] {
        reactions.react(
            "s",
            &json!({"type":"context.updated","used":used,"window":200000}),
            &ready,
        );
        let mut board = lock(&f.board);
        let tab = board.tab_mut("s").unwrap();
        assert_eq!(tab.context_tokens, Some(used));
        assert_eq!(tab.context_window, Some(200000));
    }

    reactions.react(
        "s",
        &json!({"type":"request.opened","kind":"approval","tool":"Bash"}),
        &ready,
    );
    assert_eq!(
        lock(&f.board).tab_mut("s").unwrap().note,
        Some(crate::error::with_args(
            "note.permission",
            &[("tool", "Bash".into())]
        ))
    );
    assert!(lock(&f.board).workspaces[0].unread);
    reactions.react(
        "s",
        &json!({"type":"session.identity","providerSession":"native-thread"}),
        &ready,
    );
    let before = lock(&f.effects).len();
    reactions.react(
        "s",
        &json!({"type":"session.identity","providerSession":"native-thread"}),
        &ready,
    );
    assert_eq!(lock(&f.effects).len(), before);
    reactions.react(
        "s",
        &json!({"type":"usage.updated","provider":"fixture"}),
        &ready,
    );
    assert_eq!(lock(&f.effects).last().unwrap(), "usage");
}

#[test]
fn answering_a_request_runs_the_tab_again_unless_another_still_waits() {
    let f = Fixture::new();
    let sessions = f.service();
    let work = Mutex::new(HashMap::new());
    let reactions = SessionReactions {
        sessions: &sessions,
        work: &work,
        context: &f,
        actions: &f,
        usage: &f,
    };
    let ready = AtomicBool::new(true);
    let react = |event: Value| reactions.react("s", &event, &ready);
    let status = || serde_json::to_value(lock(&f.board).tab_mut("s").unwrap().status).unwrap();
    let note = || lock(&f.board).tab_mut("s").unwrap().note.clone();
    let tool = |command: &str| json!({"type":"assistant.block","block":{"kind":"tool","name":"Bash","input":{"command":command}}});

    react(tool("npm test"));
    react(json!({"type":"request.opened","requestId":"a","kind":"approval","tool":"Bash"}));
    react(
        json!({"type":"request.opened","requestId":"b","kind":"question",
        "input":{"questions":[{"question":"Which suite?"}]}}),
    );
    assert_eq!(status(), "querendo");
    react(json!({"type":"request.closed","requestId":"a","outcome":"allowed"}));
    assert_eq!(status(), "querendo", "another request still waits");
    // Output while a request is open keeps the tab waiting and the request's note.
    react(json!({"type":"assistant.block","block":{"kind":"text","text":"Waiting"}}));
    react(tool("ls"));
    assert_eq!(status(), "querendo");
    assert_eq!(note().as_deref(), Some("Which suite?"));

    // The answer moves the tab on at once, before the long tool produces anything.
    react(json!({"type":"request.closed","requestId":"b","outcome":"answered"}));
    assert_eq!(status(), "rodando");
    assert_eq!(note(), None);
    react(tool("cargo test"));
    assert_eq!(status(), "rodando");
    react(json!({"type":"turn.completed","outcome":"ok"}));
    assert_eq!(status(), "pronta");

    // A late answer to a request the settled turn dropped does not start the tab again.
    react(json!({"type":"request.opened","requestId":"c","kind":"plan"}));
    react(json!({"type":"turn.completed","outcome":"interrupted"}));
    assert_eq!(status(), "pronta");
    react(json!({"type":"request.closed","requestId":"c","outcome":"cancelled"}));
    assert_eq!(status(), "pronta");
    // Nor does the dropped request keep the next turn waiting after its own answer.
    react(tool("make"));
    react(json!({"type":"request.opened","requestId":"d","kind":"approval","tool":"Bash"}));
    react(json!({"type":"request.closed","requestId":"d","outcome":"allowed"}));
    assert_eq!(status(), "rodando");
}

#[test]
fn an_accepted_message_clears_the_interrupted_marker() {
    let f = Fixture::new();
    lock(&f.board).tab_mut("s").unwrap().interrupted = true;
    let sessions = f.service();
    let work = Mutex::new(HashMap::new());
    let reactions = SessionReactions {
        sessions: &sessions,
        work: &work,
        context: &f,
        actions: &f,
        usage: &f,
    };
    let ready = AtomicBool::new(true);
    reactions.react(
        "s",
        &json!({"type":"assistant.block","block":{"kind":"text","text":"x"}}),
        &ready,
    );
    assert!(lock(&f.board).tab_mut("s").unwrap().interrupted);
    reactions.react(
        "s",
        &json!({"type":"user.message","content":[{"kind":"text","text":"Continue"}]}),
        &ready,
    );
    assert!(!lock(&f.board).tab_mut("s").unwrap().interrupted);
}
