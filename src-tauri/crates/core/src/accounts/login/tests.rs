use super::*;
use crate::accounts::AccountStore;

const ID: &str = "00000000-0000-4000-8000-000000000001";
#[derive(Default)]
struct Store {
    fail: AtomicBool,
}
impl AccountStore for Store {
    fn load(&self) -> Result<Option<Registry>, String> {
        Ok(None)
    }
    fn save(&self, _: &Registry) -> Result<(), String> {
        if self.fail.load(Ordering::Relaxed) {
            Err("save failed".into())
        } else {
            Ok(())
        }
    }
}
struct Fixture {
    store: Arc<Store>,
    registry: AccountRegistry,
    pending: LoginState,
}
impl Fixture {
    fn new() -> Self {
        let store = Arc::new(Store::default());
        Self {
            registry: AccountRegistry::new(store.clone()),
            store,
            pending: LoginState::new(),
        }
    }
    fn service<'a>(&'a self, effects: &'a dyn LoginEffects) -> LoginService<'a> {
        LoginService {
            registry: &self.registry,
            pending: &self.pending,
            effects,
        }
    }
    fn effects(&self, working: bool) -> Effects<'_> {
        Effects {
            pending: &self.pending,
            registry: &self.registry,
            working,
            events: Mutex::new(vec![]),
        }
    }
}
struct Effects<'a> {
    pending: &'a LoginState,
    registry: &'a AccountRegistry,
    working: bool,
    events: Mutex<Vec<String>>,
}
impl LoginEffects for Effects<'_> {
    fn working(&self, id: &str) -> bool {
        assert!(self.pending.logging_in(id));
        lock(&self.events).push("working".into());
        self.working
    }
    fn refresh(&self, provider: Option<ProviderId>) {
        lock(&self.events).push(format!(
            "refresh:{provider:?}:{}",
            self.pending.snapshot().is_some()
        ));
    }
    fn forget(&self, id: &str) {
        // Host effects can inspect both states; no core lock may be held here.
        let _ = self.registry.snapshot().unwrap();
        let _ = self.pending.snapshot();
        lock(&self.events).push(format!("forget:{id}"));
    }
    fn publish(&self) {
        let _ = self.registry.snapshot().unwrap();
        lock(&self.events).push(format!("publish:{}", self.pending.snapshot().is_some()));
    }
}
struct Auth<F>(F);
impl<F> AccountAuthentication for Auth<F>
where
    F: Fn(&Account, Arc<AtomicBool>) -> Result<Identity, String> + Send + Sync,
{
    fn authenticate(&self, account: &Account, cancel: Arc<AtomicBool>) -> Result<Identity, String> {
        (self.0)(account, cancel)
    }
}
fn identity() -> Identity {
    Identity {
        connected: true,
        email: Some("person@example.com".into()),
        plan: Some("pro".into()),
    }
}
fn begin(service: &LoginService<'_>) -> LoginAttempt {
    service
        .begin(ProviderId::Claude, ID.into(), "browser".into())
        .unwrap()
}

#[test]
fn successful_login_commits_revision_before_forgetting_quota_and_finishes_after_native_work() {
    let fixture = Fixture::new();
    let effects = fixture.effects(false);
    let service = fixture.service(&effects);
    let attempt = begin(&service);
    assert!(!fixture.registry.find(ID).unwrap().identity.connected);
    assert_eq!(
        serde_json::to_value(fixture.pending.snapshot()).unwrap(),
        serde_json::json!({"id":ID,"provider":"claude"})
    );
    assert_eq!(
        fixture.registry.active(ProviderId::Claude).unwrap().id,
        "claude"
    );
    service
        .authenticate(
            &attempt,
            &Auth(|account: &Account, cancel: Arc<AtomicBool>| {
                assert_eq!(account.id, ID);
                assert_eq!(account.revision, 0);
                assert!(!cancel.load(Ordering::Relaxed));
                Ok(identity())
            }),
        )
        .unwrap();
    assert!(fixture.pending.logging_in(ID));
    assert_eq!(fixture.registry.find(ID).unwrap().revision, 1);
    assert!(fixture.registry.find(ID).unwrap().identity == identity());
    service.finish(&attempt);
    assert!(fixture.pending.snapshot().is_none());
    assert_eq!(
        *lock(&effects.events),
        vec![
            "refresh:Some(Claude):true",
            "working",
            "publish:true",
            &format!("forget:{ID}"),
            "refresh:Some(Claude):false",
            "publish:false",
        ]
    );
    // Reconnection of the same ID bumps its captured revision without changing selection.
    let attempt = begin(&service);
    service
        .authenticate(
            &attempt,
            &Auth(|_: &Account, _: Arc<AtomicBool>| Ok(identity())),
        )
        .unwrap();
    service.finish(&attempt);
    assert_eq!(fixture.registry.find(ID).unwrap().revision, 2);
    assert_eq!(
        fixture.registry.active(ProviderId::Claude).unwrap().id,
        "claude"
    );
}

#[test]
fn working_account_is_refused_after_reservation_and_quota_invalidation_then_reenabled() {
    let fixture = Fixture::new();
    let effects = fixture.effects(true);
    let service = fixture.service(&effects);
    assert_eq!(
        service
            .begin(ProviderId::Claude, ID.into(), "browser".into())
            .err(),
        Some(code("err.account.working"))
    );
    assert!(fixture.pending.snapshot().is_none());
    assert_eq!(fixture.registry.find(ID).unwrap().revision, 0);
    assert_eq!(
        *lock(&effects.events),
        vec![
            "refresh:Some(Claude):true",
            "working",
            "refresh:Some(Claude):false",
            "publish:false"
        ]
    );
}

