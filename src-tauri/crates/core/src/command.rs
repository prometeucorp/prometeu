//! Bounded private queries and finite commands. Request types belong to native adapters.
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum CommandError {
    Unavailable,
    Timeout,
    OutputLimit,
    InvalidOutput,
    Io(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPolicy {
    Capture { limit: usize },
    Discard,
    Inherit,
}
#[derive(Clone, Copy, Debug)]
pub struct CommandPolicy {
    pub timeout: Duration,
    pub stdout: OutputPolicy,
    pub stderr: OutputPolicy,
}
#[derive(Debug)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
pub trait CommandRunner<Request>: Send + Sync {
    /// Close input after these bytes, drain both captured streams while writing, and bound the
    /// whole operation. A nonzero exit is a result; timeout and output overflow are errors.
    fn run(
        &self,
        request: &mut Request,
        input: &[u8],
        policy: CommandPolicy,
    ) -> Result<CommandOutput, CommandError>;
}

#[derive(Clone, Copy, Debug)]
pub struct QueryPolicy {
    pub timeout: Duration,
    /// Total stdout bytes across all lines, including delimiters.
    pub max_output: usize,
}
pub trait QueryProcess: Send {
    fn send(&mut self, bytes: &[u8]) -> Result<(), CommandError>;
    fn next(&mut self) -> Result<Option<String>, CommandError>;
    fn close_input(&mut self);
    fn finish(&mut self) -> Result<bool, CommandError>;
}
pub trait QueryLauncher<Request>: Send + Sync {
    /// One deadline covers writes, all response pages and exit. Dropping the returned session
    /// terminates/reaps its child. It never publishes private data through conversation events.
    fn launch(
        &self,
        request: &mut Request,
        policy: QueryPolicy,
    ) -> Result<Box<dyn QueryProcess>, CommandError>;
}
