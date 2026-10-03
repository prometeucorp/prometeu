//! Process supervision policy. Native launch requests and effects belong to execution adapters.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

/// In-process identity, independent of a recyclable system PID or a running flag.
#[derive(Clone, Default, Debug)]
pub struct ProcessIdentity(Arc<()>);

impl PartialEq for ProcessIdentity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ProcessIdentity {}

/// Stop requests are best effort. Only the waiter establishes that the process has exited.
/// Implementations must serialize signaling with reaping to prevent signaling a reused PID.
pub trait ProcessControl: Send + Sync {
    fn system_id(&self) -> u32;
    fn running(&self) -> bool;
    fn interrupt(&self);
    fn terminate(&self);
    fn kill(&self);
    fn wait_exit(&self, timeout: Duration) -> bool;
}

#[derive(Clone)]
pub struct ProcessHandle {
    pub identity: ProcessIdentity,
    pub control: Arc<dyn ProcessControl>,
}

impl ProcessHandle {
    pub fn new(control: Arc<dyn ProcessControl>) -> Self {
        Self {
            identity: ProcessIdentity::default(),
            control,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessExit {
    /// Absent when the native host cannot express termination as an exit code.
    pub code: Option<i32>,
}

pub trait ProcessWait: Send {
    /// Reap once; subsequent calls return the same exit. Dropping an unreaped waiter must clean
    /// up its owned process. The host consumes queued output before calling this method.
    fn wait(&mut self) -> Result<ProcessExit, String>;
}

pub type ProcessOutput = Box<dyn Iterator<Item = String> + Send>;

pub struct StartedProcess {
    pub input: Box<dyn Write + Send>,
    pub stdout: ProcessOutput,
    pub stderr: ProcessOutput,
    pub handle: ProcessHandle,
    pub waiter: Box<dyn ProcessWait>,
}

#[derive(Debug)]
pub enum LaunchError {
    Spawn(String),
    Pipe,
}

/// A typed launch operation over the provider adapter's prepared request. Native command builders
/// stay outside the core; this request is not a serializable desktop/runtime bridge contract.
pub trait ProcessLauncher<Request>: Send + Sync {
    /// Establish both independent pipe drains before returning input to the caller.
    fn launch(&self, request: Request) -> Result<StartedProcess, LaunchError>;
}

#[derive(Clone, Copy)]
pub struct ShutdownPolicy {
    pub input_grace: Duration,
    pub terminate_grace: Duration,
}

impl ShutdownPolicy {
    /// The terminal adapter requests hangup before applying this policy.
    pub const TERMINAL: Self = Self {
        input_grace: Duration::from_secs(1),
        terminate_grace: Duration::from_millis(500),
    };

    pub const AGENT: Self = Self {
        input_grace: Duration::from_secs(2),
        terminate_grace: Duration::from_millis(500),
    };

    /// Close provider input before scheduling. If no worker can be created, request immediate
    /// kill rather than leaking the child; its owning waiter still performs reaping.
    pub fn schedule(
        self,
        process: Arc<dyn ProcessControl>,
        tasks: &dyn crate::tasks::TaskExecutor,
    ) -> Result<(), String> {
        let fallback = process.clone();
        let result = tasks.spawn(Box::new(move || self.shutdown(process.as_ref())));
        if result.is_err() {
            fallback.kill();
        }
        result
    }

    /// The host closes provider input first, then runs this policy off its interaction thread.
    /// A kill request does not imply an observed exit; the process waiter remains authoritative.
    pub fn shutdown(self, process: &dyn ProcessControl) {
        if process.wait_exit(self.input_grace) {
            return;
        }
        process.terminate();
        if process.wait_exit(self.terminate_grace) {
            return;
        }
        process.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::lock;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct Controlled {
        exits: Mutex<VecDeque<bool>>,
        effects: Mutex<Vec<String>>,
    }
    impl ProcessControl for Controlled {
        fn system_id(&self) -> u32 {
            10
        }
        fn running(&self) -> bool {
            true
        }
        fn interrupt(&self) {
            lock(&self.effects).push("interrupt".into());
        }
        fn terminate(&self) {
            lock(&self.effects).push("terminate".into());
        }
        fn kill(&self) {
            lock(&self.effects).push("kill".into());
        }
        fn wait_exit(&self, timeout: Duration) -> bool {
            lock(&self.effects).push(format!("wait:{}", timeout.as_millis()));
            lock(&self.exits).pop_front().unwrap()
        }
    }

    #[test]
    fn shutdown_uses_only_the_escalation_needed_by_the_injected_process() {
        for (exits, expected) in [
            (vec![true], vec!["wait:2000"]),
            (
                vec![false, true],
                vec!["wait:2000", "terminate", "wait:500"],
            ),
            (
                vec![false, false],
                vec!["wait:2000", "terminate", "wait:500", "kill"],
            ),
        ] {
            let process = Controlled {
                exits: Mutex::new(exits.into()),
                effects: Mutex::new(vec![]),
            };
            ShutdownPolicy::AGENT.shutdown(&process);
            assert_eq!(*lock(&process.effects), expected);
        }
    }

    #[test]
    fn rejected_shutdown_scheduling_kills_without_waiting_and_accepted_work_is_deferred() {
        struct Reject;
        impl crate::tasks::TaskExecutor for Reject {
            fn spawn(&self, task: crate::tasks::Task) -> Result<(), String> {
                drop(task);
                Err("thread limit".into())
            }
        }
        let process = Arc::new(Controlled {
            exits: Mutex::new(VecDeque::new()),
            effects: Mutex::new(vec![]),
        });
        assert_eq!(
            ShutdownPolicy::AGENT.schedule(process.clone(), &Reject),
            Err("thread limit".into())
        );
        assert_eq!(*lock(&process.effects), ["kill"]);

        struct Queue(Mutex<Option<crate::tasks::Task>>);
        impl crate::tasks::TaskExecutor for Queue {
            fn spawn(&self, task: crate::tasks::Task) -> Result<(), String> {
                *lock(&self.0) = Some(task);
                Ok(())
            }
        }
        let queue = Queue(Mutex::new(None));
        let process = Arc::new(Controlled {
            exits: Mutex::new(vec![true].into()),
            effects: Mutex::new(vec![]),
        });
        ShutdownPolicy::AGENT
            .schedule(process.clone(), &queue)
            .unwrap();
        assert!(lock(&process.effects).is_empty());
        let task = lock(&queue.0).take().unwrap();
        task();
        assert_eq!(*lock(&process.effects), ["wait:2000"]);
    }

    #[test]
    fn replacement_identity_is_independent_of_native_id_and_running_state() {
        let process = Arc::new(Controlled {
            exits: Mutex::new(VecDeque::new()),
            effects: Mutex::new(vec![]),
        });
        let old = ProcessHandle::new(process.clone());
        let replacement = ProcessHandle::new(process);
        assert_eq!(old.identity, old.clone().identity);
        assert_ne!(old.identity, replacement.identity);
        assert_eq!(old.control.system_id(), replacement.control.system_id());
    }
}
