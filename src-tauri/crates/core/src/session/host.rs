//! Per-host session ownership and service composition. Native conversation adapters supply effects.
use super::{
    AccountState, InputGates, RuntimeState, SessionAccounts, SessionDiagnostics,
    SessionPublication, SessionRuntime, SessionService,
};
use crate::board::Board;
use crate::conversation::work::Work;
use crate::lock::lock;
use crate::process::ProcessIdentity;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// The registry observes a conversation without knowing its provider or transport.
pub trait HostedConversation {
    fn identity(&self) -> &ProcessIdentity;
    fn alive(&self) -> bool;
    fn waits_for_turn(&self) -> bool;
    fn working(&self) -> bool;
    fn changing_account(&self) -> Result<bool, String>;
    /// Suppress further output before removal clears transient state. Runs under the registry
    /// lock; native teardown belongs in Drop, after releasing that lock.
    fn retire(&mut self);
}

/// Effects that need native launch configuration, account adapters or feature integration.
pub trait SessionLifecycle {
    fn setup_running(&self, session: &str) -> bool;
    fn revive(&self, session: &str) -> Result<(), String>;
    /// Called after removing a conversation and clearing its transient state.
    fn stopped(&self, session: &str);
    /// Recheck the current process and idle-only admission under the conversation lock.
    fn send(&self, session: &str, text: &str, idle_only: bool) -> Result<(), String>;
}

pub struct SessionHost<C> {
    /// Command capture needs this lock through process validation and transcript writes.
    pub conversations: Mutex<HashMap<String, C>>,
    pub inputs: InputGates,
    /// Shared with canonical event reactions; no process adapter owns settlement policy.
    pub work: Mutex<HashMap<String, Work>>,
    ready: Mutex<HashSet<String>>,
}

impl<C> Default for SessionHost<C> {
    fn default() -> Self {
        Self {
            conversations: Mutex::new(HashMap::new()),
            inputs: InputGates::default(),
            work: Mutex::new(HashMap::new()),
            ready: Mutex::new(HashSet::new()),
        }
    }
}

impl<C> SessionHost<C> {
    pub fn is_ready(&self, session: &str) -> bool {
        lock(&self.ready).contains(session)
    }

    pub fn mark_ready(&self, session: &str) {
        lock(&self.ready).insert(session.into());
    }

    pub fn busy(&self, session: &str) -> bool {
        lock(&self.work).get(session).is_some_and(Work::busy)
    }

    fn clear(&self, session: &str) {
        lock(&self.ready).remove(session);
        lock(&self.work).remove(session);
    }
}

impl<C: HostedConversation> SessionHost<C> {
    /// Retire and clear state while excluding replacement, then drop the transport outside locks.
    pub fn remove(&self, session: &str) {
        let removed = {
            let mut conversations = lock(&self.conversations);
            let mut removed = conversations.remove(session);
            if let Some(conversation) = removed.as_mut() {
                conversation.retire();
            }
            self.clear(session);
            removed
        };
        drop(removed);
    }

    pub fn state(&self, session: &str) -> RuntimeState {
        let conversations = lock(&self.conversations);
        let conversation = conversations.get(session);
        RuntimeState {
            up: conversation.is_some_and(HostedConversation::alive),
            ready: self.is_ready(session),
            waits_for_turn: conversation.is_some_and(HostedConversation::waits_for_turn),
        }
    }

    pub fn account_state(&self, session: &str) -> Result<AccountState, String> {
        let conversations = lock(&self.conversations);
        match conversations.get(session).filter(|chat| chat.alive()) {
            Some(chat) => Ok(AccountState {
                changed: chat.changing_account()?,
                working: chat.working(),
            }),
            None => Ok(AccountState::default()),
        }
    }

    /// Keep replay available after exit. Only the current identity may clear state or publish
    /// closure. The callback runs under the conversation lock and must not reenter this registry.
    pub fn exited(
        &self,
        session: &str,
        identity: &ProcessIdentity,
        publish: impl FnOnce(),
    ) -> bool {
        let conversations = lock(&self.conversations);
        if conversations.get(session).map(HostedConversation::identity) != Some(identity) {
            return false;
        }
        self.clear(session);
        publish();
        true
    }

    pub fn with_service<T>(
        &self,
        board: &Mutex<Board>,
        lifecycle: &dyn SessionLifecycle,
        publication: &dyn SessionPublication,
        diagnostics: &dyn SessionDiagnostics,
        run: impl FnOnce(&SessionService<'_>) -> T,
    ) -> T {
        let runtime = HostedRuntime {
            host: self,
            lifecycle,
        };
        run(&SessionService {
            board,
            runtime: &runtime,
            accounts: &runtime,
            publication,
            diagnostics,
        })
    }
}

struct HostedRuntime<'a, C> {
    host: &'a SessionHost<C>,
    lifecycle: &'a dyn SessionLifecycle,
}

impl<C: HostedConversation> SessionRuntime for HostedRuntime<'_, C> {
    fn state(&self, session: &str) -> RuntimeState {
        self.host.state(session)
    }
    fn setup_running(&self, session: &str) -> bool {
        self.lifecycle.setup_running(session)
    }
    fn stop(&self, session: &str) {
        self.host.remove(session);
        self.lifecycle.stopped(session);
    }
    fn revive(&self, session: &str) -> Result<(), String> {
        self.lifecycle.revive(session)
    }
    fn mark_ready(&self, session: &str) {
        self.host.mark_ready(session);
    }
    fn send(&self, session: &str, text: &str, idle_only: bool) -> Result<(), String> {
        self.lifecycle.send(session, text, idle_only)
    }
}

impl<C: HostedConversation> SessionAccounts for HostedRuntime<'_, C> {
    fn state(&self, session: &str) -> Result<AccountState, String> {
        self.host.account_state(session)
    }
}

#[cfg(test)]
mod tests;
