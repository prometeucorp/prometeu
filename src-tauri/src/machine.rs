//! Resource usage grouped by the app, conversations and dock terminals, including each process
//! subtree. One `ps` snapshot per tick supplies parent-child relationships. The app total excludes
//! separately listed agent and terminal roots. CPU comes from elapsed CPU time between ticks, not
//! lifetime `%CPU` averages.

use crate::background::{self, Context, Power};
use crate::lock::lock;
use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// A compact chip needs fewer snapshots than an open process panel.
fn sample_interval(context: Context, detail: bool) -> Option<Duration> {
    if !(context.visible && context.focused) {
        return None;
    }
    Some(Duration::from_secs(
        match (detail, context.power == Power::Ac) {
            (true, true) => 3,
            (true, false) => 5,
            (false, true) => 15,
            (false, false) => 30,
        },
    ))
}

fn sample_due(
    last: Option<Duration>,
    now: Duration,
    interval: Option<Duration>,
    returned: bool,
) -> bool {
    let Some(interval) = interval else {
        return false;
    };
    returned || last.is_none_or(|last| now.saturating_sub(last) >= interval)
}

fn cpu_percent(before: f64, after: f64, elapsed: Duration) -> f32 {
    if elapsed.is_zero() {
        return 0.0;
    }
    (((after - before) / elapsed.as_secs_f64()) * 100.0).max(0.0) as f32
}

/// A gap longer than twice the cadence that produced the samples, such as a hidden window or a
/// sleeping Mac, cannot be drawn as a continuous history.
fn history_gap(elapsed: Duration, cadence: Duration) -> bool {
    elapsed > cadence * 2
}
/// Keep 40 CPU samples: about two minutes at the detailed cadence.
const HISTORY: usize = 40;

/// One resource row for the app, a conversation or a terminal and its process subtree.
#[derive(Serialize, Clone, PartialEq)]
pub struct Proc {
    /// `app`, `chat` or `term` selects the icon.
    pub kind: String,
    /// The user-authored workspace title stays outside the translation catalog.
    pub name: String,
    /// The tab title for chats, or the dock key for terminals. The frontend translates built-in
    /// dock names.
    pub detail: String,
    pub rss: u64,
    pub cpu: f32,
    /// Recent CPU samples, oldest first.
    pub hist: Vec<f32>,
}

/// A workspace port currently serving traffic.
#[derive(Serialize, Clone, PartialEq)]
pub struct Port {
    /// The workspace whose browser opens when the port is selected.
    pub id: String,
    pub title: String,
    pub port: u16,
}

#[derive(Serialize, Clone, PartialEq, Default)]
pub struct Machine {
    pub rss: u64,
    pub cpu: f32,
    pub procs: Vec<Proc>,
    /// Live terminal count shown in the status bar.
    pub terms: usize,
    pub ports: Vec<Port>,
}

/// A process root and its display metadata.
struct Root {
    pid: u32,
    kind: &'static str,
    name: String,
    detail: String,
}

/// CPU totals and history for live roots. Removed processes leave the cache.
#[derive(Default)]
struct Memo {
    /// Cumulative subtree CPU time in seconds at the previous sample.
    time: HashMap<u32, f64>,
    hist: HashMap<u32, Vec<f32>>,
    last: Option<Instant>,
    sent: Machine,
}

pub struct Service {
    detail: Mutex<bool>,
    changed: Condvar,
    epoch: AtomicU64,
    cache: Mutex<Machine>,
}

impl Service {
    pub fn new() -> Self {
        Self {
            detail: Mutex::new(false),
            changed: Condvar::new(),
            epoch: AtomicU64::new(0),
            cache: Mutex::new(Machine::default()),
        }
    }

    /// Change the epoch under the mutex the sampler waits on, so a wake between its epoch check
    /// and its wait cannot be lost.
    fn wake(&self) {
        let _detail = lock(&self.detail);
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.changed.notify_all();
    }
}

#[tauri::command]
pub fn set_resource_detail(app: AppHandle, open: bool) {
    let service = app.state::<Service>();
    let mut detail = lock(&service.detail);
    *detail = open;
    service.epoch.fetch_add(1, Ordering::Relaxed);
    service.changed.notify_all();
}

