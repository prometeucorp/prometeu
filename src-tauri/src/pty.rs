use crate::i18n;
use crate::lock::lock;
use crate::AppState;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// Retain 512 KB of terminal output for restoring a session's scrollback.
const SCROLLBACK: usize = 512 * 1024;

/// Allow graceful shutdown before escalating signals so agents can close transcripts and servers
/// can handle SIGHUP.
const GRACE: Duration = Duration::from_secs(1);

/// Escalate from SIGTERM to SIGKILL after the final grace period.
const REAP: Duration = Duration::from_millis(500);

/// Process-exit callback with its status; setup completion releases the initial agent message here.
pub type OnExit = Box<dyn FnOnce(Option<u32>) + Send>;

/// Dock script and shell PTY metadata. Agent conversations use chat.rs rather than this terminal
/// transport.
#[derive(Default)]
pub struct Dock {
    pub on_exit: Option<OnExit>,
    /// The configured Run entry, retained so callers do not mistake another live script for it.
    pub script_name: Option<String>,
    /// Write the app's setup-copy header before starting the reader thread so it cannot interleave
    /// with initial process output.
    pub header: Option<String>,
}

/// Keep buffered output and sequence under one lock. Snapshots must describe exactly which numbered
/// chunks they include so live forwarding can avoid gaps or duplicates.
#[derive(Default)]
pub struct Scroll {
    pub bytes: Vec<u8>,
    pub seq: u64,
    pub exit_code: Option<u32>,
}

impl Scroll {
    /// Append a chunk, enforce the scrollback limit, and return its sequence.
    pub fn absorb(&mut self, chunk: &[u8]) -> u64 {
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > SCROLLBACK {
            let cut = self.bytes.len() - SCROLLBACK;
            self.bytes.drain(..cut);
        }
        self.seq += 1;
        self.seq
    }
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    /// Retain recent session bytes for restoring the terminal.
    pub buffer: Arc<Mutex<Scroll>>,
    pub script_name: Option<String>,
    /// Distinguish running processes from retained scrollback entries after exit.
    alive: Arc<AtomicBool>,
    /// Suppress output from a killed or replaced PTY so the old reader cannot contaminate the
    /// replacement terminal.
    gone: Arc<AtomicBool>,
    /// The child's process group used during Drop; zero means unknown.
    pid: u32,
}

impl Pty {
    pub fn write(&mut self, data: &str) -> Result<(), String> {
        self.writer.write_all(data.as_bytes()).map_err(i18n::io)?;
        self.writer.flush().map_err(i18n::io)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(i18n::io)
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// The group leader anchors process-tree inspection, including descendants such as node started
    /// by npm.
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

/// Signal the child's process group to reach descendants. portable-pty creates a separate session
/// before exec, making the child's PID its group ID rather than the app's group. Signal only before
/// the child is reaped to avoid PID reuse; alive is cleared before wait.
pub(crate) fn signal_group(pid: u32, alive: &AtomicBool, sig: i32) {
    if pid == 0 || !alive.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: killpg has no memory-safety contract, and the unreaped child's PID remains reserved
    // for its process group.
    unsafe { libc::killpg(pid as libc::pid_t, sig) };
}

/// Wait until exit or the deadline; return true on exit.
pub(crate) fn wait_exit(alive: &AtomicBool, until: Duration) -> bool {
    let deadline = Instant::now() + until;
    while Instant::now() < deadline {
        if !alive.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    !alive.load(Ordering::Relaxed)
}

/// Removing a PTY must stop its entire process group. Rust Child does not kill on Drop, and
/// descendants can ignore SIGHUP. Send SIGHUP, then escalate to SIGTERM and SIGKILL on a separate
/// thread so closing the tab never waits for shutdown.
impl Drop for Pty {
    fn drop(&mut self) {
        self.gone.store(true, Ordering::Relaxed);
        signal_group(self.pid, &self.alive, libc::SIGHUP);
        let (pid, alive) = (self.pid, self.alive.clone());
        std::thread::spawn(move || {
            if wait_exit(&alive, GRACE) {
                return;
            }
            signal_group(pid, &alive, libc::SIGTERM);
            if wait_exit(&alive, REAP) {
                return;
            }
            signal_group(pid, &alive, libc::SIGKILL);
        });
    }
}

/// Remove the PTY from the map, triggering process shutdown.
pub fn kill(state: &AppState, key: &str) {
    lock(&state.ptys).remove(key);
}

/// Return the PTY handle plus the reader and child consumed by the output pump.
type Opened = (Pty, Box<dyn Read + Send>, Box<dyn Child + Send + Sync>);

/// Open the PTY and subprocess separately from Tauri output pumping so lifecycle behavior can be
/// tested without the app runtime.
fn open(cmd: CommandBuilder, cols: u16, rows: u16) -> Result<Opened, String> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| i18n::ta("err.pty.openpty", &[("cause", e.to_string())]))?;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| i18n::ta("err.pty.spawn", &[("cause", e.to_string())]))?;
    let pid = child.process_id().unwrap_or(0);
    drop(pair.slave); // Close the extra handle so child exit can deliver EOF.

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| i18n::ta("err.pty.reader", &[("cause", e.to_string())]))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| i18n::ta("err.pty.writer", &[("cause", e.to_string())]))?;

    let pty = Pty {
        master: pair.master,
        writer,
        buffer: Arc::new(Mutex::new(Scroll::default())),
        script_name: None,
        alive: Arc::new(AtomicBool::new(true)),
        gone: Arc::new(AtomicBool::new(false)),
        pid,
    };
    Ok((pty, reader, child))
}

