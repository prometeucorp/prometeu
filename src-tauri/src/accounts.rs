//! Accounts on this Mac. Selection is global per provider; each process captures an immutable
//! profile and changes accounts only between turns.

use crate::lock::lock;
use crate::state::ProviderId;
use crate::{i18n, paths};
use prometeu_profiles::ProfileBackend;
use serde::ser::SerializeStruct;
use serde::Serialize;
use serde_json::Value;
#[cfg(test)]
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use prometeu_core::accounts::AccountRegistry;
pub use prometeu_core::accounts::{Account, Identity, Registry};

fn provider(value: &str) -> Result<ProviderId, String> {
    match value {
        "claude" => Ok(ProviderId::Claude),
        "codex" => Ok(ProviderId::Codex),
        "antigravity" => Ok(ProviderId::Antigravity),
        _ => Err(i18n::t("err.account.provider")),
    }
}

fn registry() -> &'static AccountRegistry {
    static REGISTRY: OnceLock<AccountRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        AccountRegistry::new(Arc::new(crate::account_store::FileAccountStore::new(
            paths::root(),
        )))
    })
}

#[cfg(test)]
fn read(path: &Path) -> Result<Registry, String> {
    use prometeu_core::accounts::AccountStore;
    let registry =
        crate::account_store::FileAccountStore::new(path.parent().unwrap().to_path_buf())
            .load()?
            .unwrap_or_default();
    registry.validate()?;
    Ok(registry)
}

fn change(update: impl FnOnce(&mut Registry) -> Result<(), String>) -> Result<(), String> {
    registry().update(update)
}

pub use prometeu_profiles::Profile;

pub fn prepare_profile(profile: &Profile) -> Result<(), String> {
    crate::account_profiles::native().prepare(profile)
}
pub fn apply_profile(profile: &Profile, command: &mut Command) -> Result<(), String> {
    crate::account_profiles::native().apply(profile, command)
}
pub(crate) fn selected(provider: ProviderId) -> Result<Account, String> {
    registry().active(provider)
}

pub fn active(provider: ProviderId) -> Result<Profile, String> {
    crate::account_profiles::native().resolve(&selected(provider)?)
}

pub fn profiles() -> Result<Vec<Profile>, String> {
    let profiles = crate::account_profiles::native();
    registry()
        .snapshot()?
        .accounts
        .iter()
        .map(|account| profiles.resolve(account))
        .collect()
}

pub fn selected_ids() -> Vec<String> {
    registry().selected_ids().unwrap_or_default()
}

pub fn registered(id: &str, revision: Option<u64>) -> bool {
    registry().registered(id, revision)
}

use prometeu_core::accounts::login::{LoginEffects, LoginService, LoginState, LoginStatus};

fn pending() -> &'static LoginState {
    static LOGIN: LoginState = LoginState::new();
    &LOGIN
}

struct DesktopLogin<'a>(&'a AppHandle);
impl DesktopLogin<'_> {
    fn service(&self) -> LoginService<'_> {
        LoginService {
            registry: registry(),
            pending: pending(),
            effects: self,
        }
    }
}
impl LoginEffects for DesktopLogin<'_> {
    fn working(&self, id: &str) -> bool {
        lock(&self.0.state::<crate::AppState>().sessions.conversations)
            .values()
            .any(|chat| chat.account() == id && chat.working())
    }
    fn refresh(&self, provider: Option<ProviderId>) {
        crate::usage::refresh(self.0, provider);
    }
    fn forget(&self, id: &str) {
        crate::usage::forget(self.0, id);
    }
    fn publish(&self) {
        publish(self.0);
    }
}

#[derive(Clone)]
pub struct Snapshot {
    registry: Registry,
    login: Option<LoginStatus>,
}

impl Serialize for Snapshot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("Snapshot", 3)?;
        state.serialize_field("accounts", &self.registry.accounts)?;
        state.serialize_field("active", &self.registry.active)?;
        state.serialize_field("login", &self.login)?;
        state.end()
    }
}

