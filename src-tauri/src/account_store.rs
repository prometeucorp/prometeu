#[cfg(test)]
use crate::i18n;
pub use prometeu_files::accounts::FileAccountStore;

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::accounts::AccountRegistry;
    use std::sync::Arc;

    #[test]
    fn registry_changes_persist_in_a_private_root() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("prometeu-account-store-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(FileAccountStore::new(root.join("accounts")));
        AccountRegistry::new(store.clone())
            .update(|data| data.remove("claude"))
            .unwrap();
        let reloaded = AccountRegistry::new(store);
        assert!(!reloaded.registered("claude", None));
        assert!(reloaded.registered("codex", None));
        assert_eq!(
            std::fs::metadata(root.join("accounts/accounts.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(root.join("accounts"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_or_unreadable_storage_is_never_overwritten() {
        let root =
            std::env::temp_dir().join(format!("prometeu-account-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("accounts.json");
        std::fs::write(&path, "{truncated").unwrap();
        let registry = AccountRegistry::new(Arc::new(FileAccountStore::new(root.clone())));
        assert_eq!(
            registry.snapshot().err(),
            Some(i18n::t("err.account.store"))
        );
        assert!(registry.update(|data| data.remove("claude")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{truncated");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let registry = AccountRegistry::new(Arc::new(FileAccountStore::new(root.clone())));
        assert!(registry.snapshot().is_err());
        assert!(registry.update(|data| data.remove("claude")).is_err());
        assert!(path.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }
}
