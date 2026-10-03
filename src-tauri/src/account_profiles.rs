//! Desktop composition of the Tauri-free native profile implementation.
use crate::{claude, codex, i18n, paths};
use prometeu_profiles::{NativeProfiles, ProfileFiles};
use std::path::Path;
use std::sync::Arc;

struct PrivateFiles;
impl ProfileFiles for PrivateFiles {
    fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
        paths::ensure_private_dir(path).map_err(i18n::io)
    }
    fn write_private(&self, path: &Path, body: &str) -> Result<(), String> {
        paths::write_private(path, body).map_err(i18n::io)
    }
}
pub fn native() -> NativeProfiles {
    NativeProfiles::new(
        paths::root(),
        claude::user_home(),
        codex::user_home(),
        paths::home(),
        Arc::new(PrivateFiles),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_profiles::ProfileBackend;
    #[test]
    fn materialized_profile_uses_private_desktop_storage_and_keeps_original_credentials() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("prometeu-profile-storage-{}", uuid::Uuid::new_v4()));
        let base = root.join("codex");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("auth.json"), "original login").unwrap();
        let profiles = NativeProfiles::new(
            root.join("app"),
            root.join("claude"),
            base.clone(),
            root.clone(),
            Arc::new(PrivateFiles),
        );
        let account = prometeu_core::accounts::Account {
            id: uuid::Uuid::new_v4().to_string(),
            provider: crate::state::ProviderId::Codex,
            revision: 0,
            auth_method: Some("browser".into()),
            key_suffix: None,
            identity: Default::default(),
        };
        let profile = profiles.resolve(&account).unwrap();
        profiles.prepare(&profile).unwrap();
        assert_eq!(
            std::fs::metadata(&profile.home)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(profile.home.join("config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!profile.home.join("auth.json").exists());
        assert_eq!(
            std::fs::read_to_string(base.join("auth.json")).unwrap(),
            "original login"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