#[tauri::command]
pub fn accounts() -> Result<Snapshot, String> {
    let data = registry().snapshot()?;
    let login = pending().snapshot();
    Ok(Snapshot {
        registry: data,
        login,
    })
}

pub fn publish(app: &AppHandle) {
    if let Ok(snapshot) = accounts() {
        let _ = app.emit("accounts", snapshot);
    }
}

#[tauri::command]
pub fn account_select(app: AppHandle, id: String) -> Result<Snapshot, String> {
    DesktopLogin(&app).service().select(&id)?;
    accounts()
}

#[tauri::command]
pub fn account_remove(app: AppHandle, id: String) -> Result<Snapshot, String> {
    DesktopLogin(&app).service().remove(&id)?;
    accounts()
}

#[tauri::command]
pub fn account_login_cancel(id: String) {
    pending().cancel(&id);
}

#[tauri::command]
pub async fn account_login(
    app: AppHandle,
    provider: String,
    id: Option<String>,
    method: Option<String>,
) -> Result<Snapshot, String> {
    let provider = self::provider(&provider)?;
    if provider == ProviderId::Antigravity {
        if method.as_deref().is_some_and(|m| m != "external")
            || id.as_deref().is_some_and(|id| id != "antigravity")
        {
            return Err(i18n::t("err.account.external"));
        }
        if !crate::antigravity::installed() {
            return Err(i18n::t("err.antigravity.version"));
        }
        DesktopLogin(&app).service().attach(Account {
            id: "antigravity".into(),
            provider,
            revision: 0,
            auth_method: Some("external".into()),
            key_suffix: None,
            identity: Identity::default(),
        })?;
        return accounts();
    }
    let method = method.unwrap_or_else(|| "browser".into());
    if method != "browser" {
        return Err(i18n::t("err.account.provider"));
    }
    let attempt = DesktopLogin(&app).service().begin(
        provider,
        id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        method,
    )?;
    let handle = app.clone();
    let worker_attempt = attempt.clone();
    let authentication = app.state::<crate::AppState>().authentication.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        DesktopLogin(&handle)
            .service()
            .authenticate(&worker_attempt, authentication.as_ref())
    })
    .await
    .map_err(i18n::io)
    .and_then(|result| result);
    DesktopLogin(&app).service().finish(&attempt);
    result?;
    accounts()
}

pub fn set_identity(app: &AppHandle, id: &str, identity: Identity) -> Result<(), String> {
    if registry().find(id)?.identity == identity {
        return Ok(());
    }
    change(|data| {
        let account = data
            .accounts
            .iter_mut()
            .find(|account| account.id == id)
            .ok_or_else(|| i18n::t("err.account.missing"))?;
        account.identity = identity;
        Ok(())
    })?;
    publish(app);
    Ok(())
}

pub fn logging_in(id: &str) -> bool {
    pending().logging_in(id)
}

