//! Ordered session observations around transcript delivery. The host supplies the capture gate.

use crate::conversation::stream::{ConversationEvents, Delivery, Lines, TranscriptStore};
use crate::lock::lock;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub trait ExecutionObservation: Send + Sync {
    /// Memory only; called under the transcript ordering lock.
    fn observe(&self, event: &Value);
    /// Called after transcript and telemetry capture locks are released.
    fn publish(&self, event: &Value);
}

pub trait SessionTelemetry {
    /// Original private frame, after transcript release but inside the host's capture gate.
    /// Recording failure must not reject an agent command or stop public output.
    fn observe(&mut self, event: &Value);
}

pub trait CommandTelemetry: SessionTelemetry {
    type Prepared;
    fn generation(&self) -> u64;
    fn accepted(&mut self, prepared: Self::Prepared, generation: u64, events: &[Value]);
}

/// Preserve capture order across accepted input and output. The command closure releases its
/// process/transcript locks before capture commits; callers run reactions after this returns.
pub fn capture_command<T: CommandTelemetry>(
    capture: &Mutex<T>,
    prepared: T::Prepared,
    execute: impl FnOnce() -> Result<Vec<Value>, String>,
) -> Result<Vec<Value>, String> {
    let mut capture = lock(capture);
    let generation = capture.generation();
    let events = execute()?;
    capture.accepted(prepared, generation, &events);
    Ok(events)
}

pub struct SessionOutput {
    pub lines: Arc<Mutex<Lines>>,
    pub gone: Arc<AtomicBool>,
    pub turn: AtomicBool,
    pub store: Arc<dyn TranscriptStore>,
    pub events: Arc<dyn ConversationEvents>,
    pub execution: Arc<dyn ExecutionObservation>,
}

impl SessionOutput {
    pub fn receive(
        &self,
        text: &str,
        telemetry: &mut dyn SessionTelemetry,
    ) -> Option<(Value, Option<Delivery>)> {
        if self.gone.load(Ordering::Relaxed) {
            return None;
        }
        let text = text.trim_end();
        let frame = serde_json::from_str::<Value>(text).ok()?;
        let delivery = self.record(&mut lock(&self.lines), text, &frame);
        if !self.gone.load(Ordering::Relaxed) {
            telemetry.observe(&frame);
        }
        Some((frame, delivery))
    }

    pub fn record(&self, lines: &mut Lines, text: &str, frame: &Value) -> Option<Delivery> {
        if self.gone.load(Ordering::Relaxed) || frame["type"] == "telemetry.usage" {
            return None;
        }
        match frame["type"].as_str() {
            Some("turn.completed") => self.turn.store(false, Ordering::Relaxed),
            Some("assistant.block" | "assistant.started") => {
                self.turn.store(true, Ordering::Relaxed)
            }
            _ => {}
        }
        self.execution.observe(frame);
        lines.record(text, frame, self.store.as_ref(), self.events.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Effects {
        lines: Arc<Mutex<Lines>>,
        order: Mutex<Vec<&'static str>>,
    }
    impl TranscriptStore for Effects {
        fn load(&self) -> Result<String, String> {
            Ok(String::new())
        }
        fn append(&self, _: &str) -> Result<(), String> {
            assert!(self.lines.try_lock().is_err());
            lock(&self.order).push("store");
            Ok(())
        }
    }
    impl ConversationEvents for Effects {
        fn emit(&self, text: &str, _: u64) -> Result<(), String> {
            assert!(self.lines.try_lock().is_err());
            assert!(!text.contains("telemetry"));
            lock(&self.order).push("event");
            Ok(())
        }
    }
    impl ExecutionObservation for Effects {
        fn observe(&self, _: &Value) {
            assert!(self.lines.try_lock().is_err());
            lock(&self.order).push("execution");
        }
        fn publish(&self, _: &Value) {
            assert!(self.lines.try_lock().is_ok());
            lock(&self.order).push("publish");
        }
    }
    struct Capture(Arc<Effects>);
    impl SessionTelemetry for Capture {
        fn observe(&mut self, event: &Value) {
            assert!(self.0.lines.try_lock().is_ok());
            assert!(event.get("telemetry").is_some());
            lock(&self.0.order).push("capture");
        }
    }
    impl CommandTelemetry for Capture {
        type Prepared = &'static str;
        fn generation(&self) -> u64 {
            12
        }
        fn accepted(&mut self, prepared: Self::Prepared, generation: u64, events: &[Value]) {
            assert_eq!(prepared, "prepared");
            assert_eq!(generation, 12);
            assert_eq!(events.len(), 1);
            assert!(self.0.lines.try_lock().is_ok());
            lock(&self.0.order).push("accepted");
        }
    }
    fn fixture() -> (SessionOutput, Arc<Effects>) {
        let lines = Arc::new(Mutex::new(Lines::default()));
        let effects = Arc::new(Effects {
            lines: lines.clone(),
            order: Mutex::new(vec![]),
        });
        (
            SessionOutput {
                lines,
                gone: Arc::new(AtomicBool::new(false)),
                turn: AtomicBool::new(false),
                store: effects.clone(),
                events: effects.clone(),
                execution: effects.clone(),
            },
            effects,
        )
    }

    #[test]
    fn execution_precedes_publication_and_private_capture_follows_transcript_release() {
        let (output, effects) = fixture();
        let capture = Mutex::new(Capture(effects.clone()));
        let frame = json!({"v":1,"type":"turn.completed","outcome":"ok","telemetry":{"usage":1}});
        let received = output
            .receive(&frame.to_string(), &mut *lock(&capture))
            .unwrap();
        assert_eq!(received.0, frame);
        assert!(capture.try_lock().is_ok());
        output.execution.publish(&received.0);
        assert_eq!(
            *lock(&effects.order),
            ["execution", "store", "event", "capture", "publish"]
        );
        assert_eq!(lock(&output.lines).seq, 1);
    }

    #[test]
    fn private_events_are_captured_without_public_effects_and_replaced_output_is_ignored() {
        let (output, effects) = fixture();
        let mut capture = Capture(effects.clone());
        assert!(output.receive("invalid", &mut capture).is_none());
        let private = json!({"type":"telemetry.usage","telemetry":{"usage":1}}).to_string();
        let (_, delivery) = output.receive(&private, &mut capture).unwrap();
        assert!(delivery.is_none());
        assert_eq!(lock(&output.lines).seq, 0);
        assert_eq!(*lock(&effects.order), ["capture"]);
        output.gone.store(true, Ordering::Relaxed);
        assert!(output.receive(&private, &mut capture).is_none());
        assert_eq!(*lock(&effects.order), ["capture"]);
    }

    #[test]
    fn accepted_capture_commits_after_execution_locks_and_failed_input_records_nothing() {
        let (output, effects) = fixture();
        let capture = Mutex::new(Capture(effects.clone()));
        let result = capture_command(&capture, "prepared", || {
            assert!(capture.try_lock().is_err());
            let _lines = lock(&output.lines);
            lock(&effects.order).push("send");
            Ok(vec![json!({"type":"user.message"})])
        });
        assert!(result.is_ok());
        assert!(capture.try_lock().is_ok());
        assert_eq!(*lock(&effects.order), ["send", "accepted"]);
        let result = capture_command(&capture, "prepared", || Err("broken pipe".into()));
        assert_eq!(result, Err("broken pipe".into()));
        assert_eq!(*lock(&effects.order), ["send", "accepted"]);
    }
}