/// Terminal and port chips follow PTY starts, exits and port assignments without a `ps` sample.
pub fn publish_counts(app: &AppHandle) {
    let (Some(service), Some(state)) = (app.try_state::<Service>(), app.try_state::<AppState>())
    else {
        return;
    };
    let (terms, ports) = (alive_terms(&state), ports(&state));
    let mut cache = lock(&service.cache);
    if cache.terms == terms && cache.ports == ports {
        return;
    }
    cache.terms = terms;
    cache.ports = ports;
    let _ = app.emit("machine", &*cache);
}

/// The sampler waits while the native window is not foreground, then refreshes on return.
pub fn watch(app: AppHandle) {
    let waker = app.clone();
    app.state::<background::State>()
        .on_change(move || waker.state::<Service>().wake());
    std::thread::spawn(move || {
        let mut memo = Memo::default();
        let started = Instant::now();
        let mut sampled: Option<Duration> = None;
        let mut cadence: Option<Duration> = None;
        let mut was_foreground = false;
        let mut was_detail = false;
        loop {
            let service = app.state::<Service>();
            let epoch = service.epoch.load(Ordering::Relaxed);
            let context = app.state::<background::State>().snapshot();
            let detail = *lock(&service.detail);
            let foreground = context.visible && context.focused;
            let interval = sample_interval(context, detail);
            let now = started.elapsed();
            if sample_due(
                sampled,
                now,
                interval,
                foreground && !was_foreground || detail && !was_detail,
            ) {
                sampled = Some(now);
                // History continues across a cadence change, such as opening the panel after
                // compact samples, but not across a gap longer than either cadence allows.
                let current = interval.unwrap_or_default();
                let limit = cadence.map_or(current, |previous| previous.max(current));
                cadence = Some(current);
                if let Some(next) = read(&app, &mut memo, limit) {
                    *lock(&service.cache) = next.clone();
                    let _ = app.emit("machine", &next);
                }
            }
            was_foreground = foreground;
            was_detail = detail;
            let delay = interval
                .map(|interval| {
                    sampled
                        .map(|last| (last + interval).saturating_sub(started.elapsed()))
                        .unwrap_or(Duration::ZERO)
                })
                .unwrap_or(Duration::from_secs(60))
                .max(Duration::from_millis(100));
            let guard = lock(&service.detail);
            if service.epoch.load(Ordering::Relaxed) == epoch {
                let _ = service.changed.wait_timeout(guard, delay);
            }
        }
    });
}

/// The cached snapshot returned when the panel opens before the next sample.
#[tauri::command]
pub fn machine(state: tauri::State<AppState>, service: tauri::State<Service>) -> Machine {
    let mut cached = lock(&service.cache).clone();
    cached.terms = alive_terms(&state);
    cached.ports = ports(&state);
    cached
}

/// Return a changed snapshot, or nothing if `ps` fails or the displayed state is unchanged.
fn read(app: &AppHandle, memo: &mut Memo, cadence: Duration) -> Option<Machine> {
    let state = app.state::<AppState>();
    let table = snapshot()?;
    let now = Instant::now();
    let elapsed = memo.last.replace(now).map(|last| now - last);

    let roots = roots(&state);
    let mut procs = Vec::new();
    let mut time = HashMap::new();
    let mut hist = HashMap::new();
    // Exclude separately listed process roots from the app's resource total.
    let mine: Vec<(u64, f64)> = roots
        .iter()
        .filter(|r| r.kind != "app")
        .map(|r| table.sum(r.pid))
        .collect();

    for root in &roots {
        let (mut rss, mut secs) = table.sum(root.pid);
        if root.kind == "app" {
            for (child_rss, child_secs) in &mine {
                rss = rss.saturating_sub(*child_rss);
                secs = (secs - child_secs).max(0.0);
            }
        }
        if rss == 0 && secs == 0.0 {
            continue;
        }
        // The first sample has no elapsed-time baseline, so CPU starts at zero.
        let cpu = match (memo.time.get(&root.pid), elapsed) {
            (Some(before), Some(gap)) => cpu_percent(*before, secs, gap),
            _ => 0.0,
        };
        let mut line = if elapsed.is_some_and(|gap| history_gap(gap, cadence)) {
            Vec::new()
        } else {
            memo.hist.get(&root.pid).cloned().unwrap_or_default()
        };
        line.push(cpu);
        if line.len() > HISTORY {
            line.drain(..line.len() - HISTORY);
        }
        time.insert(root.pid, secs);
        hist.insert(root.pid, line.clone());
        procs.push(Proc {
            kind: root.kind.to_string(),
            name: root.name.clone(),
            detail: root.detail.clone(),
            rss,
            cpu,
            hist: line,
        });
    }
    memo.time = time;
    memo.hist = hist;

    let next = Machine {
        rss: procs.iter().map(|p| p.rss).sum(),
        cpu: procs.iter().map(|p| p.cpu).sum(),
        procs,
        terms: alive_terms(&state),
        ports: ports(&state),
    };
    (next != memo.sent).then(|| {
        memo.sent = next.clone();
        next
    })
}

