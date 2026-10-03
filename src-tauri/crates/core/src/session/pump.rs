//! Shared command/output ordering with injected host reactions and pending-input scheduling.
use super::output::{capture_command, CommandTelemetry, SessionOutput, SessionTelemetry};
use crate::conversation::stream::{ConversationInput, Delivery};
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

pub struct SessionPump<T> {
    pub telemetry: Arc<Mutex<T>>,
    pub output: Arc<SessionOutput>,
    pub clock: Arc<dyn Clock>,
    ready: Arc<AtomicBool>,
    reactions: Arc<dyn PumpReactions>,
    pending: Arc<dyn PendingInput>,
    diagnostics: Arc<dyn PumpDiagnostics>,
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
        }
    }

    pub fn feed(&self, text: &str) {
        let received = {
            let mut capture = lock(&self.telemetry);
            self.output.receive(text, &mut *capture)
        };
        if let Some((frame, delivery)) = received {
            self.report_delivery(delivery);
            self.react(&frame);
        }
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
}

impl<T: CommandTelemetry> SessionPump<T> {
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
