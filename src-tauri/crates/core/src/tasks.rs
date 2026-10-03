//! Execution-host scheduling. Implementations must not run accepted work inline.
pub type Task = Box<dyn FnOnce() + Send + 'static>;

pub trait TaskExecutor: Send + Sync {
    /// Accept independent background work without waiting for completion. On error, drop the
    /// task before returning: captured process waiters own cleanup even when scheduling fails.
    fn spawn(&self, task: Task) -> Result<(), String>;
}
