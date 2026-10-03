//! One supporting terminal, independent of conversation lifecycle.
use crate::RuntimeEvents;
use portable_pty::CommandBuilder;
use prometeu_core::{
    lock::lock,
    tasks::TaskExecutor,
    terminal::{
        Terminal, TerminalControl, TerminalEvents, TerminalFactory, TerminalOutput, TerminalSize,
    },
};
use serde_json::{json, Value};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Condvar, Mutex},
    time::Duration,
};

pub trait ShellPreparation: Send + Sync {
    fn prepare(&self, workdir: &Path) -> CommandBuilder;
}
pub struct LoginShell(pub PathBuf);
impl ShellPreparation for LoginShell {
    fn prepare(&self, workdir: &Path) -> CommandBuilder {
        let mut command = CommandBuilder::new(&self.0);
        command.arg("-l");
        command.cwd(workdir);
        command.env("TERM", "xterm-256color");
        command
    }
}
pub struct ScriptShell {
    pub command: String,
    pub environment: Vec<(String, String)>,
}
pub struct EnvironmentShell {
    pub shell: LoginShell,
    pub environment: Vec<(String, String)>,
}
impl ShellPreparation for EnvironmentShell {
    fn prepare(&self, workdir: &Path) -> CommandBuilder {
        let mut command = self.shell.prepare(workdir);
        for (key, value) in &self.environment {
            command.env(key, value);
        }
        command
    }
}
impl ShellPreparation for ScriptShell {
    fn prepare(&self, workdir: &Path) -> CommandBuilder {
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-lc", &self.command]);
        command.cwd(workdir);
        command.env("TERM", "xterm-256color");
        for (key, value) in &self.environment {
            command.env(key, value);
        }
        command
    }
}
// Reader waits outside the scrollback lock, so snapshots and acknowledgements remain available.
#[derive(Default)]
struct Window {
    sent: u64,
    acknowledged: u64,
    closing: bool,
    detached: bool,
}
#[derive(Default)]
struct Flow {
    state: Mutex<Window>,
    changed: Condvar,
}
impl Flow {
    fn ready(&self) -> bool {
        let state = self
            .changed
            .wait_while(lock(&self.state), |s| {
                !s.closing && !s.detached && s.sent.saturating_sub(s.acknowledged) >= 64
            })
            .unwrap_or_else(|e| e.into_inner());
        !state.closing
    }
    fn sent(&self, seq: u64) {
        lock(&self.state).sent = seq;
    }
    fn acknowledge(&self, seq: u64) -> Result<(), String> {
        let mut state = lock(&self.state);
        if seq > state.sent {
            return Err("terminal acknowledgement exceeds delivered output".into());
        }
        state.acknowledged = state.acknowledged.max(seq);
        self.changed.notify_all();
        Ok(())
    }
    fn close(&self) {
        lock(&self.state).closing = true;
        self.changed.notify_all();
    }
}
struct Events {
    id: String,
    sink: Arc<dyn RuntimeEvents>,
    flow: Arc<Flow>,
}
impl TerminalEvents for Events {
    fn output(&self, bytes: &[u8], seq: u64) {
        self.flow.sent(seq);
        let _ = self.sink.publish(
            json!({"v":1,"terminal":{"kind":"output","id":self.id,"seq":seq,"data":bytes}}),
        );
    }
    fn closed(&self, code: Option<u32>) {
        let _ = self
            .sink
            .publish(json!({"v":1,"terminal":{"kind":"closed","id":self.id,"code":code}}));
    }
}
struct QueuedInput {
    queue: mpsc::SyncSender<Vec<u8>>,
    fault: Arc<Mutex<Option<String>>>,
}
impl Write for QueuedInput {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(error) = &*lock(&self.fault) {
            return Err(io::Error::other(error.clone()));
        }
        self.queue.try_send(data.to_vec()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("terminal input queue unavailable: {error}"),
            )
        })?;
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Active {
    id: String,
    terminal: Terminal,
    control: Arc<dyn TerminalControl>,
    done: Arc<(Mutex<u8>, Condvar)>,
    flow: Arc<Flow>,
}
pub struct TerminalService {
    workdir: PathBuf,
    factory: Arc<dyn TerminalFactory<CommandBuilder>>,
    shell: Arc<dyn ShellPreparation>,
    events: Arc<dyn RuntimeEvents>,
    executor: Arc<dyn TaskExecutor>,
    active: Option<Active>,
    header: Option<String>,
    pub script_name: Option<String>,
}
impl TerminalService {
    pub fn new(
        workdir: PathBuf,
        factory: Arc<dyn TerminalFactory<CommandBuilder>>,
        shell: Arc<dyn ShellPreparation>,
        events: Arc<dyn RuntimeEvents>,
        executor: Arc<dyn TaskExecutor>,
    ) -> Self {
        Self {
            workdir,
            factory,
            shell,
            events,
            executor,
            active: None,
            header: None,
            script_name: None,
        }
    }
    pub fn with_header(mut self, header: Option<String>) -> Self {
        self.header = header;
        self
    }
    pub fn open(&mut self, cols: u16, rows: u16) -> Result<Value, String> {
        let size = size(cols, rows)?;
        if self.active.is_some() {
            return Err("close the current terminal before opening another".into());
        }
        let started = self
            .factory
            .open(self.shell.prepare(&self.workdir), size)
            .map_err(|e| format!("terminal launch failed: {e:?}"))?;
        let id = uuid::Uuid::new_v4().to_string();
        let flow = Arc::new(Flow::default());
        let output = Arc::new(TerminalOutput::new(Arc::new(Events {
            id: id.clone(),
            sink: self.events.clone(),
            flow: flow.clone(),
        })));
        let control = started.control.clone();
        if let Some(header) = &self.header {
            output.feed(header.as_bytes());
        }
        let (queue, inputs) = mpsc::sync_channel::<Vec<u8>>(16);
        let fault = Arc::new(Mutex::new(None));
        let terminal = Terminal::new(
            Box::new(QueuedInput {
                queue,
                fault: fault.clone(),
            }),
            started.control,
            output.clone(),
            None,
        );
        let done = Arc::new((Mutex::new(0u8), Condvar::new()));
        let written = done.clone();
        let writing = flow.clone();
        let errors = self.events.clone();
        let input_id = id.clone();
        self.executor.spawn(Box::new(move || {
            let mut input = started.input;
            loop {
                if lock(&writing.state).closing {
                    break;
                }
                let data = match inputs.recv_timeout(Duration::from_millis(50)) {
                    Ok(data) => data,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if lock(&writing.state).closing {
                    break;
                }
                if let Err(error) = input.write_all(&data).and_then(|()| input.flush()) {
                    let error = error.to_string();
                    *lock(&fault) = Some(error.clone());
                    if !lock(&writing.state).closing {
                        let _ = errors.publish(
                            json!({"v":1,"terminal":{"kind":"error","id":input_id,"error":error}}),
                        );
                    }
                    break;
                }
            }
            drop(input);
            *lock(&written.0) += 1;
            written.1.notify_all();
        }))?;
        let completed = done.clone();
        let delivery = flow.clone();
        self.executor.spawn(Box::new(move || {
            let mut reader = started.output;
            let mut waiter = started.waiter;
            let mut chunk = [0; 8192];
            while let Ok(n) = reader.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if delivery.ready() {
                    output.feed(&chunk[..n]);
                }
            }
            let code = waiter.wait().ok();
            drop(waiter);
            output.finish(code, &[]);
            output.closed(code);
            *lock(&completed.0) += 1;
            completed.1.notify_all();
        }))?;
        self.active = Some(Active {
            id: id.clone(),
            terminal,
            control,
            done,
            flow,
        });
        Ok(json!({"id":id}))
    }
    fn active(&self, id: &str) -> Result<&Active, String> {
        self.active
            .as_ref()
            .filter(|active| active.id == id)
            .ok_or("terminal is no longer available".into())
    }
    pub fn write(&mut self, id: &str, data: &[u8]) -> Result<(), String> {
        if data.len() > 4096 {
            return Err("terminal input exceeds 4 KiB".into());
        }
        self.active(id)?;
        let terminal = &mut self.active.as_mut().unwrap().terminal;
        if !terminal.alive() {
            return Err("terminal has exited".into());
        }
        terminal.write_bytes(data)
    }
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        size(cols, rows)?;
        self.active(id)?.terminal.resize(cols, rows)
    }
    pub fn snapshot(&self, id: &str) -> Result<Value, String> {
        let active = self.active(id)?;
        let scroll = lock(&active.terminal.buffer);
        Ok(
            json!({"id":id,"data":scroll.bytes,"seq":scroll.seq,"running":active.terminal.alive(),"code":scroll.exit_code}),
        )
    }
    pub fn acknowledge(&self, id: &str, seq: u64) -> Result<(), String> {
        self.active(id)?.flow.acknowledge(seq)
    }
    pub fn close(&mut self, id: &str) -> Result<(), String> {
        self.stop(id)?;
        self.active = None;
        Ok(())
    }
    /// Stop execution while retaining its scrollback and observed exit code.
    pub fn stop(&mut self, id: &str) -> Result<(), String> {
        let active = self.active(id)?;
        active.flow.close();
        active.control.close();
        let (done, _) = active
            .done
            .1
            .wait_timeout_while(lock(&active.done.0), Duration::from_secs(5), |done| {
                *done != 2
            })
            .map_err(|e| e.to_string())?;
        if *done != 2 {
            return Err("terminal cleanup timed out; the handle is retained".into());
        }
        drop(done);
        Ok(())
    }
    pub fn current(&self) -> Result<Value, String> {
        match &self.active {
            Some(active) => self.snapshot(&active.id),
            None => Ok(Value::Null),
        }
    }
    pub fn attached(&self, attached: bool) {
        if let Some(active) = &self.active {
            let mut window = lock(&active.flow.state);
            window.detached = !attached;
            window.acknowledged = window.sent;
            active.flow.changed.notify_all();
        }
    }
    pub fn shutdown(&mut self) -> Result<(), String> {
        match self.active.as_ref().map(|active| active.id.clone()) {
            Some(id) => self.close(&id),
            None => Ok(()),
        }
    }
}
impl Drop for TerminalService {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
fn size(cols: u16, rows: u16) -> Result<TerminalSize, String> {
    if !(1..=500).contains(&cols) || !(1..=500).contains(&rows) {
        return Err("terminal dimensions must be between 1 and 500".into());
    }
    Ok(TerminalSize { cols, rows })
}
