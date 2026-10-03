//! Finite Unix commands with concurrent nonblocking pipe progress and owned cleanup.
use prometeu_core::command::{
    CommandError, CommandOutput, CommandPolicy, CommandRunner, OutputPolicy,
};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub(crate) fn io(error: std::io::Error) -> CommandError {
    CommandError::Io(error.to_string())
}
pub(crate) fn spawn_error(error: std::io::Error) -> CommandError {
    match error.kind() {
        std::io::ErrorKind::NotFound => CommandError::Unavailable,
        _ => io(error),
    }
}
pub(crate) fn nonblocking(pipe: &impl AsRawFd) -> Result<(), CommandError> {
    let fd = pipe.as_raw_fd();
    // SAFETY: the owning pipe keeps fd valid for both calls. F_GETFL/F_SETFL take no pointers.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(())
}
/// Unique mutable ownership serializes every reap and signal; a retained status retires the PID.
pub(crate) struct OwnedChild {
    pub child: Child,
    exit: Option<ExitStatus>,
}
impl OwnedChild {
    pub fn new(child: Child) -> Self {
        Self { child, exit: None }
    }
    pub fn poll(&mut self) -> Result<Option<ExitStatus>, CommandError> {
        if self.exit.is_none() {
            self.exit = self.child.try_wait().map_err(io)?;
        }
        Ok(self.exit)
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.exit.is_none() {
            // SAFETY: the dedicated group's leader has not been reaped by this unique owner.
            // Kill before wait so descendants holding pipes cannot keep cleanup blocked.
            unsafe {
                libc::killpg(self.child.id() as libc::pid_t, libc::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
fn stdio(policy: OutputPolicy) -> Stdio {
    match policy {
        OutputPolicy::Capture { .. } => Stdio::piped(),
        OutputPolicy::Discard => Stdio::null(),
        OutputPolicy::Inherit => Stdio::inherit(),
    }
}
struct Capture {
    pipe: Option<Box<dyn Read + Send>>,
    bytes: Vec<u8>,
    limit: usize,
}
impl Capture {
    fn new<T: Read + AsRawFd + Send + 'static>(
        pipe: Option<T>,
        policy: OutputPolicy,
    ) -> Result<Self, CommandError> {
        let pipe = pipe
            .map(|pipe| {
                nonblocking(&pipe)?;
                Ok(Box::new(pipe) as Box<dyn Read + Send>)
            })
            .transpose()?;
        let limit = match policy {
            OutputPolicy::Capture { limit } => limit,
            _ => 0,
        };
        Ok(Self {
            pipe,
            bytes: vec![],
            limit,
        })
    }
    fn drain(&mut self) -> Result<bool, CommandError> {
        let Some(pipe) = self.pipe.as_mut() else {
            return Ok(false);
        };
        let mut bytes = [0; 8192];
        match pipe.read(&mut bytes) {
            Ok(0) => {
                self.pipe = None;
                Ok(true)
            }
            Ok(n) => {
                if n > self.limit.saturating_sub(self.bytes.len()) {
                    return Err(CommandError::OutputLimit);
                }
                self.bytes.extend_from_slice(&bytes[..n]);
                Ok(true)
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(io(e)),
        }
    }
}
pub struct UnixCommandRunner;
impl CommandRunner<Command> for UnixCommandRunner {
    fn run(
        &self,
        command: &mut Command,
        input: &[u8],
        policy: CommandPolicy,
    ) -> Result<CommandOutput, CommandError> {
        let deadline = Instant::now() + policy.timeout;
        command
            .stdin(match input.is_empty() {
                true => Stdio::null(),
                false => Stdio::piped(),
            })
            .stdout(stdio(policy.stdout))
            .stderr(stdio(policy.stderr))
            .process_group(0);
        let mut owner = OwnedChild::new(command.spawn().map_err(spawn_error)?);
        let mut stdin = owner.child.stdin.take();
        if let Some(pipe) = &stdin {
            nonblocking(pipe)?;
        }
        let mut stdout = Capture::new(owner.child.stdout.take(), policy.stdout)?;
        let mut stderr = Capture::new(owner.child.stderr.take(), policy.stderr)?;
        let mut remaining = input;
        loop {
            if Instant::now() >= deadline {
                return Err(CommandError::Timeout);
            }
            let mut progressed = stdout.drain()?;
            progressed |= stderr.drain()?;
            if let Some(pipe) = stdin.as_mut() {
                match pipe.write(&remaining[..remaining.len().min(8192)]) {
                    Ok(0) => return Err(CommandError::Io("command input closed".into())),
                    Ok(n) => {
                        remaining = &remaining[n..];
                        progressed = true;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(io(e)),
                }
                if remaining.is_empty() {
                    stdin = None;
                }
            }
            // Keep the leader unreaped while a descendant could still hold a captured pipe.
            // Timeout cleanup can then signal the group without targeting a recycled PID.
            if stdin.is_none() && stdout.pipe.is_none() && stderr.pipe.is_none() {
                if let Some(status) = owner.poll()? {
                    return Ok(CommandOutput {
                        success: status.success(),
                        stdout: stdout.bytes,
                        stderr: stderr.bytes,
                    });
                }
            }
            if !progressed {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
#[cfg(test)]
mod tests;
