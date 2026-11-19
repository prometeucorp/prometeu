//! Native PTYs with one owner for group signaling and child reaping.
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use prometeu_core::lock::lock;
use prometeu_core::process::{ProcessControl, ShutdownPolicy};
use prometeu_core::terminal::{
    StartedTerminal, TerminalControl, TerminalError, TerminalFactory, TerminalSize, TerminalWait,
};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Handles of the terminal or multiplexer that launched Prometeu, and markers of an agent session
/// it may run under. They describe where Prometeu came from, never one of its own terminals.
const LAUNCHER_ENV: &[&str] = &[
    "ITERM_SESSION_ID",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "KITTY_WINDOW_ID",
    "WEZTERM_PANE",
    "WT_SESSION",
    "TERM_SESSION_ID",
    "TMUX",
    "TMUX_PANE",
    "STY",
    "ZELLIJ",
    "ZELLIJ_SESSION_NAME",
    "ZELLIJ_PANE_ID",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SSE_PORT",
    "CODEX_THREAD_ID",
];

/// A dock shell or script runs in Prometeu's own terminal: it presents itself as Prometeu and
/// carries neither the launching terminal's identity nor a parent agent's session. `version` is
/// the app's, when the host knows it. Apply before the workspace variables, which it leaves alone.
pub fn own_terminal(command: &mut CommandBuilder, version: Option<&str>) {
    for key in LAUNCHER_ENV {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    command.env("TERM_PROGRAM", "Prometeu");
    match version {
        Some(version) => command.env("TERM_PROGRAM_VERSION", version),
        None => command.env_remove("TERM_PROGRAM_VERSION"),
    }
}

pub struct UnixTerminalFactory;
impl TerminalFactory<CommandBuilder> for UnixTerminalFactory {
    fn open(
        &self,
        command: CommandBuilder,
        size: TerminalSize,
    ) -> Result<StartedTerminal, TerminalError> {
        open_with_streams(command, size, native_streams)
    }
}
/// Compatibility factory for hosts that require cancellable IO. Both Unix factories now share
/// descriptor ownership, so explicit disconnect and waiter cleanup can cancel every stream.
pub struct ResponsiveTerminalFactory;
impl TerminalFactory<CommandBuilder> for ResponsiveTerminalFactory {
    fn open(
        &self,
        command: CommandBuilder,
        size: TerminalSize,
    ) -> Result<StartedTerminal, TerminalError> {
        open_with_streams(command, size, native_streams)
    }
}
type Streams = (Box<dyn Read + Send>, Box<dyn Write + Send>);
type OpenStreams = fn(&dyn MasterPty) -> Result<Streams, TerminalError>;
fn open_with_streams(
    command: CommandBuilder,
    size: TerminalSize,
    streams: OpenStreams,
) -> Result<StartedTerminal, TerminalError> {
    let pair = native_pty_system()
        .openpty(native_size(size))
        .map_err(|e| TerminalError::Open(e.to_string()))?;
    let program = command
        .get_argv()
        .first()
        .and_then(|program| Path::new(program).file_name())
        .map(OsStr::to_os_string);
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| TerminalError::Spawn(e.to_string()))?;
    let group = Arc::new(Group {
        pid: child.process_id().unwrap_or(0),
        child: Mutex::new(ChildState {
            child: Some(child),
            exit: None,
        }),
        running: AtomicBool::new(true),
    });
    drop(pair.slave);
    // Every operation under an IO lock must be nonblocking so disconnect can take and close all
    // descriptors even while a host retains the reader, writer or control handle.
    let result = nonblocking(pair.master.as_ref()).and_then(|()| streams(pair.master.as_ref()));
    let (output, input) = match result {
        Ok(streams) => streams,
        Err(error) => {
            drop(pair.master);
            group.kill();
            let _ = group.wait();
            return Err(error);
        }
    };
    let io = Arc::new(TerminalIo {
        master: Mutex::new(Some(pair.master)),
        reader: Mutex::new(Some(output)),
        writer: Mutex::new(Some(input)),
    });
    Ok(StartedTerminal {
        input: Box::new(SharedWriter(io.clone())),
        output: Box::new(SharedReader(io.clone())),
        process: prometeu_core::process::ProcessHandle::new(group.clone()),
        control: Arc::new(Control {
            io: io.clone(),
            group: group.clone(),
            closing: AtomicBool::new(false),
            program,
        }),
        waiter: Box::new(Waiter { group, io }),
    })
}
fn native_streams(master: &dyn MasterPty) -> Result<Streams, TerminalError> {
    let output = master
        .try_clone_reader()
        .map_err(|e| TerminalError::Reader(e.to_string()))?;
    let input = master
        .take_writer()
        .map_err(|e| TerminalError::Writer(e.to_string()))?;
    Ok((output, input))
}
fn nonblocking(master: &dyn MasterPty) -> Result<(), TerminalError> {
    let fd = master
        .as_raw_fd()
        .ok_or_else(|| TerminalError::Writer("PTY descriptor unavailable".into()))?;
    // SAFETY: the master owns this live descriptor; cloned streams share its file description.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(TerminalError::Writer(
            io::Error::last_os_error().to_string(),
        ));
    }
    Ok(())
}
struct TerminalIo {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    reader: Mutex<Option<Box<dyn Read + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
}
impl TerminalIo {
    fn disconnect(&self) {
        // Closing only the original master is insufficient: cloned descriptors keep the PTY
        // alive, and on macOS can keep a signaled slave process in kernel exit indefinitely.
        lock(&self.writer).take();
        lock(&self.reader).take();
        lock(&self.master).take();
    }
}
struct SharedReader(Arc<TerminalIo>);
impl Read for SharedReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            let result = match lock(&self.0.reader).as_mut() {
                Some(reader) => reader.read(bytes),
                None => return Ok(0),
            };
            match result {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}