pub fn shutdown() {
    pending().cancel_all();
    let deadline = Instant::now() + Duration::from_secs(2);
    while pending().snapshot().is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Private authentication protocol adapter over an injected subprocess port.
pub struct AuthProcess(prometeu_core::auxiliary::AuxiliarySession);

impl AuthProcess {
    pub fn spawn(
        launcher: &dyn prometeu_core::auxiliary::AuxiliaryLauncher<Command>,
        command: Command,
        timeout: Duration,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let process = launcher.launch(command).map_err(i18n::io)?;
        Ok(Self(prometeu_core::auxiliary::AuxiliarySession::new(
            process, timeout, cancel,
        )))
    }
    pub fn send(&mut self, value: &Value) -> Result<(), String> {
        self.0
            .send(format!("{value}\n").as_bytes())
            .map_err(auth_error)
    }
    pub fn line(&mut self) -> Result<Option<String>, String> {
        self.0.line().map_err(auth_error)
    }
    pub fn finish(&mut self) -> Result<(), String> {
        self.0.finish().map_err(auth_error)
    }
}
fn auth_error(error: prometeu_core::auxiliary::AuxiliaryError) -> String {
    use prometeu_core::auxiliary::AuxiliaryError;
    match error {
        AuxiliaryError::Cancelled => i18n::t("err.account.cancelled"),
        AuxiliaryError::Timeout => i18n::t("err.account.timeout"),
        AuxiliaryError::Failed => i18n::t("err.account.login"),
        AuxiliaryError::Io(error) => i18n::io(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_registration_preserves_cli_and_persists_independent_selection() {
        let dir = std::env::temp_dir().join(format!("prometeu-accounts-{}", uuid::Uuid::new_v4()));
        let path = dir.join("accounts.json");
        let mut registry = read(&path).unwrap();
        assert_eq!(registry.active["claude"], "claude");
        assert_eq!(registry.active["codex"], "codex");
        let id = uuid::Uuid::new_v4().to_string();
        registry.accounts.push(Account {
            auth_method: None,
            key_suffix: None,
            id: id.clone(),
            provider: ProviderId::Codex,
            revision: 1,
            identity: Identity {
                connected: true,
                email: Some("work@example.com".into()),
                plan: Some("pro".into()),
            },
        });
        registry.select(&id).unwrap();
        assert_eq!(registry.active["claude"], "claude");
        assert_eq!(registry.active["codex"], id);
        paths::write_private(&path, &serde_json::to_string(&registry).unwrap()).unwrap();
        let loaded = read(&path).unwrap();
        assert!(loaded == registry);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        registry.accounts.last_mut().unwrap().identity.connected = false;
        assert!(registry.select(&id).is_err());
        registry.accounts.last_mut().unwrap().id = "../fora".into();
        assert!(registry.validate().is_err());
        std::fs::write(&path, "cadastro truncado").unwrap();
        assert!(read(&path).is_err());
        assert!(provider("desconhecido").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn removal_accepts_all_accounts_and_legacy_registration_with_nickname() {
        let mut legacy = serde_json::to_value(Registry::default()).unwrap();
        let codex = uuid::Uuid::new_v4().to_string();
        let claude = uuid::Uuid::new_v4().to_string();
        for (id, provider) in [(&codex, "codex"), (&claude, "claude")] {
            legacy["accounts"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "id": id, "provider": provider, "label": "apelido antigo",
                    "connected": true, "email": "personal@example.com", "plan": "pro"
                }));
        }
        let mut registry: Registry = serde_json::from_value(legacy).unwrap();
        registry.validate().unwrap();
        assert!(serde_json::to_value(&registry).unwrap()["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|account| account.get("label").is_none()));
        registry.select(&claude).unwrap();
        let before = registry.clone();
        assert!(registry.remove("../fora").is_err());
        assert!(registry == before);

        registry.remove(&codex).unwrap();
        assert_eq!(registry.active["codex"], "codex");
        assert_eq!(registry.active["claude"], claude);
        assert!(registry.find(&codex).is_err());
        registry.remove(&claude).unwrap();
        assert!(!registry.active.contains_key("claude"));
        assert_eq!(registry.accounts.len(), 2);
        registry.validate().unwrap();
        registry.remove("claude").unwrap();
        registry.remove("codex").unwrap();
        assert!(registry.accounts.is_empty());
        assert!(registry.active.is_empty());
        registry.validate().unwrap();

        let dir = std::env::temp_dir().join(format!("prometeu-accounts-{}", uuid::Uuid::new_v4()));
        let path = dir.join("accounts.json");
        paths::write_private(&path, &serde_json::to_string(&registry).unwrap()).unwrap();
        let restored = read(&path).unwrap();
        assert!(restored == registry);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn future_registration_roundtrips_without_appearing_in_the_interface() {
        let dir = std::env::temp_dir().join(format!("prometeu-accounts-{}", uuid::Uuid::new_v4()));
        let path = dir.join("accounts.json");
        let future = serde_json::json!({
            "id": "future-provider",
            "provider": "future-provider",
            "connected": true,
            "email": "future@example.com",
            "plan": { "tier": "ultra" },
            "futureField": [1, 2, 3]
        });
        paths::write_private(
            &path,
            &serde_json::json!({
                "accounts": [
                    {
                        "id": "claude", "provider": "claude", "revision": 0,
                        "connected": false, "email": null, "plan": null
                    },
                    future.clone()
                ],
                "active": { "claude": "claude", "future-provider": "future-provider" }
            })
            .to_string(),
        )
        .unwrap();

        let mut registry = read(&path).unwrap();
        assert_eq!(registry.accounts.len(), 1);
        assert_eq!(registry.active.len(), 1);
        assert_eq!(
            serde_json::to_value(Snapshot {
                registry: registry.clone(),
                login: None,
            })
            .unwrap(),
            serde_json::json!({
                "accounts": [{
                    "id": "claude", "provider": "claude", "revision": 0,
                    "connected": false, "email": null, "plan": null
                }],
                "active": { "claude": "claude" },
                "login": null
            })
        );
        registry.remove("claude").unwrap();

        let stored = serde_json::to_value(&registry).unwrap();
        assert_eq!(stored["accounts"], serde_json::json!([future]));
        assert_eq!(
            stored["active"],
            serde_json::json!({ "future-provider": "future-provider" })
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn future_registration_does_not_relax_known_account_validation() {
        let dir = std::env::temp_dir().join(format!("prometeu-accounts-{}", uuid::Uuid::new_v4()));
        let path = dir.join("accounts.json");
        paths::write_private(
            &path,
            &serde_json::json!({
                "accounts": [
                    { "provider": "claude", "connected": false },
                    { "provider": "future-provider", "payload": { "version": 2 } }
                ],
                "active": {}
            })
            .to_string(),
        )
        .unwrap();

        assert!(read(&path).is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelled_login_terminates_the_process_without_returning_private_output() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut command = Command::new("sh");
        command.args(["-c", "echo $$; printf 'ready\\n'; exec sleep 30"]);
        let mut process = AuthProcess::spawn(
            &prometeu_process::auxiliary::UnixAuxiliaryLauncher,
            command,
            Duration::from_secs(5),
            cancel.clone(),
        )
        .unwrap();
        let pid: u32 = process.line().unwrap().unwrap().parse().unwrap();
        assert_eq!(process.line().unwrap().as_deref(), Some("ready"));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            process.line().unwrap_err(),
            i18n::t("err.account.cancelled")
        );
        drop(process);
        // Drop reaps the child so no orphaned login server remains.
        assert_eq!(
            unsafe { libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
    }
    #[test]
    fn external_antigravity_registration_is_explicit_and_removable() {
        let mut registry = Registry::default();
        assert!(!registry.active.contains_key("antigravity"));
        registry.accounts.push(Account {
            id: "antigravity".into(),
            provider: ProviderId::Antigravity,
            revision: 0,
            auth_method: Some("external".into()),
            key_suffix: None,
            identity: Identity::default(),
        });
        registry.validate().unwrap();
        assert!(!registry.active.contains_key("antigravity"));
        registry.select("antigravity").unwrap();
        let restored: Registry =
            serde_json::from_value(serde_json::to_value(&registry).unwrap()).unwrap();
        assert!(restored == registry);
        registry.remove("antigravity").unwrap();
        assert!(!registry.active.contains_key("antigravity"));
    }

    #[test]
    fn retired_gemini_credentials_are_preserved_but_never_exposed_or_selected() {
        let value = serde_json::json!({"accounts":[{"id":"old-gemini","provider":"gemini","authMethod":"apiKey","keySuffix":"1234"}],"active":{"gemini":"old-gemini"}});
        let mut registry: Registry = serde_json::from_value(value.clone()).unwrap();
        registry.validate().unwrap();
        assert!(registry.accounts.is_empty());
        assert!(registry.select("old-gemini").is_err());
        let snapshot = serde_json::to_value(Snapshot {
            registry: registry.clone(),
            login: None,
        })
        .unwrap();
        assert_eq!(snapshot["accounts"], serde_json::json!([]));
        assert_eq!(snapshot["active"], serde_json::json!({}));
        assert_eq!(serde_json::to_value(registry).unwrap(), value);
        assert!(provider("gemini").is_err());
    }
}
