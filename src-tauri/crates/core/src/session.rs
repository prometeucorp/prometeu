//! Logical session input and recovery. Hosts supply effects; the board remains authoritative.

use crate::board::{Board, Note, Status};
use crate::error::code;
use crate::lock::lock;
use std::sync::{Arc, Mutex};

/// One execution host owns its admission gates; unrelated roots never share an ambient registry.
#[derive(Default)]
pub struct InputGates(Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>);

impl InputGates {
    pub fn session(&self, id: &str) -> Arc<Mutex<()>> {
        lock(&self.0).entry(id.into()).or_default().clone()
    }
}

pub mod host;
pub mod launch;
pub mod output;
pub mod provider;
pub mod pump;
pub mod reactions;
pub mod workers;

#[derive(Default, Clone, Copy)]
pub struct RuntimeState {
    pub up: bool,
    pub ready: bool,
    pub waits_for_turn: bool,
}

pub trait SessionRuntime {
    fn state(&self, session: &str) -> RuntimeState;
    fn setup_running(&self, session: &str) -> bool;
    fn stop(&self, session: &str);
    fn revive(&self, session: &str) -> Result<(), String>;
    fn mark_ready(&self, session: &str);
    /// Implementations recheck process identity and idle-only admission under their process lock.
    fn send(&self, session: &str, text: &str, idle_only: bool) -> Result<(), String>;
}

#[derive(Default, Clone, Copy)]
pub struct AccountState {
    pub changed: bool,
    pub working: bool,
}

pub trait SessionAccounts {
    /// No selected account or an in-progress login remains a structured application error.
    fn state(&self, session: &str) -> Result<AccountState, String>;
}

pub trait SessionPublication {
    fn publish(&self);
    fn looking(&self) -> Option<String>;
}

pub trait SessionDiagnostics {
    fn account_error(&self, error: &str);
    fn pending_error(&self, session: &str, error: &str);
}

#[derive(Debug, PartialEq)]
pub enum AccountBoundary {
    Keep,
    Wait,
    Restart,
}

pub fn boundary(changed: bool, working: bool) -> AccountBoundary {
    match (changed, working) {
        (false, _) => AccountBoundary::Keep,
        (true, true) => AccountBoundary::Wait,
        (true, false) => AccountBoundary::Restart,
    }
}

pub struct SessionService<'a> {
    pub board: &'a Mutex<Board>,
    pub runtime: &'a dyn SessionRuntime,
    pub accounts: &'a dyn SessionAccounts,
    pub publication: &'a dyn SessionPublication,
    pub diagnostics: &'a dyn SessionDiagnostics,
}