struct SharedWriter(Arc<TerminalIo>);
fn disconnected() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "terminal disconnected")
}
impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            let result = match lock(&self.0.writer).as_mut() {
                Some(writer) => writer.write(bytes),
                None => return Err(disconnected()),
            };
            match result {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        lock(&self.0.writer)
            .as_mut()
            .ok_or_else(disconnected)?
            .flush()
    }
}

/// The name the system shows for a process, as `ps -o comm=` does; Linux truncates it to 15 bytes.
#[cfg(target_os = "macos")]
fn process_name(pid: u32) -> Option<OsString> {
    let mut name = [0u8; 64];
    // SAFETY: proc_name writes at most the given length into the buffer and returns how much.
    let written = unsafe {
        libc::proc_name(
            libc::c_int::try_from(pid).ok()?,
            name.as_mut_ptr().cast(),
            name.len() as u32,
        )
    };
    let written = usize::try_from(written)
        .ok()
        .filter(|&written| written > 0)?;
    Some(OsString::from_vec(name[..written].to_vec()))
}
#[cfg(target_os = "linux")]
fn process_name(pid: u32) -> Option<OsString> {
    let name = std::fs::read(format!("/proc/{pid}/comm")).ok()?;
    Some(OsString::from_vec(
        name.strip_suffix(b"\n").unwrap_or(&name).to_vec(),
    ))
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_name(_: u32) -> Option<OsString> {
    None
}

/// A login shell listed in /etc/shells, or the program the terminal started even when unlisted.
fn is_shell(name: &OsStr, program: Option<&OsStr>) -> bool {
    program == Some(name)
        || std::fs::read_to_string("/etc/shells").is_ok_and(|shells| {
            shells
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with('#'))
                .any(|path| Path::new(path).file_name() == Some(name))
        })
}

fn native_size(size: TerminalSize) -> PtySize {
    PtySize {
        cols: size.cols,
        rows: size.rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}
struct ChildState {
    child: Option<Box<dyn Child + Send + Sync>>,
    exit: Option<u32>,
}
struct Group {
    pid: u32,
    child: Mutex<ChildState>,
    running: AtomicBool,
}
impl Group {
    fn signal(&self, signal: i32) {
        let mut state = lock(&self.child);
        let Some(child) = state.child.as_mut() else {
            return;
        };
        if self.pid == 0 {
            if signal == libc::SIGKILL {
                let _ = child.kill();
            }
            return;
        }
        // SAFETY: portable-pty starts a dedicated session. Its unreaped leader reserves the PID;
        // signaling and try_wait use the same lock, so no signal can race PID retirement.
        unsafe {
            libc::killpg(self.pid as libc::pid_t, signal);
        }
    }
    fn wait(&self) -> Result<u32, String> {
        loop {
            {
                let mut state = lock(&self.child);
                if let Some(code) = state.exit {
                    return Ok(code);
                }
                if let Some(exit) = state
                    .child
                    .as_mut()
                    .expect("unreaped terminal")
                    .try_wait()
                    .map_err(|e| e.to_string())?
                {
                    let code = exit.exit_code();
                    state.exit = Some(code);
                    state.child = None;
                    self.running.store(false, Ordering::Release);
                    return Ok(code);
                }
            }
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
struct Control {
    io: Arc<TerminalIo>,
    group: Arc<Group>,
    closing: AtomicBool,
    /// The file name of the program the terminal started, normally the person's shell.
    program: Option<OsString>,
}
impl TerminalControl for Control {
    fn resize(&self, size: TerminalSize) -> Result<(), String> {
        lock(&self.io.master)
            .as_ref()
            .ok_or_else(|| "terminal disconnected".to_owned())?
            .resize(native_size(size))
            .map_err(|e| e.to_string())
    }
    fn running(&self) -> bool {
        self.group.running()
    }
    /// An interactive shell moves each job into its own process group and hands it the terminal,
    /// so a foreground group other than the shell's own is a command still running. `exec` keeps
    /// the shell's process and group, so in its own group the shell waits for input only while its
    /// process is still a shell, as after `exec zsh`.
    fn running_command(&self) -> bool {
        let leader = lock(&self.io.master)
            .as_ref()
            .and_then(|master| master.process_group_leader());
        match leader.and_then(|leader| u32::try_from(leader).ok()) {
            Some(leader) if leader == self.group.pid => {
                process_name(leader).is_some_and(|name| !is_shell(&name, self.program.as_deref()))
            }
            Some(_) => true,
            None => false,
        }
    }
    fn system_id(&self) -> u32 {
        self.group.pid
    }
    fn disconnect(&self) {
        self.io.disconnect();
    }
    fn close(&self) {
        if self.closing.swap(true, Ordering::Relaxed) {
            return;
        }
        self.group.signal(libc::SIGHUP);
        self.disconnect();
        let group = self.group.clone();
        std::thread::spawn(move || ShutdownPolicy::TERMINAL.shutdown(group.as_ref()));
    }
}
struct Waiter {
    group: Arc<Group>,
    io: Arc<TerminalIo>,
}
impl TerminalWait for Waiter {
    fn wait(&mut self) -> Result<u32, String> {
        self.group.wait()
    }
}
impl Drop for Waiter {
    fn drop(&mut self) {
        if self.group.running() {
            self.io.disconnect();
            self.group.kill();
            let _ = self.group.wait();
        }
    }
}

#[cfg(test)]
mod tests;
