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
    fn send_dispatch(
        &self,
        _: &str,
        _: &crate::board::NativeDispatch,
    ) -> Result<DispatchOutcome, String> {
        Err("native_dispatch_unavailable".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    Sent,
    AlreadyAccepted,
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
    fn persist(&self) -> Result<(), String> {
        Err("durable_publication_unavailable".into())
    }
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
            if tab.native_dispatch.is_some() {
                match &mut tab.pending_prompt {
                    Some(pending) => {
                        pending.push_str("\n\n");
                        pending.push_str(text);
                    }
                    None => tab.pending_prompt = Some(text.to_owned()),
                }
                return true;
            }
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
        if lock(self.board)
            .tab_mut(session)
            .is_some_and(|tab| tab.conversation_mode == crate::board::ConversationMode::Terminal)
        {
            self.send_native_prompt(session, prefix);
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

    fn send_native_prompt(&self, session: &str, prefix: Option<String>) {
        if let Some(prefix) = prefix {
            if let Some(prompt) = lock(self.board)
                .tab_mut(session)
                .and_then(|tab| tab.pending_prompt.as_mut())
            {
                prompt.insert_str(0, &prefix);
            }
        }
        match self.account_boundary(session) {
            Ok(AccountBoundary::Keep) => {}
            Ok(AccountBoundary::Restart) => {
                if let Err(error) = self.runtime.revive(session) {
                    self.diagnostics.account_error(&error);
                }
                return;
            }
            Ok(AccountBoundary::Wait) => return,
            Err(error) => {
                self.diagnostics.account_error(&error);
                return;
            }
        }
        let dispatch = {
            let mut board = lock(self.board);
            let Some(tab) = board.tab_mut(session) else {
                return;
            };
            if tab.native_dispatch.is_none() {
                let Some(text) = tab.pending_prompt.take() else {
                    return;
                };
                tab.native_dispatch = Some(crate::board::NativeDispatch {
                    id: uuid::Uuid::new_v4().to_string(),
                    text,
                });
            }
            tab.native_dispatch.clone().unwrap()
        };
        // The item remains recoverable on the board until a durable acceptance receipt exists.
        // A host cannot write it to the native process before this persistence barrier succeeds.
        let outcome = self
            .publication
            .persist()
            .and_then(|_| self.runtime.send_dispatch(session, &dispatch));
        match outcome {
            Ok(outcome) => {
                {
                    let mut board = lock(self.board);
                    if let Some(tab) = board.tab_mut(session) {
                        if tab
                            .native_dispatch
                            .as_ref()
                            .is_some_and(|current| current.id == dispatch.id)
                        {
                            tab.native_dispatch = None;
                        }
                    }
                }
                if let Err(error) = self.publication.persist() {
                    if let Some(tab) = lock(self.board).tab_mut(session) {
                        tab.native_dispatch = Some(dispatch);
                    }
                    self.update(session, None, Note::Set(error.clone()), None);
                    self.diagnostics.pending_error(session, &error);
                    return;
                }
                self.update(
                    session,
                    (outcome == DispatchOutcome::Sent).then_some(Status::Rodando),
                    Note::Clear,
                    None,
                );
            }
            Err(error) => {
                if let Some(tab) = lock(self.board).tab_mut(session) {
                    if let Some(run) = tab.task.as_mut() {
                        run.paused = true;
                        run.error = Some(error.clone());
                    }
                }
                self.update(session, None, Note::Set(error.clone()), None);
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
        self.update_state(session, status, note, tokens, true);
    }

    /// Read-only availability is not a completed turn or a new request for the person's attention.
    pub(crate) fn observe_state(&self, session: &str, status: Option<Status>, note: Note) {
        self.update_state(session, status, note, None, false);
    }

    fn update_state(
        &self,
        session: &str,
        status: Option<Status>,
        note: Note,
        tokens: Option<u64>,
        attention: bool,
    ) {
        let looking = self.publication.looking();
        let mut board = lock(self.board);
        let Some(ws) = board.workspace_of_mut(session) else {
            return;
        };
        if attention
            && matches!(status, Some(Status::Pronta | Status::Querendo))
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