impl SessionService<'_> {
    fn account_boundary(&self, session: &str) -> Result<AccountBoundary, String> {
        let account = self.accounts.state(session)?;
        Ok(boundary(account.changed, account.working))
    }

    /// Callers retain their per-session input gate across admission and delegation reservation.
    pub fn send(&self, session: &str, text: &str, idle_only: bool) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        let boundary = self.account_boundary(session)?;
        if boundary == AccountBoundary::Restart {
            self.runtime.stop(session);
        }
        let queued = lock(self.board).tab_mut(session).is_some_and(|tab| {
            tab.pending_prompt.as_mut().is_some_and(|pending| {
                pending.push_str("\n\n");
                pending.push_str(text);
                true
            })
        });
        let state = self.runtime.state(session);
        if !queued
            && state.up
            && state.ready
            && !state.waits_for_turn
            && boundary != AccountBoundary::Wait
        {
            self.runtime.send(session, text, idle_only)?;
            self.update(session, Some(Status::Rodando), Note::Clear, None);
            return Ok(());
        }
        if !queued {
            let mut board = lock(self.board);
            let tab = board
                .tab_mut(session)
                .ok_or_else(|| code("err.session.noTab"))?;
            tab.pending_prompt = Some(text.to_string());
        }
        self.publication.publish();
        self.flush_pending(session)
    }

    pub fn flush_pending(&self, session: &str) -> Result<(), String> {
        match self.account_boundary(session)? {
            AccountBoundary::Wait => return Ok(()),
            AccountBoundary::Restart => return self.runtime.revive(session),
            AccountBoundary::Keep => {}
        }
        let state = self.runtime.state(session);
        match (state.up, state.ready, self.runtime.setup_running(session)) {
            (false, _, _) => self.runtime.revive(session)?,
            (true, false, _) => self.ready_now(session),
            (true, true, false) => self.send_prompt(session, None),
            (true, true, true) => {}
        }
        Ok(())
    }

    pub fn ready_now(&self, session: &str) {
        self.runtime.mark_ready(session);
        if !self.runtime.setup_running(session) {
            self.send_prompt(session, None);
        }
    }

    pub fn send_prompt(&self, session: &str, prefix: Option<String>) {
        if self.runtime.state(session).waits_for_turn {
            return;
        }
        {
            let mut board = lock(self.board);
            let Some(prompt) = board
                .tab_mut(session)
                .and_then(|tab| tab.pending_prompt.as_mut())
            else {
                return;
            };
            if let Some(prefix) = prefix {
                prompt.insert_str(0, &prefix);
            }
        }
        match self.account_boundary(session) {
            Ok(AccountBoundary::Wait) => return,
            Ok(AccountBoundary::Restart) => {
                if let Err(error) = self.runtime.revive(session) {
                    self.update(session, None, Note::Set(error.clone()), None);
                    self.diagnostics.account_error(&error);
                }
                return;
            }
            Err(error) => {
                self.diagnostics.account_error(&error);
                return;
            }
            Ok(AccountBoundary::Keep) => {}
        }
        let prompt = {
            let mut board = lock(self.board);
            let Some(prompt) = board
                .tab_mut(session)
                .and_then(|tab| tab.pending_prompt.take())
            else {
                return;
            };
            prompt
        };
        match self.runtime.send(session, &prompt, false) {
            Ok(()) => self.update(session, Some(Status::Rodando), Note::Clear, None),
            Err(error) => {
                let mut board = lock(self.board);
                if let Some(tab) = board.tab_mut(session) {
                    if let Some(run) = tab.task.as_mut() {
                        run.paused = true;
                        run.error = Some(error.clone());
                    }
                    tab.pending_prompt = Some(match tab.pending_prompt.take() {
                        Some(after) => format!("{prompt}\n\n{after}"),
                        None => prompt,
                    });
                }
                drop(board);
                self.publication.publish();
                self.diagnostics.pending_error(session, &error);
            }
        }
    }

    pub fn remember_session(&self, session: &str, provider_session: &str) {
        let mut board = lock(self.board);
        let Some(tab) = board.tab_mut(session) else {
            return;
        };
        if tab.agent_session.as_deref() == Some(provider_session) {
            return;
        }
        tab.agent_session = Some(provider_session.into());
        drop(board);
        self.publication.publish();
    }

    pub fn update(&self, session: &str, status: Option<Status>, note: Note, tokens: Option<u64>) {
        let looking = self.publication.looking();
        let mut board = lock(self.board);
        let Some(ws) = board.workspace_of_mut(session) else {
            return;
        };
        if matches!(status, Some(Status::Pronta | Status::Querendo))
            && looking.as_deref() != Some(ws.id.as_str())
        {
            ws.unread = true;
        }
        let Some(tab) = ws.tabs.iter_mut().find(|tab| tab.id == session) else {
            return;
        };
        if let Some(status) = status {
            tab.status = status;
        }
        match note {
            Note::Clear => tab.note = None,
            Note::Set(note) => tab.note = Some(note),
            Note::Keep => {}
        }
        if let Some(tokens) = tokens {
            tab.observe_tokens(tokens);
        }
        drop(board);
        self.publication.publish();
    }
}

#[cfg(test)]
mod tests;