fn reap(child: &mut dyn Child, alive: &AtomicBool, sink: &Mutex<Scroll>) -> Option<u32> {
    // Clear alive before wait so no later signal can target a reused PID.
    alive.store(false, Ordering::Relaxed);
    let code = child.wait().ok().map(|status| status.exit_code());
    lock(sink).exit_code = code;
    code
}

/// Forward numbered output under the dock key while all sessions continue in the background.
/// Persist the exit status in scrollback so setup failures remain visible after reopening the
/// panel.
pub fn spawn(
    app: &AppHandle,
    session_id: &str,
    cmd: CommandBuilder,
    cols: u16,
    rows: u16,
    dock: Dock,
) -> Result<Pty, String> {
    let Dock {
        on_exit,
        header,
        script_name,
    } = dock;
    let (mut pty, mut reader, mut child) = open(cmd, cols, rows)?;
    pty.script_name = script_name;

    if let Some(text) = header {
        let seq = lock(&pty.buffer).absorb(text.as_bytes());
        let _ = app.emit("pty", (session_id.to_string(), text.into_bytes(), seq));
    }

    let sink = pty.buffer.clone();
    let (alive_t, gone_t) = (pty.alive.clone(), pty.gone.clone());
    let app = app.clone();
    let id = session_id.to_string();

    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let seq = lock(&sink).absorb(&chunk[..n]);
                    // ponytail: raw bytes use a larger JSON array so the frontend TextDecoder can
                    // join split UTF-8; switch to base64 if transport cost matters. Include the
                    // sequence for snapshot reconciliation.
                    if !gone_t.load(Ordering::Relaxed) {
                        let _ = app.emit("pty", (id.clone(), chunk[..n].to_vec(), seq));
                    }
                }
            }
        }
        let code = reap(child.as_mut(), &alive_t, &sink);
        if !gone_t.load(Ordering::Relaxed) {
            let line = match code {
                Some(0) => format!(
                    "\r\n\x1b[32m✓ {}\x1b[0m\r\n",
                    i18n::pick("terminou", "finished")
                ),
                Some(n) => format!(
                    "\r\n\x1b[31m✗ {}\x1b[0m\r\n",
                    i18n::pick(
                        &format!("saiu com código {n}"),
                        &format!("exited with code {n}")
                    ),
                ),
                None => format!(
                    "\r\n\x1b[31m✗ {}\x1b[0m\r\n",
                    i18n::pick("encerrado", "stopped")
                ),
            };
            let seq = lock(&sink).absorb(line.as_bytes());
            let _ = app.emit("pty", (id.clone(), line.into_bytes(), seq));
        }
        if let Some(on_exit) = on_exit {
            on_exit(code);
        }
        let _ = app.emit("pty-closed", (id, code));
        crate::machine::publish_counts(&app);
    });

    Ok(pty)
}

#[tauri::command]
pub fn pty_write(state: State<AppState>, session: String, data: String) -> Result<(), String> {
    let mut ptys = lock(&state.ptys);
    ptys.get_mut(&session)
        .ok_or_else(|| i18n::t("err.pty.gone"))?
        .write(&data)
}

#[tauri::command]
pub fn pty_resize(
    state: State<AppState>,
    session: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let ptys = lock(&state.ptys);
    ptys.get(&session)
        .ok_or_else(|| i18n::t("err.pty.gone"))?
        .resize(cols, rows)
}