/// Include the app and live chats and terminals. Stopped entries retain scrollback but use no
/// process resources. Clone the board before locking process maps to avoid reversing the chat
/// lifecycle lock order.
fn roots(state: &tauri::State<AppState>) -> Vec<Root> {
    let board = lock(&state.board).clone();
    let title_of = |id: &str| {
        board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.title.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let mut roots = vec![Root {
        pid: std::process::id(),
        kind: "app",
        name: "Prometeu".to_string(),
        detail: String::new(),
    }];
    for (id, chat) in lock(&state.chats).iter() {
        if !chat.alive() {
            continue;
        }
        // Use workspace and tab titles so resource rows identify the conversation.
        let named = board.workspaces.iter().find_map(|w| {
            let tab = w.tabs.iter().find(|t| &t.id == id)?;
            Some((w.title.clone(), tab.title.clone()))
        });
        let (name, detail) = named.unwrap_or_else(|| (id.clone(), String::new()));
        roots.push(Root {
            pid: chat.pid(),
            kind: "chat",
            name,
            detail,
        });
    }
    for (key, pty) in lock(&state.ptys).iter() {
        if !pty.alive() {
            continue;
        }
        let (id, kind) = key.split_once(':').unwrap_or((key.as_str(), ""));
        roots.push(Root {
            pid: pty.pid(),
            kind: "term",
            name: title_of(id),
            detail: kind.to_string(),
        });
    }
    roots
}

fn alive_terms(state: &tauri::State<AppState>) -> usize {
    lock(&state.ptys).values().filter(|p| p.alive()).count()
}

/// Expose ports belonging to live Run scripts. This lists app-managed workspaces rather than
/// scanning the machine.
fn ports(state: &tauri::State<AppState>) -> Vec<Port> {
    let ptys = lock(&state.ptys);
    let board = lock(&state.board);
    let mut ports: Vec<Port> = board
        .workspaces
        .iter()
        .filter_map(|w| {
            let port = w.port?;
            let up = ptys
                .get(&format!("{}:run", w.id))
                .is_some_and(|pty| pty.alive());
            up.then(|| Port {
                id: w.id.clone(),
                title: w.title.clone(),
                port,
            })
        })
        .collect();
    ports.sort_by_key(|process| process.port);
    ports
}

/* ---------- process snapshot ---------- */

/// The process tree and each process's resource usage.
struct Table {
    rss: HashMap<u32, u64>,
    /// Cumulative process CPU time in seconds.
    time: HashMap<u32, f64>,
    kids: HashMap<u32, Vec<u32>>,
}

impl Table {
    /// Sum this process and every descendant.
    fn sum(&self, pid: u32) -> (u64, f64) {
        let mut rss = *self.rss.get(&pid).unwrap_or(&0);
        let mut time = *self.time.get(&pid).unwrap_or(&0.0);
        for kid in self.kids.get(&pid).map(Vec::as_slice).unwrap_or(&[]) {
            let (r, t) = self.sum(*kid);
            rss += r;
            time += t;
        }
        (rss, time)
    }
}

/// Read all processes in one `ps` invocation per tick instead of forking once per conversation.
fn snapshot() -> Option<Table> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-Ao", "pid=,ppid=,rss=,time="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut table = Table {
        rss: HashMap::new(),
        time: HashMap::new(),
        kids: HashMap::new(),
    };
    for line in text.lines() {
        let mut cols = line.split_whitespace();
        let (Some(pid), Some(ppid), Some(rss), Some(time)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let (Ok(pid), Ok(ppid), Ok(rss)) = (pid.parse(), ppid.parse::<u32>(), rss.parse::<u64>())
        else {
            continue;
        };
        // macOS `ps` reports RSS in KiB.
        table.rss.insert(pid, rss * 1024);
        table.time.insert(pid, cpu_time(time));
        table.kids.entry(ppid).or_default().push(pid);
    }
    (!table.rss.is_empty()).then_some(table)
}

