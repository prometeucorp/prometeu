//! Recover poisoned mutexes instead of cascading one worker panic across the app. Callers retain
//! responsibility for the protected data's invariants when recovering it.

use std::sync::{Mutex, MutexGuard};

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poison| poison.into_inner())
}
