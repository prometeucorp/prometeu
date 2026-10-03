//! Private line-oriented subprocess sessions, separate from agent conversations and terminals.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub enum LinePoll {
    Line(String),
    Pending,
    Closed,
}

/// Implementations own pipes and cleanup. Dropping the port must terminate and reap its child.
/// Output is private and must never be routed through conversation or terminal events.
pub trait AuxiliaryProcess: Send {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn line(&mut self, wait: Duration) -> Result<LinePoll, String>;
    fn wait(&mut self, wait: Duration) -> Result<Option<bool>, String>;
}
pub trait AuxiliaryLauncher<Request>: Send + Sync {
    fn launch(&self, request: Request) -> Result<Box<dyn AuxiliaryProcess>, String>;
}
#[derive(Debug, PartialEq)]
pub enum AuxiliaryError {
    Cancelled,
    Timeout,
    Failed,
    Io(String),
}

pub struct AuxiliarySession {
    process: Box<dyn AuxiliaryProcess>,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
}
impl AuxiliarySession {
    pub fn new(
        process: Box<dyn AuxiliaryProcess>,
        timeout: Duration,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            process,
            deadline: Instant::now() + timeout,
            cancel,
        }
    }
    pub fn check(&self) -> Result<(), AuxiliaryError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(AuxiliaryError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(AuxiliaryError::Timeout);
        }
        Ok(())
    }
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), AuxiliaryError> {
        self.check()?;
        self.process.send(bytes).map_err(AuxiliaryError::Io)
    }
    pub fn line(&mut self) -> Result<Option<String>, AuxiliaryError> {
        loop {
            self.check()?;
            match self
                .process
                .line(Duration::from_millis(100))
                .map_err(AuxiliaryError::Io)?
            {
                LinePoll::Line(line) => return Ok(Some(line)),
                LinePoll::Closed => return Ok(None),
                LinePoll::Pending => {}
            }
        }
    }
    pub fn finish(&mut self) -> Result<(), AuxiliaryError> {
        loop {
            self.check()?;
            match self
                .process
                .wait(Duration::from_millis(50))
                .map_err(AuxiliaryError::Io)?
            {
                Some(true) => return Ok(()),
                Some(false) => return Err(AuxiliaryError::Failed),
                None => {}
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        lines: VecDeque<LinePoll>,
        exits: VecDeque<Option<bool>>,
        dropped: Arc<AtomicBool>,
    }
    impl AuxiliaryProcess for Fake {
        fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
            assert_eq!(bytes, b"request\n");
            Ok(())
        }
        fn line(&mut self, _: Duration) -> Result<LinePoll, String> {
            Ok(self.lines.pop_front().unwrap())
        }
        fn wait(&mut self, _: Duration) -> Result<Option<bool>, String> {
            Ok(self.exits.pop_front().unwrap())
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Relaxed);
        }
    }
    fn session(
        timeout: Duration,
        cancel: bool,
        success: bool,
    ) -> (AuxiliarySession, Arc<AtomicBool>) {
        let dropped = Arc::new(AtomicBool::new(false));
        let process = Fake {
            lines: [
                LinePoll::Pending,
                LinePoll::Line("response".into()),
                LinePoll::Closed,
            ]
            .into(),
            exits: [None, Some(success)].into(),
            dropped: dropped.clone(),
        };
        (
            AuxiliarySession::new(
                Box::new(process),
                timeout,
                Arc::new(AtomicBool::new(cancel)),
            ),
            dropped,
        )
    }
    #[test]
    fn private_session_polls_then_returns_lines_and_exit_without_publication() {
        let (mut session, dropped) = session(Duration::from_secs(5), false, true);
        session.send(b"request\n").unwrap();
        assert_eq!(session.line().unwrap().as_deref(), Some("response"));
        assert_eq!(session.line().unwrap(), None);
        session.finish().unwrap();
        drop(session);
        assert!(dropped.load(Ordering::Relaxed));
    }
    #[test]
    fn cancellation_deadline_and_unsuccessful_exit_keep_distinct_errors() {
        for (timeout, cancel, expected) in [
            (Duration::ZERO, false, AuxiliaryError::Timeout),
            (Duration::from_secs(5), true, AuxiliaryError::Cancelled),
            (Duration::from_secs(5), false, AuxiliaryError::Failed),
        ] {
            let (mut session, dropped) = session(timeout, cancel, false);
            assert_eq!(session.finish(), Err(expected));
            drop(session);
            assert!(dropped.load(Ordering::Relaxed));
        }
    }
}
