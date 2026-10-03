//! Unix execution adapter shared by the local desktop and future headless hosts.
#![cfg(unix)]

use prometeu_core::lock::lock;
use prometeu_core::process::{
    LaunchError, ProcessControl, ProcessExit, ProcessHandle, ProcessLauncher, ProcessOutput,
    ProcessWait, StartedProcess,
};
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One native thread per accepted task; process workers require independent blocking progress.
pub struct ThreadExecutor;
impl prometeu_core::tasks::TaskExecutor for ThreadExecutor {
    fn spawn(&self, task: prometeu_core::tasks::Task) -> Result<(), String> {
        std::thread::Builder::new()
            .spawn(task)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

pub struct UnixProcessLauncher;

impl ProcessLauncher<Command> for UnixProcessLauncher {
    fn launch(&self, mut command: Command) -> Result<StartedProcess, LaunchError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let child = command
            .spawn()
            .map_err(|error| LaunchError::Spawn(error.to_string()))?;
        let group = Arc::new(Group {
            pid: child.id(),
            state: Mutex::new(ChildState {
                child: Some(child),
                exit: None,
            }),
            running: AtomicBool::new(true),
        });
        // The waiter owns cleanup even if extracting a pipe fails after spawn.
        let waiter = Waiter(group.clone());
        let (input, stdout, stderr) = {
            let mut state = lock(&group.state);
            let child = state.child.as_mut().expect("new child");
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        };
        let input = input.ok_or(LaunchError::Pipe)?;
        let stdout = output_lines(stdout.ok_or(LaunchError::Pipe)?);
        let stderr = output_lines(stderr.ok_or(LaunchError::Pipe)?);
        Ok(StartedProcess {
            input: Box::new(input),
            stdout,
            stderr,
            handle: ProcessHandle::new(group),
            waiter: Box::new(waiter),
        })
    }
}

/// Unbounded draining prevents stdin/output pipe deadlock while publication is blocked. This
/// retains the current memory-growth limit: sustained output during stalls can grow the queue.
pub fn output_lines(output: impl Read + Send + 'static) -> ProcessOutput {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Box::new(rx.into_iter())
}

struct ChildState {
    child: Option<Child>,
    exit: Option<ProcessExit>,
}

struct Group {
    pid: u32,
    state: Mutex<ChildState>,
    running: AtomicBool,
}

impl Group {
    fn signal(&self, signal: i32) {
        let state = lock(&self.state);
        if state.child.is_none() {
            return;
        }
        // SAFETY: the child PID is reserved until reaping, which takes this same lock. process_group
        // creates a dedicated group; killpg has no memory-safety preconditions.
        unsafe { libc::killpg(self.pid as libc::pid_t, signal) };
    }

    fn wait(&self) -> Result<ProcessExit, String> {
        loop {
            {
                let mut state = lock(&self.state);
                if let Some(exit) = state.exit {
                    return Ok(exit);
                }
                let child = state.child.as_mut().expect("unreaped child");
                if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                    let exit = ProcessExit {
                        code: status.code(),
                    };
                    state.exit = Some(exit);
                    state.child = None;
                    self.running.store(false, Ordering::Release);
                    return Ok(exit);
                }
            }
            // Never hold the signaling lock during a blocking child wait.
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl ProcessControl for Group {
    fn system_id(&self) -> u32 {
        self.pid
    }
    fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    fn interrupt(&self) {
        self.signal(libc::SIGINT);
    }
    fn terminate(&self) {
        self.signal(libc::SIGTERM);
    }
    fn kill(&self) {
        self.signal(libc::SIGKILL);
    }
    fn wait_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        !self.running()
    }
}

struct Waiter(Arc<Group>);

impl ProcessWait for Waiter {
    fn wait(&mut self) -> Result<ProcessExit, String> {
        self.0.wait()
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        // A partially constructed host or a panicking output worker must not orphan the child.
        self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests;

pub mod terminal;

pub mod auxiliary;

pub mod command;
pub mod query;
