//! Account registration and selection, independent of credentials and native profile paths.
pub mod login;

use crate::{board::ProviderId, error::code, lock::lock};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub connected: bool,
    pub email: Option<String>,
    pub plan: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_suffix: Option<String>,
    pub id: String,
    pub provider: ProviderId,
    #[serde(default)]
    pub revision: u64,
    #[serde(flatten)]
    pub identity: Identity,
}

#[derive(Clone, PartialEq)]
pub struct Registry {
    pub accounts: Vec<Account>,
    pub active: BTreeMap<String, String>,
    unknown_accounts: Vec<Value>,
    unknown_active: Map<String, Value>,
}

impl Default for Registry {
    fn default() -> Self {
        let accounts = [ProviderId::Claude, ProviderId::Codex]
            .into_iter()
            .map(|provider| Account {
                auth_method: None,
                key_suffix: None,
                id: key(provider).into(),
                provider,
                revision: 0,
                identity: Identity::default(),
            })
            .collect();
        Self {
            accounts,
            active: BTreeMap::from([
                ("claude".into(), "claude".into()),
                ("codex".into(), "codex".into()),
            ]),
            unknown_accounts: Vec::new(),
            unknown_active: Map::new(),
        }
    }
}

impl Serialize for Registry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut accounts = self
            .accounts
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(serde::ser::Error::custom)?;
        accounts.extend(self.unknown_accounts.iter().cloned());
        let mut active = self.unknown_active.clone();
        active.extend(
            self.active
                .iter()
                .map(|(provider, id)| (provider.clone(), Value::String(id.clone()))),
        );
        let mut state = serializer.serialize_struct("Registry", 2)?;
        state.serialize_field("accounts", &accounts)?;
        state.serialize_field("active", &active)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Registry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("registry must be an object"))?;
        let mut accounts = Vec::new();
        let mut unknown_accounts = Vec::new();
        for value in object
            .get("accounts")
            .and_then(Value::as_array)
            .ok_or_else(|| serde::de::Error::custom("accounts must be an array"))?
        {
            match value.get("provider").and_then(Value::as_str) {
                Some("claude" | "codex" | "antigravity") => accounts
                    .push(serde_json::from_value(value.clone()).map_err(serde::de::Error::custom)?),
                Some(_) => unknown_accounts.push(value.clone()),
                None => {
                    return Err(serde::de::Error::custom(
                        "account provider must be a string",
                    ))
                }
            }
        }
        let mut active = BTreeMap::new();
        let mut unknown_active = Map::new();
        for (provider, value) in object
            .get("active")
            .and_then(Value::as_object)
            .ok_or_else(|| serde::de::Error::custom("active must be an object"))?
        {
            if matches!(provider.as_str(), "claude" | "codex" | "antigravity") {
                active.insert(
                    provider.clone(),
                    value
                        .as_str()
                        .ok_or_else(|| serde::de::Error::custom("active account must be a string"))?
                        .to_owned(),
                );
            } else {
                unknown_active.insert(provider.clone(), value.clone());
            }
        }
        Ok(Self {
            accounts,
            active,
            unknown_accounts,
            unknown_active,
        })
    }
}

impl Registry {
    pub fn active(&self, provider: ProviderId) -> Result<&Account, String> {
        if provider == ProviderId::RetiredGemini {
            return Err(code("err.provider.retired"));
        }
        self.find(
            self.active
                .get(key(provider))
                .ok_or_else(|| code("err.account.noActive"))?,
        )
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for account in &self.accounts {
            if (account.provider == ProviderId::Antigravity
                && (account.id != "antigravity"
                    || account.auth_method.as_deref() != Some("external")))
                || !ids.insert(&account.id)
                || (account.id != key(account.provider)
                    && uuid::Uuid::parse_str(&account.id)
                        .map(|id| id.to_string())
                        .ok()
                        .as_deref()
                        != Some(&account.id))
            {
                return Err(code("err.account.store"));
            }
        }
        for (provider, id) in &self.active {
            if !self
                .accounts
                .iter()
                .any(|account| key(account.provider) == provider && &account.id == id)
            {
                return Err(code("err.account.store"));
            }
        }
        Ok(())
    }

    pub fn find(&self, id: &str) -> Result<&Account, String> {
        self.accounts
            .iter()
            .find(|account| account.id == id)
            .ok_or_else(|| code("err.account.missing"))
    }

    pub fn select(&mut self, id: &str) -> Result<(), String> {
        let account = self.find(id)?;
        if !account.identity.connected && account.id != key(account.provider) {
            return Err(code("err.account.disconnected"));
        }
        self.active.insert(key(account.provider).into(), id.into());
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let account = self.find(id)?;
        let provider = key(account.provider);
        if self.active.get(provider).is_some_and(|active| active == id) {
            self.active.remove(provider);
        }
        self.accounts.retain(|account| account.id != id);
        Ok(())
    }
}

pub fn key(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude",
        ProviderId::Codex => "codex",
        ProviderId::Antigravity => "antigravity",
        ProviderId::RetiredGemini => "gemini",
    }
}

/// Missing storage is distinct from an empty registry. Implementations must preserve unknown
/// provider entries and must not call back into the registry while a save is in progress.
pub trait AccountStore: Send + Sync {
    fn load(&self) -> Result<Option<Registry>, String>;
    fn save(&self, registry: &Registry) -> Result<(), String>;
}

/// One execution host's account state. A load failure remains an error until the host is recreated.
/// Updates are serialized through durable storage before replacing the observable in-memory state.
pub struct AccountRegistry {
    state: Mutex<Result<Registry, String>>,
    store: Arc<dyn AccountStore>,
}

impl AccountRegistry {
    pub fn new(store: Arc<dyn AccountStore>) -> Self {
        let state = store.load().and_then(|loaded| {
            let registry = loaded.unwrap_or_default();
            registry.validate()?;
            Ok(registry)
        });
        Self {
            state: Mutex::new(state),
            store,
        }
    }

    pub fn snapshot(&self) -> Result<Registry, String> {
        lock(&self.state).clone()
    }

    pub fn active(&self, provider: ProviderId) -> Result<Account, String> {
        // Retired identities keep their error even when storage is unavailable.
        if provider == ProviderId::RetiredGemini {
            return Err(code("err.provider.retired"));
        }
        let guard = lock(&self.state);
        guard
            .as_ref()
            .map_err(Clone::clone)?
            .active(provider)
            .cloned()
    }

    pub fn find(&self, id: &str) -> Result<Account, String> {
        let guard = lock(&self.state);
        guard.as_ref().map_err(Clone::clone)?.find(id).cloned()
    }

    pub fn selected_ids(&self) -> Result<Vec<String>, String> {
        let guard = lock(&self.state);
        Ok(guard
            .as_ref()
            .map_err(Clone::clone)?
            .active
            .values()
            .cloned()
            .collect())
    }

    pub fn registered(&self, id: &str, revision: Option<u64>) -> bool {
        lock(&self.state).as_ref().is_ok_and(|data| {
            data.accounts.iter().any(|account| {
                account.id == id && revision.is_none_or(|expected| account.revision == expected)
            })
        })
    }

    /// The update callback must not re-enter this registry. Failure of the callback, validation or
    /// storage leaves memory unchanged; unchanged updates do not write.
    pub fn update(
        &self,
        update: impl FnOnce(&mut Registry) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut guard = lock(&self.state);
        let current = guard.as_mut().map_err(|error| error.clone())?;
        let mut next = current.clone();
        update(&mut next)?;
        next.validate()?;
        if &next == current {
            return Ok(());
        }
        self.store.save(&next)?;
        *current = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
