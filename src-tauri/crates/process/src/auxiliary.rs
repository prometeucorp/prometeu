//! Private authentication transport. Protocols and application errors stay in provider adapters.
use prometeu_core::auxiliary::{AuxiliaryLauncher, AuxiliaryProcess, LinePoll};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_LINE: usize = 1_048_576;
pub struct UnixAuxiliaryLauncher;
impl AuxiliaryLauncher<Command> for UnixAuxiliaryLauncher {
    fn launch(&self, mut command: Command) -> Result<Box<dyn AuxiliaryProcess>, String> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let child = command.spawn().map_err(|e| e.to_string())?;
        let (send, lines) = mpsc::sync_channel(64);
        // Establish the cleanup owner before taking either pipe.
        let mut process = PrivateProcess {
            child,
            input: None,
            lines,
            exit: None,
        };
        process.input = process.child.stdin.take();
        let stdout = process
            .child
            .stdout
            .take()
            .ok_or("missing private output pipe")?;
        if process.input.is_none() {
            return Err("missing private input pipe".into());
        }
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                // Include room for CRLF without allocating an unbounded line before checking it.
                let read = reader
                    .by_ref()
                    .take((MAX_LINE + 3) as u64)
                    .read_line(&mut line);
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if line.ends_with('\n') {
                            line.pop();
                            if line.ends_with('\r') {
                                line.pop();
                            }
                        }
                        if line.len() > MAX_LINE || send.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Box::new(process))
    }
}
struct PrivateProcess {
    child: Child,
    input: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    exit: Option<bool>,
}
impl AuxiliaryProcess for PrivateProcess {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let input = self.input.as_mut().ok_or("missing private input pipe")?;
        input
            .write_all(bytes)
            .and_then(|()| input.flush())
            .map_err(|e| e.to_string())
    }
    fn line(&mut self, wait: Duration) -> Result<LinePoll, String> {
        Ok(match self.lines.recv_timeout(wait) {
            Ok(line) => LinePoll::Line(line),
            Err(mpsc::RecvTimeoutError::Timeout) => LinePoll::Pending,
            Err(mpsc::RecvTimeoutError::Disconnected) => LinePoll::Closed,
        })
    }
    fn wait(&mut self, wait: Duration) -> Result<Option<bool>, String> {
        let deadline = Instant::now() + wait;
        loop {
            if self.exit.is_some() {
                return Ok(self.exit);
            }
            self.exit = self
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .map(|s| s.success());
            if self.exit.is_some() || Instant::now() >= deadline {
                return Ok(self.exit);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for PrivateProcess {
    fn drop(&mut self) {
        // All polling, signaling and reaping require this unique mutable owner. A successful
        // try_wait retires the PID before any subsequent drop can signal it.
        if self.exit.is_none() && self.child.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::killpg(self.child.id() as libc::pid_t, libc::SIGKILL);
            }
        }
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests;
