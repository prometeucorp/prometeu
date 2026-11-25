use super::*;
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Events(Mutex<Vec<(Vec<u8>, u64)>>, Mutex<Vec<Option<u32>>>);
impl TerminalEvents for Events {
    fn output(&self, bytes: &[u8], seq: u64) {
        lock(&self.0).push((bytes.to_vec(), seq));
    }
    fn closed(&self, code: Option<u32>) {
        lock(&self.1).push(code);
    }
}
#[test]
fn raw_chunks_headers_and_exit_notices_share_snapshot_sequence() {
    let events = Arc::new(Events::default());
    let output = TerminalOutput::new(events.clone());
    output.feed(b"header\r\n");
    output.feed(&[0xf0, 0x9f]);
    let snapshot = {
        let s = lock(&output.buffer);
        (s.bytes.clone(), s.seq)
    };
    output.feed(&[0x98, 0x80]);
    output.finish(Some(7), b"exit 7");
    output.closed(Some(7));
    let s = lock(&output.buffer);
    assert_eq!(snapshot.1, 2);
    assert_eq!(s.seq, 4);
    assert_eq!(s.exit_code, Some(7));
    let live: Vec<u8> = lock(&events.0)
        .iter()
        .filter(|(_, seq)| *seq > snapshot.1)
        .flat_map(|(b, _)| b.clone())
        .collect();
    assert_eq!([snapshot.0, live].concat(), s.bytes);
    assert_eq!(*lock(&events.1), [Some(7)]);
}
#[test]
fn retention_discards_bytes_without_resetting_the_sequence() {
    let mut scroll = Scroll::default();
    scroll.absorb(&vec![b'x'; SCROLLBACK]);
    assert_eq!(scroll.absorb(b"end"), 2);
    assert_eq!(scroll.bytes.len(), SCROLLBACK);
    assert!(scroll.bytes.ends_with(b"end"));
}
#[test]
fn retired_output_cannot_publish_into_a_replacement() {
    let events = Arc::new(Events::default());
    let old = TerminalOutput::new(events.clone());
    old.feed(b"old");
    old.retire();
    let replacement = TerminalOutput::new(events.clone());
    replacement.feed(b"new");
    old.feed(b"late bytes");
    old.finish(Some(1), b"late exit");
    old.closed(Some(1));
    assert_eq!(
        *lock(&events.0),
        [(b"old".to_vec(), 1), (b"new".to_vec(), 1)]
    );
    assert!(lock(&events.1).is_empty());
    assert_eq!(lock(&old.buffer).exit_code, Some(1));
}
#[test]
fn snapshot_waits_until_numbered_delivery_finishes() {
    struct Blocking(
        std::sync::mpsc::SyncSender<()>,
        Mutex<std::sync::mpsc::Receiver<()>>,
    );
    impl TerminalEvents for Blocking {
        fn output(&self, _: &[u8], _: u64) {
            self.0.send(()).unwrap();
            lock(&self.1).recv().unwrap();
        }
        fn closed(&self, _: Option<u32>) {}
    }
    let (entered, wait) = std::sync::mpsc::sync_channel(0);
    let (release, gate) = std::sync::mpsc::sync_channel(0);
    let output = Arc::new(TerminalOutput::new(Arc::new(Blocking(
        entered,
        Mutex::new(gate),
    ))));
    let worker = output.clone();
    let thread = std::thread::spawn(move || worker.feed(b"chunk"));
    wait.recv().unwrap();
    assert!(output.buffer.try_lock().is_err());
    release.send(()).unwrap();
    thread.join().unwrap();
    assert_eq!(lock(&output.buffer).seq, 1);
}
#[test]
fn dropping_the_terminal_retires_output_and_requests_native_shutdown_once() {
    struct Control(AtomicUsize);
    impl TerminalControl for Control {
        fn resize(&self, _: TerminalSize) -> Result<(), String> {
            Ok(())
        }
        fn running(&self) -> bool {
            true
        }
        fn system_id(&self) -> u32 {
            10
        }
        fn close(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let control = Arc::new(Control(AtomicUsize::new(0)));
    let events = Arc::new(Events::default());
    let output = Arc::new(TerminalOutput::new(events.clone()));
    let terminal = Terminal::new(Box::new(Vec::new()), control.clone(), output.clone(), None);
    drop(terminal);
    output.feed(b"late");
    assert!(lock(&events.0).is_empty());
    assert_eq!(control.0.load(Ordering::Relaxed), 1);
}
#[test]
fn dock_keys_belong_only_to_their_exact_owner() {
    assert_eq!(dock_kind("workspace:run", "workspace"), Some("run"));
    assert_eq!(dock_kind("/repo:terminal-2", "/repo"), Some("terminal-2"));
    assert_eq!(dock_kind("/repo:v2:terminal", "/repo:v2"), Some("terminal"));
    for (key, owner) in [
        ("/repo:v2:terminal", "/repo"),
        ("/repository:terminal", "/repo"),
        ("/repo", "/repo"),
    ] {
        assert_eq!(dock_kind(key, owner), None, "{key} is not owned by {owner}");
    }
}

struct Threads;
impl TaskExecutor for Threads {
    fn spawn(&self, task: crate::tasks::Task) -> Result<(), String> {
        std::thread::spawn(task);
        Ok(())
    }
}
/// Exits with `code` when the test releases it, as a reaped child would.
struct Exits(mpsc::Receiver<u32>);
impl TerminalWait for Exits {
    fn wait(&mut self) -> Result<u32, String> {
        self.0.recv().map_err(|e| e.to_string())
    }
}
fn eventually(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn process_exit_ends_reading_even_when_the_terminal_never_reports_eof() {
    let (exit, exits) = mpsc::channel();
    let mut watch = TerminalExit::watch(Box::new(Exits(exits)), &Threads, Duration::ZERO).unwrap();
    assert!(!watch.drained(), "a running process keeps its reader open");
    exit.send(0).unwrap();
    assert!(eventually(|| watch.drained()));
    assert_eq!(watch.wait(), Ok(0));
}

#[test]
fn trailing_output_gets_the_drain_period_after_exit() {
    let (exit, exits) = mpsc::channel();
    let mut watch =
        TerminalExit::watch(Box::new(Exits(exits)), &Threads, Duration::from_secs(60)).unwrap();
    exit.send(3).unwrap();
    // The exit is observed, but the reader still has time for the last bytes.
    assert!(eventually(|| {
        watch.observe();
        watch.exited.is_some()
    }));
    assert!(!watch.drained());
    assert_eq!(watch.wait(), Ok(3));
}

#[test]
fn eof_before_exit_still_waits_for_the_exit_code() {
    let (exit, exits) = mpsc::channel();
    let watch = TerminalExit::watch(Box::new(Exits(exits)), &Threads, Duration::ZERO).unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        exit.send(9).unwrap();
    });
    assert_eq!(watch.wait(), Ok(9));
}

#[test]
fn a_failed_waiter_still_ends_the_terminal() {
    let (exit, exits) = mpsc::channel::<u32>();
    drop(exit);
    let mut watch = TerminalExit::watch(Box::new(Exits(exits)), &Threads, Duration::ZERO).unwrap();
    assert!(eventually(|| watch.drained()));
    assert!(watch.wait().is_err());
}

#[test]
fn a_rejected_waiter_task_reports_the_scheduling_error() {
    struct Reject;
    impl TaskExecutor for Reject {
        fn spawn(&self, task: crate::tasks::Task) -> Result<(), String> {
            drop(task);
            Err("thread limit".into())
        }
    }
    let (_exit, exits) = mpsc::channel();
    assert_eq!(
        TerminalExit::watch(Box::new(Exits(exits)), &Reject, Duration::ZERO).err(),
        Some("thread limit".into())
    );
}
