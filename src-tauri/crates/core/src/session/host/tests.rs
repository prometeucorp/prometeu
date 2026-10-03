use super::*;
use crate::board::Status;
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

#[derive(Default)]
struct Conversation {
    identity: ProcessIdentity,
    stopped: bool,
    waits: bool,
    working: bool,
    changed: bool,
    account_error: Option<String>,
    _drop: DropProbe,
}
#[derive(Default)]
struct DropProbe {
    owner: Weak<SessionHost<Conversation>>,
    dropped: Arc<AtomicBool>,
    retired: bool,
}
impl HostedConversation for Conversation {
    fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }
    fn retire(&mut self) {
        if let Some(host) = self._drop.owner.upgrade() {
            assert!(host.conversations.try_lock().is_err());
        }
        self._drop.retired = true;
    }
    fn alive(&self) -> bool {
        !self.stopped
    }
    fn waits_for_turn(&self) -> bool {
        self.waits
    }
    fn working(&self) -> bool {
        self.working
    }
    fn changing_account(&self) -> Result<bool, String> {
        match &self.account_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.changed),
        }
    }
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        if let Some(host) = self.owner.upgrade() {
            assert!(self.retired);
            assert!(
                host.conversations.try_lock().is_ok(),
                "transport teardown must not hold registry locks"
            );
            assert!(!host.is_ready("s"));
            assert!(!host.busy("s"));
        }
        self.dropped.store(true, Ordering::Relaxed);
    }
}
fn work(host: &SessionHost<Conversation>) {
    lock(&host.work)
        .entry("s".into())
        .or_default()
        .observe(&json!({"type":"background.changed","tasks":[{"id":"child"}]}));
}

#[test]
fn stale_exit_cannot_clear_or_publish_for_a_replacement_and_current_exit_keeps_replay() {
    let host = SessionHost::default();
    let old = Conversation::default();
    let old_id = old.identity.clone();
    lock(&host.conversations).insert("s".into(), old);
    let current = Conversation::default();
    let current_id = current.identity.clone();
    lock(&host.conversations).insert("s".into(), current);
    host.mark_ready("s");
    work(&host);
    assert!(!host.exited("s", &old_id, || panic!("stale closure publication")));
    assert!(!host.exited("missing", &current_id, || panic!(
        "unknown closure publication"
    )));
    assert!(host.is_ready("s"));
    assert!(host.busy("s"));
    assert!(host.exited("s", &current_id, || {
        assert!(
            host.conversations.try_lock().is_err(),
            "replacement must wait for closure publication"
        );
        assert!(!host.is_ready("s"));
        assert!(!host.busy("s"));
    }));
    assert!(lock(&host.conversations).contains_key("s"));
}

#[test]
fn removal_clears_transient_state_before_dropping_transport_outside_locks() {
    let host = Arc::new(SessionHost::default());
    let dropped = Arc::new(AtomicBool::new(false));
    lock(&host.conversations).insert(
        "s".into(),
        Conversation {
            _drop: DropProbe {
                owner: Arc::downgrade(&host),
                dropped: dropped.clone(),
                retired: false,
            },
            ..Default::default()
        },
    );
    host.mark_ready("s");
    work(&host);
    host.remove("s");
    assert!(dropped.load(Ordering::Relaxed));
    assert!(!host.state("s").up);
    host.mark_ready("s");
    work(&host);
    host.remove("s");
    assert!(!host.is_ready("s"));
    assert!(!host.busy("s"));
}

