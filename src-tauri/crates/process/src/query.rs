//! Bounded private line queries. Provider protocols remain in the caller.
use crate::command::{io, nonblocking, spawn_error, OwnedChild};
use prometeu_core::command::{CommandError, QueryLauncher, QueryPolicy, QueryProcess};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub struct UnixQueryLauncher;
impl QueryLauncher<Command> for UnixQueryLauncher {
    fn launch(
        &self,
        command: &mut Command,
        policy: QueryPolicy,
    ) -> Result<Box<dyn QueryProcess>, CommandError> {
        let deadline = Instant::now() + policy.timeout;
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(spawn_error)?;
        let mut owner = OwnedChild::new(child);
        let input = owner
            .child
            .stdin
            .take()
            .ok_or_else(|| CommandError::Io("missing query input".into()))?;
        nonblocking(&input)?;
        let stdout = owner
            .child
            .stdout
            .take()
            .ok_or_else(|| CommandError::Io("missing query output".into()))?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut total = 0usize;
            loop {
                let mut line = String::new();
                let remaining = policy.max_output.saturating_sub(total);
                let read = reader
                    .by_ref()
                    .take(remaining.saturating_add(1) as u64)
                    .read_line(&mut line);
                match read {
                    Ok(0) => break,
                    Ok(size) if size <= remaining => {
                        total += size;
                        if tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {
                        let _ = tx.send(Err(CommandError::OutputLimit));
                        break;
                    }
                    Err(_) => {
                        let _ = tx.send(Err(CommandError::InvalidOutput));
                        break;
                    }
                }
            }
        });
        Ok(Box::new(Query {
            owner,
            input: Some(input),
            lines,
            deadline,
        }))
    }
}
struct Query {
    owner: OwnedChild,
    input: Option<ChildStdin>,
    lines: Receiver<Result<String, CommandError>>,
    deadline: Instant,
}
impl QueryProcess for Query {
    fn send(&mut self, mut bytes: &[u8]) -> Result<(), CommandError> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| CommandError::Io("query input closed".into()))?;
        while !bytes.is_empty() {
            if Instant::now() >= self.deadline {
                return Err(CommandError::Timeout);
            }
            match input.write(bytes) {
                Ok(0) => return Err(CommandError::Io("query input closed".into())),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(io(e)),
            }
        }
        Ok(())
    }
    fn next(&mut self) -> Result<Option<String>, CommandError> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or(CommandError::Timeout)?;
        match self.lines.recv_timeout(remaining) {
            Ok(line) => line.map(Some),
            Err(RecvTimeoutError::Timeout) => Err(CommandError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
        }
    }
    fn close_input(&mut self) {
        self.input.take();
    }
    fn finish(&mut self) -> Result<bool, CommandError> {
        loop {
            if Instant::now() >= self.deadline {
                return Err(CommandError::Timeout);
            }
            if let Some(status) = self.owner.poll()? {
                return Ok(status.success());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
#[cfg(test)]
mod tests;
