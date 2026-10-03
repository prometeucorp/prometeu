use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Store {
    saved: Mutex<Option<Registry>>,
    fail_save: AtomicBool,
    writes: AtomicUsize,
}
impl AccountStore for Store {
    fn load(&self) -> Result<Option<Registry>, String> {
        Ok(lock(&self.saved).clone())
    }
    fn save(&self, registry: &Registry) -> Result<(), String> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.fail_save.load(Ordering::SeqCst) {
            return Err("storage unavailable".into());
        }
        *lock(&self.saved) = Some(registry.clone());
        Ok(())
    }
}

#[test]
fn missing_storage_imports_external_accounts_but_empty_storage_stays_empty() {
    let store = Arc::new(Store::default());
    let registry = AccountRegistry::new(store.clone());
    assert_eq!(registry.active(ProviderId::Claude).unwrap().id, "claude");
    assert_eq!(registry.active(ProviderId::Codex).unwrap().id, "codex");
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    registry
        .update(|data| {
            data.remove("claude")?;
            data.remove("codex")
        })
        .unwrap();
    let reopened = AccountRegistry::new(store);
    assert!(reopened.snapshot().unwrap().accounts.is_empty());
    assert_eq!(
        reopened.active(ProviderId::Claude).err(),
        Some(code("err.account.noActive"))
    );
    assert_eq!(
        reopened.active(ProviderId::RetiredGemini).err(),
        Some(code("err.provider.retired"))
    );
}

#[test]
fn failed_update_validation_and_save_preserve_the_previous_state() {
    let store = Arc::new(Store::default());
    let registry = AccountRegistry::new(store.clone());
    let before = registry.snapshot().unwrap();
    assert_eq!(
        registry
            .update(|data| {
                data.remove("claude")?;
                Err("operation failed".into())
            })
            .err(),
        Some("operation failed".into())
    );
    assert_eq!(
        registry
            .update(|data| {
                data.accounts[0].id = "../invalid".into();
                Ok(())
            })
            .err(),
        Some(code("err.account.store"))
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    assert!(registry.snapshot().unwrap() == before);
    store.fail_save.store(true, Ordering::SeqCst);
    assert_eq!(
        registry.update(|data| data.remove("claude")).err(),
        Some("storage unavailable".into())
    );
    assert!(registry.snapshot().unwrap() == before);
    assert!(lock(&store.saved).is_none());
    // No-op updates neither need storage nor report a spurious failure.
    registry.update(|data| data.select("claude")).unwrap();
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    store.fail_save.store(false, Ordering::SeqCst);
    registry.update(|data| data.remove("claude")).unwrap();
    assert!(!AccountRegistry::new(store).registered("claude", None));
}

#[test]
fn failed_load_is_not_replaced_by_defaults_or_overwritten() {
    struct Broken;
    impl AccountStore for Broken {
        fn load(&self) -> Result<Option<Registry>, String> {
            Err("unreadable registry".into())
        }
        fn save(&self, _: &Registry) -> Result<(), String> {
            panic!("must not overwrite unreadable storage")
        }
    }
    let registry = AccountRegistry::new(Arc::new(Broken));
    assert_eq!(
        registry.snapshot().err(),
        Some("unreadable registry".into())
    );
    assert_eq!(
        registry.active(ProviderId::Claude).err(),
        Some("unreadable registry".into())
    );
    assert_eq!(
        registry.active(ProviderId::RetiredGemini).err(),
        Some(code("err.provider.retired"))
    );
    assert_eq!(
        registry
            .update(|_| panic!("must not update failed state"))
            .err(),
        Some("unreadable registry".into())
    );
    assert!(!registry.registered("claude", None));
}

#[test]
fn invalid_loaded_registry_cannot_be_used_or_repaired_by_an_update() {
    let mut invalid = Registry::default();
    invalid.active.insert("codex".into(), "claude".into());
    let store = Arc::new(Store {
        saved: Mutex::new(Some(invalid)),
        ..Default::default()
    });
    let registry = AccountRegistry::new(store.clone());
    assert_eq!(registry.snapshot().err(), Some(code("err.account.store")));
    assert!(registry
        .update(|_| panic!("invalid registry cannot be changed"))
        .is_err());
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn revision_changes_invalidate_captured_registrations_and_selection() {
    let registry = AccountRegistry::new(Arc::new(Store::default()));
    let captured = registry.active(ProviderId::Claude).unwrap();
    assert!(registry.registered(&captured.id, Some(captured.revision)));
    registry
        .update(|data| {
            data.accounts[0].revision += 1;
            data.remove("codex")
        })
        .unwrap();
    assert!(!registry.registered(&captured.id, Some(captured.revision)));
    assert!(registry.registered(&captured.id, None));
    assert_eq!(registry.selected_ids().unwrap(), vec!["claude"]);
    assert_eq!(captured.revision, 0);
}

#[test]
fn concurrent_changes_preserve_all_commits_in_memory_and_storage() {
    let store = Arc::new(Store::default());
    let registry = Arc::new(AccountRegistry::new(store.clone()));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            let registry = registry.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                registry
                    .update(|data| {
                        data.accounts[0].revision += 1;
                        Ok(())
                    })
                    .unwrap();
            })
        })
        .collect();
    for job in jobs {
        job.join().unwrap();
    }
    assert_eq!(registry.find("claude").unwrap().revision, 8);
    assert_eq!(store.writes.load(Ordering::SeqCst), 8);
    assert_eq!(
        AccountRegistry::new(store).find("claude").unwrap().revision,
        8
    );
}

#[test]
fn legacy_and_unknown_provider_entries_survive_a_known_account_change() {
    let unknown =
        json!({"id":"future","provider":"future","plan":{"tier":2},"privateMetadata":[1,2]});
    let retired = json!({"id":"old-gemini","provider":"gemini","authMethod":"apiKey"});
    let initial: Registry = serde_json::from_value(json!({
        "accounts":[{"id":"claude","provider":"claude","connected":false,"label":"old nickname"},unknown,retired],
        "active":{"claude":"claude","future":{"opaque":true},"gemini":"old-gemini"}
    })).unwrap();
    let store = Arc::new(Store {
        saved: Mutex::new(Some(initial)),
        ..Default::default()
    });
    let registry = AccountRegistry::new(store.clone());
    assert_eq!(registry.find("claude").unwrap().revision, 0);
    assert_eq!(registry.snapshot().unwrap().accounts.len(), 1);
    assert!(!registry.registered("future", None));
    assert!(registry.update(|data| data.select("old-gemini")).is_err());
    registry.update(|data| data.select("claude")).unwrap();
    registry.update(|data| data.remove("claude")).unwrap();
    let stored = serde_json::to_value(AccountRegistry::new(store).snapshot().unwrap()).unwrap();
    assert_eq!(stored["accounts"], json!([unknown, retired]));
    assert_eq!(
        stored["active"],
        json!({"future":{"opaque":true},"gemini":"old-gemini"})
    );
}
