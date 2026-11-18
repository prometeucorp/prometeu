//! Shared command/output ordering with injected host reactions and pending-input scheduling.
use super::output::{capture_command, CommandTelemetry, SessionOutput, SessionTelemetry};
use crate::conversation::stream::{keep, public_text, CommitOutcome, ConversationInput, Delivery};
use crate::conversation::Clock;
use crate::lock::lock;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub trait PumpReactions: Send + Sync {
    /// Called outside capture and transcript locks. True means the turn and its children settled.
    fn react(&self, event: &Value, ready: &AtomicBool) -> bool;
}

pub trait PendingInput: Send + Sync {
    fn queued(&self) -> bool;
    fn setup_running(&self) -> bool;
    /// Schedule a send off the output reader. The session service rechecks admission at execution.
    fn schedule(&self);
}

pub trait PumpDiagnostics: Send + Sync {
    /// May run under command/capture/transcript locks; must not reenter conversation operations.
    fn storage_error(&self, error: &str);
}

/// Optional durability boundary for transports that must commit before any live effects.
/// The host receives sanitized canonical lines only; a command's lines form one atomic batch.
pub trait CanonicalCommit: Send + Sync {
    fn commit(&self, public_lines: &[String], line_offset: u64) -> Result<CommitOutcome, String>;
}

pub struct SessionPump<T> {
    pub telemetry: Arc<Mutex<T>>,
    pub output: Arc<SessionOutput>,
    pub clock: Arc<dyn Clock>,
    ready: Arc<AtomicBool>,
    reactions: Arc<dyn PumpReactions>,
    pending: Arc<dyn PendingInput>,
    diagnostics: Arc<dyn PumpDiagnostics>,
    commit: Option<Arc<dyn CanonicalCommit>>,
}

impl<T> Clone for SessionPump<T> {
    fn clone(&self) -> Self {
        Self {
            telemetry: self.telemetry.clone(),
            output: self.output.clone(),
            clock: self.clock.clone(),
            ready: self.ready.clone(),
            reactions: self.reactions.clone(),
            pending: self.pending.clone(),
            diagnostics: self.diagnostics.clone(),
            commit: self.commit.clone(),
        }
    }
}

impl<T: SessionTelemetry> SessionPump<T> {
    pub fn new(
        output: Arc<SessionOutput>,
        telemetry: T,
        clock: Arc<dyn Clock>,
        reactions: Arc<dyn PumpReactions>,
        pending: Arc<dyn PendingInput>,
        diagnostics: Arc<dyn PumpDiagnostics>,
    ) -> Self {
        Self {
            output,
            telemetry: Arc::new(Mutex::new(telemetry)),
            clock,
            ready: Arc::new(AtomicBool::new(false)),
            reactions,
            pending,
            diagnostics,
            commit: None,
        }
    }

    pub fn with_commit(mut self, commit: Arc<dyn CanonicalCommit>) -> Self {
        self.commit = Some(commit);
        self
    }

    pub fn feed(&self, text: &str) {
        if let Some(commit) = &self.commit {
            if let Err(error) =
                self.feed_committed(text, |line, offset| commit.commit(&[line.into()], offset))
            {
                self.diagnostics.storage_error(&error);
            }
            return;
        }
        let received = {
            let mut capture = lock(&self.telemetry);
            self.output.receive(text, &mut *capture)
        };
        if let Some((frame, delivery)) = received {
            self.report_delivery(delivery);
            self.react(&frame);
        }
    }

    /// Commit before public delivery, telemetry and reactions. The callback runs under capture
    /// and transcript locks and must not reenter the pump. Duplicate or failed commits cause no
    /// live effects. Nonpersistent events follow ordinary delivery without calling `commit`.
    pub fn feed_committed(
        &self,
        text: &str,
        commit: impl FnOnce(&str, u64) -> Result<CommitOutcome, String>,
    ) -> Result<bool, String> {
        self.feed_observed_committed(text, |line, offset| match line {
            Some(line) => commit(line, offset),
            None => Ok(CommitOutcome::Appended),
        })
    }

