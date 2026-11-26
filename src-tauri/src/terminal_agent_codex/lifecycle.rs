//! Explicit app-exit ownership; Tauri exit does not guarantee dropping retained engine handles.
use super::{lock, Error};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock, RwLock, RwLockReadGuard, Weak,
};
use std::thread::JoinHandle;

#[derive(Default)]
struct Registry {
    closing: AtomicBool,
    starts: RwLock<()>,
    engines: Mutex<Vec<Weak<EngineShutdown>>>,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

pub(super) fn closing() -> bool {
    registry().closing.load(Ordering::Acquire)
}

/// Hold until the child either fails and is reaped or its shutdown handle is registered.
/// Exit waits for bounded in-flight bootstrap, so a late start cannot escape the exit snapshot.
pub(super) fn admit() -> Result<RwLockReadGuard<'static, ()>, Error> {
    let registry = registry();
    if registry.closing.load(Ordering::Acquire) {
        return Err(Error::Closed);
    }
    let admission = registry.starts.read().unwrap_or_else(|e| e.into_inner());
    if registry.closing.load(Ordering::Acquire) {
        return Err(Error::Closed);
    }
    Ok(admission)
}

pub(super) struct EngineShutdown {
    stopped: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl EngineShutdown {
    pub(super) fn register(stopped: Arc<AtomicBool>, worker: JoinHandle<()>) -> Arc<Self> {
        let handle = Arc::new(Self {
            stopped,
            worker: Mutex::new(Some(worker)),
        });
        let mut engines = lock(&registry().engines);
        engines.retain(|engine| engine.strong_count() != 0);
        engines.push(Arc::downgrade(&handle));
        handle
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        // Hold the join gate through reaping. Concurrent stop/exit must also wait until the
        // child and its endpoint are gone, not merely until another caller took the handle.
        let mut worker = lock(&self.worker);
        if let Some(worker) = worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn shutdown() {
    let registry = registry();
    registry.closing.store(true, Ordering::Release);
    let _starts = registry.starts.write().unwrap_or_else(|e| e.into_inner());
    let engines: Vec<_> = lock(&registry.engines)
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
    // Signal all engines first; each worker owns bounded termination and the sole child wait.
    for engine in &engines {
        engine.stopped.store(true, Ordering::Release);
    }
    for engine in engines {
        engine.stop();
    }
}
