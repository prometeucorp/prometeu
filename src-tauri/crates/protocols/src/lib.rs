//! Provider protocol adapters shared by desktop and headless execution.
pub mod antigravity_store;
pub mod cli_version;
pub mod codex;
pub mod cursor;
pub mod cursor_store;
pub mod terminal_execution;
pub mod terminal_status;
pub mod terminal_transcript;
mod conversation {
    pub use prometeu_core::conversation::event;
    pub fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}
mod i18n {
    pub use prometeu_core::error::{code as t, with_args as ta};
    pub fn io(cause: impl ToString) -> String {
        ta("err.io", &[("cause", cause.to_string())])
    }
}

pub mod catalog;

pub mod account;