struct Effects<'a> {
    host: &'a SessionHost<Conversation>,
    events: Mutex<Vec<String>>,
    setup: bool,
}
impl SessionLifecycle for Effects<'_> {
    fn setup_running(&self, _: &str) -> bool {
        self.setup
    }
    fn revive(&self, session: &str) -> Result<(), String> {
        lock(&self.events).push("revive".into());
        lock(&self.host.conversations).insert(session.into(), Conversation::default());
        Ok(())
    }
    fn stopped(&self, session: &str) {
        assert!(!self.host.state(session).up);
        assert!(!self.host.is_ready(session));
        assert!(!self.host.busy(session));
        lock(&self.events).push("stop".into());
    }
    fn send(&self, _: &str, text: &str, idle: bool) -> Result<(), String> {
        lock(&self.events).push(format!("send:{idle}:{text}"));
        Ok(())
    }
}
impl SessionPublication for Effects<'_> {
    fn publish(&self) {
        lock(&self.events).push("publish".into());
    }
    fn looking(&self) -> Option<String> {
        Some("w".into())
    }
}
impl SessionDiagnostics for Effects<'_> {
    fn account_error(&self, error: &str) {
        panic!("unexpected account error: {error}");
    }
    fn pending_error(&self, _: &str, error: &str) {
        panic!("unexpected input error: {error}");
    }
}
fn board() -> Mutex<Board> {
    Mutex::new(serde_json::from_value(json!({"stages":[],"workspaces":[{
        "id":"w","title":"workspace","repo":"","repo_name":"","branch":"","worktree":"","stage":"review",
        "tabs":[{"id":"s","title":"session","status":"pronta"}]
    }]})).unwrap())
}

#[test]
fn composed_service_recovers_queued_input_and_waits_for_readiness() {
    let host = SessionHost::<Conversation>::default();
    let board = board();
    let effects = Effects {
        host: &host,
        events: Mutex::new(vec![]),
        setup: false,
    };
    host.with_service(&board, &effects, &effects, &effects, |service| {
        service.send("s", "queued input", false).unwrap();
        assert_eq!(*lock(&effects.events), ["publish", "revive"]);
        assert!(!host.is_ready("s"));
        assert_eq!(
            lock(&board).tab_mut("s").unwrap().pending_prompt.as_deref(),
            Some("queued input")
        );
        service.ready_now("s");
        assert!(host.is_ready("s"));
        service.send("s", "next input", true).unwrap();
    });
    assert_eq!(
        *lock(&effects.events),
        [
            "publish",
            "revive",
            "send:false:queued input",
            "publish",
            "send:true:next input",
            "publish"
        ]
    );
    let mut board = lock(&board);
    assert!(board.tab_mut("s").unwrap().pending_prompt.is_none());
    assert!(board.tab_mut("s").unwrap().status == Status::Rodando);
    assert_eq!(board.workspace_of("s").unwrap().stage, "review");
}

#[test]
fn account_handoff_uses_the_host_registry_and_preserves_pending_input() {
    let host = SessionHost::default();
    lock(&host.conversations).insert(
        "s".into(),
        Conversation {
            changed: true,
            ..Default::default()
        },
    );
    host.mark_ready("s");
    lock(&host.work).insert("s".into(), Work::default());
    let board = board();
    let effects = Effects {
        host: &host,
        events: Mutex::new(vec![]),
        setup: false,
    };
    host.with_service(&board, &effects, &effects, &effects, |service| {
        service.send("s", "after handoff", false).unwrap();
        assert_eq!(*lock(&effects.events), ["stop", "publish", "revive"]);
        service.ready_now("s");
    });
    assert_eq!(
        *lock(&effects.events),
        [
            "stop",
            "publish",
            "revive",
            "send:false:after handoff",
            "publish"
        ]
    );
}

#[test]
fn account_and_turn_observations_remain_adapter_owned() {
    let host = SessionHost::default();
    lock(&host.conversations).insert(
        "s".into(),
        Conversation {
            changed: true,
            working: true,
            waits: true,
            ..Default::default()
        },
    );
    assert!(host.state("s").waits_for_turn);
    let account = host.account_state("s").unwrap();
    assert!(account.changed && account.working);
    lock(&host.conversations)
        .get_mut("s")
        .unwrap()
        .account_error = Some("login busy".into());
    assert!(matches!(host.account_state("s"), Err(error) if error == "login busy"));
    lock(&host.conversations).get_mut("s").unwrap().stopped = true;
    assert!(!host.account_state("s").unwrap().changed);
    assert!(!host.account_state("missing").unwrap().working);
}
