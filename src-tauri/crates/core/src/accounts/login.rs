//! Account login lifecycle. Authentication and host observations are injected effects.
use super::{key, Account, AccountRegistry, Identity, Registry};
use crate::{board::ProviderId, error::code, lock::lock};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Serialize)]
pub struct LoginStatus {
    pub id: String,
    pub provider: ProviderId,
}

#[derive(Clone)]
pub struct LoginAttempt {
    account: Account,
    cancel: Arc<AtomicBool>,
}

pub struct LoginState(Mutex<Option<LoginAttempt>>);
impl Default for LoginState {
    fn default() -> Self {
        Self::new()
    }
}
impl LoginState {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }
    pub fn snapshot(&self) -> Option<LoginStatus> {
        lock(&self.0).as_ref().map(|attempt| LoginStatus {
            id: attempt.account.id.clone(),
            provider: attempt.account.provider,
        })
    }
    pub fn logging_in(&self, id: &str) -> bool {
        lock(&self.0)
            .as_ref()
            .is_some_and(|attempt| attempt.account.id == id)
    }
    pub fn cancel(&self, id: &str) {
        if let Some(attempt) = lock(&self.0)
            .as_ref()
            .filter(|attempt| attempt.account.id == id)
        {
            attempt.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn cancel_all(&self) {
        if let Some(attempt) = lock(&self.0).as_ref() {
            attempt.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn while_idle<T>(&self, run: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let pending = lock(&self.0);
        if pending.is_some() {
            return Err(code("err.account.busy"));
        }
        run()
    }
    fn begin(
        &self,
        registry: &AccountRegistry,
        provider: ProviderId,
        id: String,
        method: String,
    ) -> Result<LoginAttempt, String> {
        let mut pending = lock(&self.0);
        if pending.is_some() {
            return Err(code("err.account.busy"));
        }
        registry.update(|data| {
            match data.find(&id) {
                Ok(account) => {
                    if account.provider != provider
                        || account
                            .auth_method
                            .as_deref()
                            .is_some_and(|old| old != method)
                    {
                        return Err(code("err.account.provider"));
                    }
                    if account.id == key(provider) {
                        return Err(code("err.account.external"));
                    }
                }
                Err(_) => data.accounts.push(Account {
                    id: id.clone(),
                    provider,
                    revision: 0,
                    auth_method: Some(method),
                    key_suffix: None,
                    identity: Identity::default(),
                }),
            }
            Ok(())
        })?;
        let attempt = LoginAttempt {
            account: registry.find(&id)?,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        *pending = Some(attempt.clone());
        Ok(attempt)
    }
    fn finish(&self, attempt: &LoginAttempt) -> bool {
        let mut pending = lock(&self.0);
        if pending
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(&current.cancel, &attempt.cancel))
        {
            *pending = None;
            return true;
        }
        false
    }
}

pub trait AccountAuthentication: Send + Sync {
    /// Native implementations prepare the captured profile and own private protocol output.
    fn authenticate(&self, account: &Account, cancel: Arc<AtomicBool>) -> Result<Identity, String>;
}

pub trait LoginEffects {
    fn working(&self, id: &str) -> bool;
    fn refresh(&self, provider: Option<ProviderId>);
    fn forget(&self, id: &str);
    fn publish(&self);
}

pub struct LoginService<'a> {
    pub registry: &'a AccountRegistry,
    pub pending: &'a LoginState,
    pub effects: &'a dyn LoginEffects,
}
impl LoginService<'_> {
    pub fn select(&self, id: &str) -> Result<(), String> {
        if self.pending.logging_in(id) {
            return Err(code("err.account.busy"));
        }
        self.registry.update(|data| data.select(id))?;
        self.effects.refresh(None);
        self.effects.publish();
        Ok(())
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.pending
            .while_idle(|| self.registry.update(|data| data.remove(id)))?;
        self.effects.forget(id);
        self.effects.publish();
        Ok(())
    }
    /// The provider edge validates support and attachment parameters before entering this operation.
    pub fn attach(&self, account: Account) -> Result<(), String> {
        self.pending.while_idle(|| {
            self.registry.update(|data| {
                if !data
                    .accounts
                    .iter()
                    .any(|existing| existing.id == account.id)
                {
                    data.accounts.push(account);
                }
                Ok(())
            })
        })?;
        self.effects.publish();
        Ok(())
    }
    /// Provider/method validation and ID generation belong to the caller's adapter.
    pub fn begin(
        &self,
        provider: ProviderId,
        id: String,
        method: String,
    ) -> Result<LoginAttempt, String> {
        let attempt = self.pending.begin(self.registry, provider, id, method)?;
        self.effects.refresh(Some(provider));
        if self.effects.working(&attempt.account.id) {
            self.finish(&attempt);
            return Err(code("err.account.working"));
        }
        self.effects.publish();
        Ok(attempt)
    }
    /// Run once for a live attempt on its owning host's blocking executor. The host must call
    /// finish after completion or executor
    /// failure, so pending state remains visible until native authentication has stopped.
    pub fn authenticate(
        &self,
        attempt: &LoginAttempt,
        authentication: &dyn AccountAuthentication,
    ) -> Result<(), String> {
        let identity = authentication.authenticate(&attempt.account, attempt.cancel.clone())?;
        if attempt.cancel.load(Ordering::Relaxed) {
            return Err(code("err.account.cancelled"));
        }
        self.registry.update(|data: &mut Registry| {
            let account = data
                .accounts
                .iter_mut()
                .find(|account| account.id == attempt.account.id)
                .ok_or_else(|| code("err.account.missing"))?;
            account.identity = identity;
            account.revision += 1;
            Ok(())
        })?;
        self.effects.forget(&attempt.account.id);
        Ok(())
    }
    pub fn finish(&self, attempt: &LoginAttempt) {
        if self.pending.finish(attempt) {
            self.effects.refresh(Some(attempt.account.provider));
            self.effects.publish();
        }
    }
}

#[cfg(test)]
mod tests;
