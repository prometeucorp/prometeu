//! Native authentication dispatch. The host injects this implementation into login coordination.
use crate::{accounts, claude, codex, i18n, state::ProviderId};
use prometeu_core::accounts::login::AccountAuthentication;
use prometeu_core::accounts::{Account, Identity};
use std::sync::{atomic::AtomicBool, Arc};

type Authenticate = fn(&accounts::Profile, Arc<AtomicBool>) -> Result<Identity, String>;
const AUTHENTICATION: &[(ProviderId, Authenticate)] = &[
    (ProviderId::Claude, claude::login),
    (ProviderId::Codex, codex::login),
];
pub struct NativeAuthentication {
    pub profiles: Arc<dyn prometeu_profiles::ProfileBackend>,
}
impl AccountAuthentication for NativeAuthentication {
    fn authenticate(&self, account: &Account, cancel: Arc<AtomicBool>) -> Result<Identity, String> {
        let authenticate = AUTHENTICATION
            .iter()
            .find(|(provider, _)| *provider == account.provider)
            .map(|(_, authenticate)| authenticate)
            .ok_or_else(|| i18n::t("err.account.external"))?;
        let profile = self.profiles.resolve(account)?;
        self.profiles.prepare(&profile)?;
        authenticate(&profile, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct RefusePreparation;
    impl prometeu_profiles::ProfileBackend for RefusePreparation {
        fn resolve(&self, account: &Account) -> Result<accounts::Profile, String> {
            Ok(accounts::Profile {
                id: account.id.clone(),
                provider: account.provider,
                home: "/unused-profile".into(),
                managed: true,
                revision: account.revision,
            })
        }
        fn prepare(&self, _: &accounts::Profile) -> Result<(), String> {
            Err("profile preparation refused".into())
        }
        fn apply(
            &self,
            _: &accounts::Profile,
            _: &mut std::process::Command,
        ) -> Result<(), String> {
            panic!("authentication must stop after profile preparation fails")
        }
    }
    #[test]
    fn injected_profile_failure_prevents_native_authentication() {
        let authentication = NativeAuthentication {
            profiles: Arc::new(RefusePreparation),
        };
        let account = Account {
            id: uuid::Uuid::new_v4().to_string(),
            provider: ProviderId::Claude,
            revision: 0,
            auth_method: Some("browser".into()),
            key_suffix: None,
            identity: Identity::default(),
        };
        assert_eq!(
            authentication
                .authenticate(&account, Arc::new(AtomicBool::new(false)))
                .err(),
            Some("profile preparation refused".into())
        );
    }
}