/// Parse `ps` TIME (`MM:SS.cc` or `HH:MM:SS.cc`) into seconds. Unknown formats yield zero until the
/// next sample.
fn cpu_time(text: &str) -> f64 {
    let parts: Vec<&str> = text.split(':').collect();
    let mut seconds = 0.0;
    for part in &parts {
        let Ok(n) = part.parse::<f64>() else {
            return 0.0;
        };
        seconds = seconds * 60.0 + n;
    }
    seconds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::{Context, Power};

    fn context(visible: bool, focused: bool, power: Power) -> Context {
        Context {
            visible,
            focused,
            power,
            revision: 0,
        }
    }

    #[test]
    fn sampling_budget_counts_ps_launches_by_demand() {
        let ac = context(true, true, Power::Ac);
        let battery = context(true, true, Power::Battery);
        assert_eq!(sample_interval(ac, false), Some(Duration::from_secs(15)));
        assert_eq!(sample_interval(ac, true), Some(Duration::from_secs(3)));
        assert_eq!(
            sample_interval(battery, false),
            Some(Duration::from_secs(30))
        );
        assert_eq!(sample_interval(battery, true), Some(Duration::from_secs(5)));
        assert_eq!(sample_interval(context(false, true, Power::Ac), true), None);
        assert_eq!(
            sample_interval(context(true, false, Power::Ac), false),
            None
        );
        assert!(sample_due(
            None,
            Duration::ZERO,
            sample_interval(ac, false),
            false
        ));
        assert!(!sample_due(
            Some(Duration::ZERO),
            Duration::from_secs(14),
            sample_interval(ac, false),
            false
        ));
        assert!(sample_due(
            Some(Duration::ZERO),
            Duration::from_secs(15),
            sample_interval(ac, false),
            false
        ));
        assert!(sample_due(
            Some(Duration::from_secs(14)),
            Duration::from_secs(15),
            sample_interval(ac, false),
            true
        ));
    }

    #[test]
    fn real_elapsed_time_drives_cpu_and_a_long_gap_resets_history() {
        assert_eq!(cpu_percent(10.0, 10.6, Duration::from_secs(3)), 20.0);
        assert_eq!(cpu_percent(10.0, 10.6, Duration::from_secs(30)), 2.0);
        let (detailed, compact) = (Duration::from_secs(3), Duration::from_secs(15));
        assert!(history_gap(Duration::from_secs(15), detailed));
        assert!(!history_gap(Duration::from_secs(5), detailed));
        // Compact samples keep their history, so the panel does not open empty.
        assert!(!history_gap(Duration::from_secs(15), compact));
        assert!(history_gap(Duration::from_secs(31), compact));
    }

    #[test]
    fn cpu_time_is_measured_in_seconds() {
        assert!((cpu_time("0:12.34") - 12.34).abs() < 0.001);
        assert!((cpu_time("2:30.00") - 150.0).abs() < 0.001);
        assert!((cpu_time("1:00:00.00") - 3600.0).abs() < 0.001);
        assert_eq!(cpu_time("-"), 0.0);
    }

    #[test]
    fn subtrees_sum_child_processes() {
        let table = Table {
            rss: HashMap::from([(1, 100), (2, 20), (3, 3)]),
            time: HashMap::from([(1, 10.0), (2, 2.0), (3, 0.5)]),
            kids: HashMap::from([(1, vec![2]), (2, vec![3])]),
        };
        assert_eq!(table.sum(1), (123, 12.5));
        assert_eq!(table.sum(2), (23, 2.5));
        assert_eq!(table.sum(9), (0, 0.0));
    }

    /// A real `ps` snapshot includes at least the current test process.
    #[test]
    fn local_ps_returns_the_process_tree() {
        let table = snapshot().expect("ps did not respond");
        let me = std::process::id();
        assert!(table.rss.contains_key(&me), "current pid was missing");
        assert!(table.sum(me).0 > 0);
    }
}