/// Return retained scrollback for terminal restoration.
#[tauri::command]
pub fn pty_buffer(state: State<AppState>, session: String) -> Vec<u8> {
    lock(&state.ptys)
        .get(&session)
        .map(|p| lock(&p.buffer).bytes.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_exit_code_and_output_remain_available_after_process_exit() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf service-output; exit 7"]);
        let (pty, mut reader, mut child) = open(command, 80, 24).unwrap();
        assert!(lock(&pty.buffer).exit_code.is_none());
        let mut chunk = [0; 1024];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 {
                break;
            }
            lock(&pty.buffer).absorb(&chunk[..n]);
        }
        assert_eq!(reap(child.as_mut(), &pty.alive, &pty.buffer), Some(7));
        assert!(!pty.alive());
        let scroll = lock(&pty.buffer);
        assert_eq!(scroll.exit_code, Some(7));
        assert_eq!(scroll.bytes, b"service-output");
    }

    /// Sequences identify exactly which chunks a snapshot includes without inspecting byte content.
    #[test]
    fn numbers_output_chunks_and_records_the_last_sequence_in_snapshots() {
        let mut s = Scroll::default();
        assert_eq!(s.absorb(b"a"), 1);
        assert_eq!(s.absorb(b"b"), 2);
        assert_eq!((s.bytes.as_slice(), s.seq), (&b"ab"[..], 2));
        assert_eq!(s.absorb(b"c"), 3);
        assert_eq!(s.bytes, b"abc");
    }

    /// Truncating old bytes must not reset the sequence.
    #[test]
    fn retention_limit_drops_the_start_without_changing_sequence_numbers() {
        let mut s = Scroll::default();
        s.absorb(&vec![b'x'; SCROLLBACK]);
        assert_eq!(s.absorb(b"end"), 2);
        assert_eq!(s.bytes.len(), SCROLLBACK);
        assert!(s.bytes.ends_with(b"end"));
    }

    /// Treat zombies as exited even though kill(pid, 0) reports their PID exists. Process state
    /// determines whether they can still use CPU or hold ports.
    fn running(pid: i32) -> bool {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("ps did not run");
        let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
        !stat.is_empty() && !stat.starts_with('Z')
    }

    fn stopped(pid: i32, until: Duration) -> bool {
        let deadline = Instant::now() + until;
        while Instant::now() < deadline {
            if !running(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        !running(pid)
    }

    /// Use a real PTY with separated output chunks to verify snapshot/live reconciliation through
    /// the same Scroll implementation used in production.
    #[test]
    fn snapshots_during_output_record_what_they_already_contain() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "printf first; sleep 0.4; printf second"]);
        let (pty, mut reader, _child) = open(cmd, 80, 24).expect("pty did not open");
        let scroll = pty.buffer.clone();

        let read_chunk = |reader: &mut Box<dyn Read + Send>| -> u64 {
            let mut chunk = [0u8; 1024];
            let n = reader.read(&mut chunk).expect("read failed");
            lock(&scroll).absorb(&chunk[..n])
        };

        // Capture the first chunk and the snapshot a remote viewer would receive.
        assert_eq!(read_chunk(&mut reader), 1);
        let (bytes, seq) = {
            let s = lock(&scroll);
            (s.bytes.clone(), s.seq)
        };
        assert_eq!(String::from_utf8_lossy(&bytes), "first");
        assert_eq!(
            seq, 1,
            "snapshot includes the first chunk and records its sequence"
        );

        // Only subsequent chunks belong to the live continuation.
        assert_eq!(read_chunk(&mut reader), 2);
        assert!(lock(&scroll).bytes.ends_with(b"second"));
        assert!(
            seq < lock(&scroll).seq,
            "new chunk sequence exceeds the snapshot sequence"
        );
    }

    /// Regression for dock shutdown leaving servers alive: stop both the direct child and a
    /// descendant that ignores SIGHUP. Group signalling and escalation to SIGTERM must reach both.
    #[test]
    fn ending_a_session_terminates_children_and_grandchildren() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args([
            "-c",
            "nohup sleep 30 >/dev/null 2>&1 & echo GRANDCHILD=$!; exec sleep 30",
        ]);
        let (pty, mut reader, _child) = open(cmd, 80, 24).expect("pty did not open");
        let child = pty.pid as i32;

        // Read until the descendant reports its PID.
        let mut output = String::new();
        let mut chunk = [0u8; 512];
        let deadline = Instant::now() + Duration::from_secs(5);
        let grandchild: i32 = loop {
            assert!(
                Instant::now() < deadline,
                "grandchild did not report its pid: {output:?}"
            );
            let n = reader.read(&mut chunk).expect("read failed");
            output.push_str(&String::from_utf8_lossy(&chunk[..n]));
            let digits: String = output
                .split("GRANDCHILD=")
                .nth(1)
                .unwrap_or("")
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if output.contains('\n') && !digits.is_empty() {
                break digits.parse().expect("unreadable pid");
            }
        };

        // Drain output and release the final master descriptor as production does, so the test
        // measures shutdown rather than a blocked terminal pipe.
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        });

        assert!(
            running(child),
            "child must be running before the test starts"
        );
        assert!(
            running(grandchild),
            "grandchild must be running before the test starts"
        );

        drop(pty);

        // Shutdown escalation waits on another thread.
        let deadline = GRACE + REAP + Duration::from_secs(2);
        assert!(
            stopped(child, deadline),
            "child {child} survived session shutdown"
        );
        assert!(
            stopped(grandchild, deadline),
            "grandchild {grandchild} survived session shutdown"
        );
    }
}
