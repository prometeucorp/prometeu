//! Provider-neutral initialization and output workers, scheduled through an execution-host port.
use super::output::SessionTelemetry;
use super::pump::SessionPump;
use crate::conversation::{event, stream::ConversationInput};
use crate::process::{ProcessExit, ProcessIdentity, ProcessOutput, ProcessWait};
use crate::tasks::TaskExecutor;
use serde_json::json;
use std::sync::Arc;

/// Provider adapters own translation and filtering; workers consume canonical frames only.
pub type Translate = Box<dyn FnMut(&str) -> Vec<String> + Send>;

pub trait WorkerLifecycle: Send + Sync {
    fn initialization_failed(&self, error: &str);
    /// Output EOF is followed by wait and waiter release before this callback. Hosts must compare
    /// identity with their current registry entry before publishing closure.
    fn exited(&self, identity: &ProcessIdentity, result: Result<ProcessExit, String>);
}

pub struct ConversationWorkers {
    pub stdout: ProcessOutput,
    pub stderr: ProcessOutput,
    pub waiter: Box<dyn ProcessWait>,
    pub identity: ProcessIdentity,
    pub translate: Translate,
    pub stderr_line: fn(&str) -> Option<String>,
}

impl ConversationWorkers {
    /// Native pipe drains must already be active. Initialization errors are diagnostic; reader
    /// scheduling errors fail launch and release the owned waiter, which cleans up the process.
    pub fn start<T: SessionTelemetry + Send + 'static>(
        self,
        input: &mut dyn ConversationInput,
        pump: SessionPump<T>,
        lifecycle: Arc<dyn WorkerLifecycle>,
        executor: &dyn TaskExecutor,
    ) -> Result<(), String> {
        pump.feed(
            &event(
                "session.state",
                pump.clock.now(),
                json!({"state":"starting"}),
            )
            .to_string(),
        );
        match input.send(&json!({"v":1,"type":"commands.list"}), "") {
            Ok(echo) => {
                for frame in echo {
                    pump.feed(&frame.to_string());
                }
            }
            Err(error) => lifecycle.initialization_failed(&error),
        }
        let Self {
            stdout,
            stderr,
            mut waiter,
            identity,
            mut translate,
            stderr_line,
        } = self;
        let errors = pump.clone();
        executor.spawn(Box::new(move || {
            for line in stderr {
                let Some(line) = stderr_line(&line) else {
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                errors.feed(
                    &event(
                        "system.notice",
                        errors.clock.now(),
                        json!({
                            "level":"error", "code":"provider.stderr", "detail":line,
                        }),
                    )
                    .to_string(),
                );
            }
        }))?;
        executor.spawn(Box::new(move || {
            for line in stdout {
                for frame in translate(line.trim_end()) {
                    pump.feed(&frame);
                }
            }
            let exit = waiter.wait();
            drop(waiter);
            lifecycle.exited(&identity, exit);
        }))
    }
}

#[cfg(test)]
mod tests;
