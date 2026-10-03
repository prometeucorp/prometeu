//! Desktop compatibility facade for canonical primitives and the host clock.

use prometeu_core::conversation::Clock;
pub use prometeu_core::conversation::{agent_activity, event};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

pub fn now() -> u64 {
    SystemClock.now()
}
