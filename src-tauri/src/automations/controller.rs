//! In-process admission only. Durable branch/resource locks remain in core state.
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct Controller {
    active: Arc<Mutex<HashSet<String>>>,
    capacity: usize,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new(4)
    }
}

impl Controller {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            active: Arc::new(Mutex::new(HashSet::new())),
            capacity,
        }
    }

    /// Acquire before spawning and move the guard into the execution worker.
    /// The mutex is held only for admission; it never spans a provider request.
    pub(super) fn try_start(&self, run_id: &str) -> Option<RunGuard> {
        if run_id.is_empty() {
            return None;
        }
        let mut active = self.active.lock().ok()?;
        if active.len() >= self.capacity || !active.insert(run_id.to_owned()) {
            return None;
        }
        Some(RunGuard {
            active: Arc::clone(&self.active),
            run_id: run_id.to_owned(),
        })
    }
}

/// Intentionally not Clone: exactly one owner releases a worker admission slot.
pub(super) struct RunGuard {
    active: Arc<Mutex<HashSet<String>>>,
    run_id: String,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        // Release even if unrelated admission code poisoned the short-lived lock.
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        active.remove(&self.run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn duplicate_run_is_excluded_across_controller_clones() {
        let controller = Controller::default();
        let guard = controller.try_start("run-one").unwrap();
        assert!(controller.clone().try_start("run-one").is_none());
        drop(guard);
        assert!(controller.try_start("run-one").is_some());
    }

    #[test]
    fn independent_runs_use_bounded_slots_and_drop_releases_capacity() {
        let controller = Controller::new(2);
        let first = controller.try_start("run-one").unwrap();
        let second = controller.try_start("run-two").unwrap();
        assert!(controller.try_start("run-three").is_none());
        drop(first);
        let third = controller.try_start("run-three").unwrap();
        assert!(controller.try_start("run-four").is_none());
        drop((second, third));
        assert!(controller.try_start("run-four").is_some());
    }

    #[test]
    fn default_allows_four_workers_and_zero_capacity_admits_none() {
        let controller = Controller::default();
        let guards: Vec<_> = (0..4)
            .map(|id| controller.try_start(&id.to_string()).unwrap())
            .collect();
        assert!(controller.try_start("fifth").is_none());
        assert!(Controller::new(0).try_start("run").is_none());
        assert!(Controller::default().try_start("").is_none());
        drop(guards);
    }

    #[test]
    fn guard_is_send_static_and_unwinding_releases_slot() {
        fn assert_send_static<T: Send + 'static>() {}
        assert_send_static::<RunGuard>();
        let controller = Controller::new(1);
        let guard = controller.try_start("run").unwrap();
        let worker = std::thread::spawn(move || {
            let _guard = guard;
            panic!("simulated worker panic");
        });
        assert!(worker.join().is_err());
        assert!(controller.try_start("run").is_some());
    }

    #[test]
    fn concurrent_duplicate_admission_has_one_owner() {
        let controller = Controller::new(8);
        let start = Arc::new(Barrier::new(8));
        let acquired = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let controller = controller.clone();
                let start = Arc::clone(&start);
                let acquired = Arc::clone(&acquired);
                std::thread::spawn(move || {
                    start.wait();
                    let guard = controller.try_start("same-run");
                    let owns = guard.is_some();
                    // Hold the winning guard until every contender has tried.
                    acquired.wait();
                    drop(guard);
                    owns
                })
            })
            .collect();
        let winners = threads
            .into_iter()
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>();
        assert_eq!(winners, 1);
        assert!(controller.try_start("same-run").is_some());
    }
}