    /// Unlike ordinary committed output, a native source occurrence must be receipted even when
    /// it has no retained public line. A failed or duplicate receipt produces no live effects.
    pub fn feed_observed_committed(
        &self,
        text: &str,
        commit: impl FnOnce(Option<&str>, u64) -> Result<CommitOutcome, String>,
    ) -> Result<bool, String> {
        let received = {
            let mut capture = lock(&self.telemetry);
            self.output
                .receive_observed_committed(text, &mut *capture, commit)?
        };
        if let Some((frame, delivery)) = received {
            self.report_delivery(delivery);
            self.react(&frame);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Publish a proven revision of previously captured, finished content. The host proves
    /// occurrence/version identity; this path never reopens execution or repeats capture/reactions.
    pub fn revise_committed(
        &self,
        text: &str,
        commit: impl FnOnce(&str, u64) -> Result<CommitOutcome, String>,
    ) -> Result<bool, String> {
        let _capture = lock(&self.telemetry);
        if self.output.gone.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let frame: Value =
            serde_json::from_str(text).map_err(|_| "invalid_revision".to_string())?;
        if frame["v"] != 1
            || !matches!(
                frame["type"].as_str(),
                Some(
                    "assistant.block"
                        | "tool.completed"
                        | "system.summary"
                        | "context.reported"
                        | "system.notice"
                        | "context.compacted"
                )
            )
        {
            return Err("invalid_revision".into());
        }
        let mut lines = lock(&self.output.lines);
        if commit(&public_text(text, &frame), lines.end())? == CommitOutcome::AlreadyCommitted {
            return Ok(false);
        }
        let delivery = lines.record_committed(text, &frame, self.output.events.as_ref());
        self.report_delivery(delivery);
        Ok(true)
    }

    fn report_delivery(&self, delivery: Option<Delivery>) {
        if let Some(error) = delivery.and_then(|delivery| delivery.storage_error) {
            self.diagnostics.storage_error(&error);
        }
    }

    fn react(&self, frame: &Value) {
        if self.output.gone.load(Ordering::Relaxed) {
            return;
        }
        self.output.execution.publish(frame);
        if self.reactions.react(frame, &self.ready)
            && self.pending.queued()
            && !self.pending.setup_running()
        {
            self.pending.schedule();
        }
    }

    /// Caller holds capture and process admission locks. Reactions are deferred until capture
    /// finishes, so input echoes cannot deadlock by issuing another command under these locks.
    pub fn command(
        &self,
        frame: &Value,
        input: &mut dyn ConversationInput,
    ) -> Result<Vec<Value>, String> {
        if let Some(commit) = &self.commit {
            return self
                .command_committed(frame, input, |lines, offset| commit.commit(lines, offset));
        }
        let mut lines = lock(&self.output.lines);
        let previous = (frame["type"] == "message.send")
            .then(|| self.output.turn.swap(true, Ordering::Relaxed));
        let result = lines.command(frame, self.clock.as_ref(), input, |lines, event| {
            self.report_delivery(self.output.record(lines, &event.to_string(), event));
        });
        if result.is_err() {
            if let Some(previous) = previous {
                self.output.turn.store(previous, Ordering::Relaxed);
            }
        }
        result
    }

    /// Input is sent once. Its accepted events are staged under the transcript gate, committed
    /// as a single public batch, then published. Commit failure does not set busy or emit echoes.
    /// Storage and an external process cannot be one transaction; this method never retries input.
    pub fn command_committed(
        &self,
        frame: &Value,
        input: &mut dyn ConversationInput,
        commit: impl FnOnce(&[String], u64) -> Result<CommitOutcome, String>,
    ) -> Result<Vec<Value>, String> {
        if self.output.gone.load(Ordering::Relaxed) {
            return Err("conversation_gone".into());
        }
        let mut lines = lock(&self.output.lines);
        let events = lines.command(frame, self.clock.as_ref(), input, |_, _| {})?;
        let public = events
            .iter()
            .filter(|event| keep(event))
            .map(|event| public_text(&event.to_string(), event).into_owned())
            .collect::<Vec<_>>();
        if !public.is_empty() && commit(&public, lines.end())? == CommitOutcome::AlreadyCommitted {
            return Ok(Vec::new());
        }
        if frame["type"] == "message.send" {
            self.output.turn.store(true, Ordering::Relaxed);
        }
        for event in &events {
            self.report_delivery(self.output.record_committed(
                &mut lines,
                &event.to_string(),
                event,
            ));
        }
        Ok(events)
    }

    /// Capture a canonical user message already accepted by the native process. The caller uses
    /// `capture_committed` for attribution; no input port is invoked and retries are idempotent.
    pub fn accept_committed(
        &self,
        frame: &Value,
        commit: impl FnOnce(&str, u64) -> Result<CommitOutcome, String>,
    ) -> Result<Vec<Value>, String> {
        if frame["v"] != 1 || frame["type"] != "user.message" {
            return Err("invalid_user_message".into());
        }
        if self.output.gone.load(Ordering::Relaxed) {
            return Err("conversation_gone".into());
        }
        let mut lines = lock(&self.output.lines);
        if commit(&public_text(&frame.to_string(), frame), lines.end())?
            == CommitOutcome::AlreadyCommitted
        {
            return Ok(Vec::new());
        }
        self.output.turn.store(true, Ordering::Relaxed);
        let events = vec![
            crate::conversation::event(
                "session.state",
                self.clock.now(),
                serde_json::json!({"state":"busy"}),
            ),
            frame.clone(),
        ];
        for event in &events {
            self.report_delivery(self.output.record_committed(
                &mut lines,
                &event.to_string(),
                event,
            ));
        }
        Ok(events)
    }
}

impl<T: CommandTelemetry> SessionPump<T> {
    /// Durable acceptance skips telemetry and reactions when the storage callback found an
    /// already committed occurrence. Ordinary command capture retains its existing behavior.
    pub fn capture_committed(
        &self,
        prepared: T::Prepared,
        execute: impl FnOnce() -> Result<Vec<Value>, String>,
    ) -> Result<(), String> {
        let events = {
            let mut capture = lock(&self.telemetry);
            let generation = capture.generation();
            let events = execute()?;
            if !events.is_empty() {
                capture.accepted(prepared, generation, &events);
            }
            events
        };
        for event in events {
            self.react(&event);
        }
        Ok(())
    }
    /// The closure rechecks process identity and admission, writes through `command`, and releases
    /// its process lock before returning. Failed admission or writes capture and react to nothing.
    pub fn capture(
        &self,
        prepared: T::Prepared,
        execute: impl FnOnce() -> Result<Vec<Value>, String>,
    ) -> Result<(), String> {
        let events = capture_command(&self.telemetry, prepared, execute)?;
        for event in events {
            self.react(&event);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
