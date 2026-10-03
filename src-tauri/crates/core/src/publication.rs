//! Ordered board publication with injected persistence and event delivery.

use crate::{board::Board, lock::lock};
use std::sync::{
    mpsc::{channel, Sender},
    Arc, Mutex,
};
use std::time::Duration;

/// Storage owns decoding, backup recovery and atomic replacement. The application owns revival.
pub trait BoardStore: Send + Sync {
    fn load(&self) -> Board;
    fn save(&self, board: &Board) -> Result<(), String>;
}

/// Deliver an immutable snapshot in publication order. Implementations must not reenter publication.
pub trait BoardEvents {
    fn changed(&self, board: &Board) -> Result<(), String>;
}

const COALESCE: Duration = Duration::from_millis(250);

enum Save {
    Later(Arc<Board>),
    Now(Arc<Board>, Sender<Result<(), String>>),
}

/// One queue orders deferred snapshots and durable barriers. Clones share that ordering.
#[derive(Clone)]
pub struct BoardPublisher {
    tx: Sender<Save>,
    publication: Arc<Mutex<()>>,
    store: Arc<dyn BoardStore>,
}

impl BoardPublisher {
    pub fn new(store: Arc<dyn BoardStore>) -> Self {
        let (tx, rx) = channel::<Save>();
        let worker_store = store.clone();
        std::thread::spawn(move || {
            while let Ok(first) = rx.recv() {
                let (mut board, mut flushes) = match first {
                    Save::Later(board) => {
                        std::thread::sleep(COALESCE);
                        (board, Vec::new())
                    }
                    Save::Now(board, done) => (board, vec![done]),
                };
                while let Ok(newer) = rx.try_recv() {
                    match newer {
                        Save::Later(next) => board = next,
                        Save::Now(next, done) => {
                            board = next;
                            flushes.push(done);
                        }
                    }
                }
                let result = worker_store.save(&board);
                if let Err(error) = &result {
                    eprintln!("could not persist board: {error}");
                }
                for done in flushes {
                    let _ = done.send(result.clone());
                }
            }
        });
        Self {
            tx,
            publication: Arc::new(Mutex::new(())),
            store,
        }
    }

    /// Queue and emit the same snapshot; disk I/O and delivery never hold the board mutation lock.
    pub fn publish(&self, current: &Mutex<Board>, events: &dyn BoardEvents) -> Result<(), String> {
        let _publication = lock(&self.publication);
        let board = Self::snapshot(current);
        if self.tx.send(Save::Later(board.clone())).is_err() {
            // Only a dead worker permits a synchronous fallback; it can no longer overwrite us.
            if let Err(error) = self.store.save(&board) {
                eprintln!("could not persist board after losing the saver: {error}");
            }
        }
        events.changed(&board)
    }

    /// Return a storage failure to callers that must persist ownership before starting effects.
    pub fn persist(&self, current: &Mutex<Board>) -> Result<(), String> {
        let _publication = lock(&self.publication);
        let board = Self::snapshot(current);
        let (tx, rx) = channel();
        if self.tx.send(Save::Now(board.clone(), tx)).is_err() {
            return self.store.save(&board);
        }
        // No timeout or retry on disk failure: a still-running write must not race a fallback.
        rx.recv().unwrap_or_else(|_| self.store.save(&board))
    }

    fn snapshot(current: &Mutex<Board>) -> Arc<Board> {
        let mut board = lock(current);
        board.prepare_telemetry_ids();
        Arc::new(board.clone())
    }
}

#[cfg(test)]
mod tests;
