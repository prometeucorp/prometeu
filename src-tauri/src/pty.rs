//! Desktop terminal composition. Byte ordering and native lifecycle are injected boundaries.
use crate::lock::lock;
use crate::{i18n, AppState};
use portable_pty::CommandBuilder;
pub use prometeu_core::terminal::{Scroll, Terminal as Pty};
use prometeu_core::terminal::{TerminalError, TerminalEvents, TerminalOutput, TerminalSize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

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

struct DesktopTerminalEvents {
    app: AppHandle,
    id: String,
}
impl TerminalEvents for DesktopTerminalEvents {
    fn output(&self, bytes: &[u8], seq: u64) {
        let _ = self.app.emit("pty", (&self.id, bytes, seq));
    }
    fn closed(&self, code: Option<u32>) {
        let _ = self.app.emit("pty-closed", (&self.id, code));
    }
}
fn launch_error(error: TerminalError) -> String {
    let (code, cause) = match error {
        TerminalError::Open(cause) => ("err.pty.openpty", cause),
        TerminalError::Spawn(cause) => ("err.pty.spawn", cause),
        TerminalError::Reader(cause) => ("err.pty.reader", cause),
        TerminalError::Writer(cause) => ("err.pty.writer", cause),
    };
    i18n::ta(code, &[("cause", cause)])
}
pub fn kill(state: &AppState, key: &str) {
    lock(&state.ptys).remove(key);
}

/// Shell command construction and completion hooks belong to the host. The factory owns the
/// native PTY; core output keeps headers, chunks, exit notices and snapshots in one order.
pub fn spawn(
    app: &AppHandle,
    session_id: &str,
    cmd: CommandBuilder,
    cols: u16,
    rows: u16,
    dock: Dock,
) -> Result<Pty, String> {
    let started = app
        .state::<AppState>()
        .terminal_factory
        .open(cmd, TerminalSize { cols, rows })
        .map_err(launch_error)?;
    let output = Arc::new(TerminalOutput::new(Arc::new(DesktopTerminalEvents {
        app: app.clone(),
        id: session_id.into(),
    })));
    let pty = Pty::new(
        started.input,
        started.control,
        output.clone(),
        dock.script_name,
    );
    if let Some(header) = dock.header {
        output.feed(header.as_bytes());
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut reader = started.output;
        let mut waiter = started.waiter;
        let mut chunk = [0; 8192];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 {
                break;
            }
            output.feed(&chunk[..n]);
        }
        let code = waiter.wait().ok();
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

        output.finish(code, line.as_bytes());
        // Setup callbacks still run after deliberate closure, releasing pending agent prompts.
        if let Some(on_exit) = dock.on_exit {
            on_exit(code);
        }
        output.closed(code);
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
        .map_err(i18n::io)
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
        .map_err(i18n::io)
}

/// Return retained scrollback for terminal restoration.
#[tauri::command]
pub fn pty_buffer(state: State<AppState>, session: String) -> Vec<u8> {
    lock(&state.ptys)
        .get(&session)
        .map(|p| lock(&p.buffer).bytes.clone())
        .unwrap_or_default()
}
