use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct Store {
    written: Mutex<Vec<String>>,
    fail: AtomicBool,
}

impl BoardStore for Store {
    fn load(&self) -> Board {
        Board::default()
    }

    fn save(&self, board: &Board) -> Result<(), String> {
        lock(&self.written).push(board.stages[0].clone());
        if self.fail.load(Ordering::SeqCst) {
            return Err("disk unavailable".into());
        }
        Ok(())
    }
}

struct Events<F>(F);

impl<F: Fn(&Board) -> Result<(), String>> BoardEvents for Events<F> {
    fn changed(&self, board: &Board) -> Result<(), String> {
        (self.0)(board)
    }
}

fn current(stage: &str) -> Mutex<Board> {
    Mutex::new(Board {
        stages: vec![stage.into()],
        ..Board::default()
    })
}

#[test]
fn concurrent_publications_keep_snapshot_and_emission_order() {
    let current = current("old");
    let store = Arc::new(Store::default());
    let publisher = BoardPublisher::new(store.clone());
    let emitted = Mutex::new(Vec::new());
    let (captured, captured_rx) = channel();
    let (release, release_rx) = channel();
    let (competing, competing_rx) = channel();
    std::thread::scope(|scope| {
        let (publisher, current, emitted) = (&publisher, &current, &emitted);
        scope.spawn(move || {
            publisher
                .publish(
                    current,
                    &Events(|board: &Board| {
                        captured.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        assert!(
                            current.try_lock().is_ok(),
                            "delivery must release the board"
                        );
                        lock(emitted).push(board.stages[0].clone());
                        Ok(())
                    }),
                )
                .unwrap();
        });
        captured_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        lock(current).stages[0] = "new".into();
        scope.spawn(move || {
            assert!(publisher.publication.try_lock().is_err());
            competing.send(()).unwrap();
            publisher
                .publish(
                    current,
                    &Events(|board: &Board| {
                        lock(emitted).push(board.stages[0].clone());
                        Ok(())
                    }),
                )
                .unwrap();
        });
        competing_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
    });
    publisher.persist(&current).unwrap();
    assert_eq!(*lock(&emitted), ["old", "new"]);
    assert_eq!(lock(&store.written).last().map(String::as_str), Some("new"));
}

#[test]
fn durable_barrier_supersedes_pending_snapshots_and_keeps_worker_available() {
    let current = current("old");
    let store = Arc::new(Store::default());
    let publisher = BoardPublisher::new(store.clone());
    publisher
        .publish(&current, &Events(|_: &Board| Ok(())))
        .unwrap();
    lock(&current).stages[0] = "durable".into();
    publisher.persist(&current).unwrap();
    assert_eq!(
        lock(&store.written).last().map(String::as_str),
        Some("durable")
    );
    lock(&current).stages[0] = "later".into();
    publisher.persist(&current).unwrap();
    let written = lock(&store.written);
    let durable = written.iter().position(|stage| stage == "durable").unwrap();
    assert_eq!(&written[durable..], ["durable", "later"]);
}

#[test]
fn durable_storage_failure_is_returned_without_an_implicit_second_write() {
    let store = Arc::new(Store::default());
    store.fail.store(true, Ordering::SeqCst);
    let publisher = BoardPublisher::new(store.clone());
    assert_eq!(
        publisher.persist(&current("failed")),
        Err("disk unavailable".into())
    );
    assert_eq!(*lock(&store.written), ["failed"]);
    store.fail.store(false, Ordering::SeqCst);
    publisher.persist(&current("recovered")).unwrap();
    assert_eq!(*lock(&store.written), ["failed", "recovered"]);
}

#[test]
fn failed_delivery_does_not_cancel_persistence() {
    let current = current("accepted");
    let store = Arc::new(Store::default());
    let publisher = BoardPublisher::new(store.clone());
    assert_eq!(
        publisher.publish(
            &current,
            &Events(|_: &Board| Err("client disconnected".into()))
        ),
        Err("client disconnected".into())
    );
    publisher.persist(&current).unwrap();
    assert_eq!(
        lock(&store.written).last().map(String::as_str),
        Some("accepted")
    );
}

#[test]
fn storage_runs_without_holding_the_board_mutation_lock() {
    struct InspectingStore(Arc<Mutex<Board>>);
    impl BoardStore for InspectingStore {
        fn load(&self) -> Board {
            Board::default()
        }
        fn save(&self, _: &Board) -> Result<(), String> {
            assert!(self.0.try_lock().is_ok(), "storage must release the board");
            Ok(())
        }
    }
    let current = Arc::new(current("ready"));
    let publisher = BoardPublisher::new(Arc::new(InspectingStore(current.clone())));
    publisher.persist(&current).unwrap();
}