#[test]
fn authentication_cancellation_and_commit_errors_keep_registration_and_resume_quota_work() {
    for failure in ["authentication", "cancellation", "storage"] {
        let fixture = Fixture::new();
        let effects = fixture.effects(false);
        let service = fixture.service(&effects);
        let attempt = begin(&service);
        let before = fixture.registry.snapshot().unwrap();
        fixture
            .store
            .fail
            .store(failure == "storage", Ordering::Relaxed);
        let result = service.authenticate(
            &attempt,
            &Auth(|_: &Account, cancel: Arc<AtomicBool>| {
                fixture.pending.cancel("unrelated");
                assert!(!cancel.load(Ordering::Relaxed));
                match failure {
                    "authentication" => Err("authentication failed".into()),
                    "cancellation" => {
                        fixture.pending.cancel(ID);
                        Ok(identity())
                    }
                    _ => Ok(identity()),
                }
            }),
        );
        let expected = match failure {
            "authentication" => "authentication failed".into(),
            "cancellation" => code("err.account.cancelled"),
            _ => "save failed".into(),
        };
        assert_eq!(result.err(), Some(expected));
        assert!(fixture.registry.snapshot().unwrap() == before);
        assert!(fixture.pending.logging_in(ID));
        service.finish(&attempt);
        assert!(fixture.pending.snapshot().is_none());
        assert!(!lock(&effects.events)
            .iter()
            .any(|event| event.starts_with("forget:")));
        assert_eq!(
            &lock(&effects.events)[3..],
            ["refresh:Some(Claude):false", "publish:false"]
        );
    }
}

#[test]
fn pending_login_blocks_removal_attachment_and_same_account_selection() {
    let fixture = Fixture::new();
    let effects = fixture.effects(false);
    let service = fixture.service(&effects);
    let attempt = begin(&service);
    let busy = Some(code("err.account.busy"));
    assert_eq!(
        service
            .begin(ProviderId::Codex, ID.into(), "browser".into())
            .err(),
        busy
    );
    assert_eq!(service.select(ID).err(), busy);
    assert_eq!(service.remove("codex").err(), busy);
    assert_eq!(service.attach(external()).err(), busy);
    service.select("codex").unwrap();
    service.finish(&attempt);
    service.remove(ID).unwrap();
    assert!(!fixture.registry.registered(ID, None));
}
fn external() -> Account {
    Account {
        id: "antigravity".into(),
        provider: ProviderId::Antigravity,
        revision: 0,
        auth_method: Some("external".into()),
        key_suffix: None,
        identity: Identity::default(),
    }
}

#[test]
fn external_attachment_does_not_authenticate_or_select_and_is_idempotent() {
    let fixture = Fixture::new();
    let effects = fixture.effects(false);
    let service = fixture.service(&effects);
    service.attach(external()).unwrap();
    fixture.store.fail.store(true, Ordering::Relaxed);
    service.attach(external()).unwrap();
    assert_eq!(fixture.registry.find("antigravity").unwrap().revision, 0);
    assert_eq!(
        fixture.registry.active(ProviderId::Antigravity).err(),
        Some(code("err.account.noActive"))
    );
    assert!(fixture.pending.snapshot().is_none());
    assert_eq!(
        *lock(&effects.events),
        vec!["publish:false", "publish:false"]
    );
}

#[test]
fn registration_errors_leave_no_pending_login_or_effects() {
    let fixture = Fixture::new();
    let effects = fixture.effects(false);
    let service = fixture.service(&effects);
    assert_eq!(
        service
            .begin(ProviderId::Claude, "claude".into(), "browser".into())
            .err(),
        Some(code("err.account.external"))
    );
    assert_eq!(
        service
            .begin(ProviderId::Codex, "claude".into(), "browser".into())
            .err(),
        Some(code("err.account.provider"))
    );
    assert_eq!(
        service
            .begin(ProviderId::Claude, "../invalid".into(), "browser".into())
            .err(),
        Some(code("err.account.store"))
    );
    fixture.store.fail.store(true, Ordering::Relaxed);
    assert_eq!(
        service
            .begin(ProviderId::Claude, ID.into(), "browser".into())
            .err(),
        Some("save failed".into())
    );
    assert!(fixture.pending.snapshot().is_none());
    assert!(lock(&effects.events).is_empty());
    fixture.store.fail.store(false, Ordering::Relaxed);
    let attempt = begin(&service);
    service.finish(&attempt);
    assert_eq!(
        service
            .begin(ProviderId::Claude, ID.into(), "different-method".into())
            .err(),
        Some(code("err.account.provider"))
    );
    assert!(fixture.pending.snapshot().is_none());
}

#[test]
fn abandoned_worker_cleanup_is_idempotent_and_cannot_clear_a_later_attempt() {
    let fixture = Fixture::new();
    let effects = fixture.effects(false);
    let service = fixture.service(&effects);
    let old = begin(&service);
    // The host also finishes an attempt when its executor rejects or panics.
    service.finish(&old);
    let current = begin(&service);
    let count = lock(&effects.events).len();
    service.finish(&old);
    assert_eq!(lock(&effects.events).len(), count);
    assert!(fixture.pending.logging_in(ID));
    service.finish(&current);
    let count = lock(&effects.events).len();
    service.finish(&current);
    assert_eq!(lock(&effects.events).len(), count);
}

#[test]
fn cancel_all_signals_the_pending_attempt_which_stays_admitted_until_finished() {
    let fixture = Fixture::new();
    let effects = fixture.effects(false);
    let service = fixture.service(&effects);
    let attempt = begin(&service);
    fixture.pending.cancel_all();
    assert!(attempt.cancel.load(Ordering::Relaxed));
    assert!(fixture.pending.logging_in(ID));
    service.finish(&attempt);
    assert!(!fixture.pending.logging_in(ID));
}
