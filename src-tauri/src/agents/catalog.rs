//! Account-scoped, bounded discovery. Raw provider responses never cross IPC.
use crate::{accounts, paths, state::ProviderId};
#[cfg(test)]
use prometeu_core::agents::Model;
use prometeu_core::command::QueryLauncher;
use std::{
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const TIMEOUT: Duration = Duration::from_secs(20);

pub use prometeu_core::agents::{CatalogError, ModelCatalog};
use prometeu_protocols::catalog::{command_output, query_claude, query_codex};
#[cfg(test)]
pub(super) use prometeu_protocols::catalog::{parse_claude, parse_codex_page, CatalogProcess};

pub(super) fn fetch(
    launcher: &dyn QueryLauncher<Command>,
    agent: ProviderId,
) -> Result<ModelCatalog, CatalogError> {
    if agent == ProviderId::RetiredGemini {
        return Err(CatalogError::new("unavailable"));
    }
    fetch_for_profile(launcher, agent, accounts::active(agent))
}

fn fetch_for_profile(
    launcher: &dyn QueryLauncher<Command>,
    agent: ProviderId,
    profile: Result<accounts::Profile, String>,
) -> Result<ModelCatalog, CatalogError> {
    let profile = profile.map_err(|_| CatalogError::new("noAccount"))?;
    crate::accounts::prepare_profile(&profile).map_err(|_| CatalogError::new("failed"))?;
    let mut command = match agent {
        ProviderId::Claude => {
            let mut cmd = Command::new("claude");
            cmd.args([
                "-p",
                "--verbose",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--no-session-persistence",
                "--settings",
                r#"{"hooks":{}}"#,
            ]);
            cmd.env_clear();
            for (key, value) in std::env::vars_os() {
                if !key.to_string_lossy().starts_with("CLAUDE") {
                    cmd.env(key, value);
                }
            }
            cmd
        }
        ProviderId::Codex => {
            let mut cmd = Command::new("codex");
            cmd.arg("app-server");
            cmd
        }
        ProviderId::Antigravity => {
            let mut cmd = Command::new("agy");
            cmd.arg("models");
            cmd
        }
        ProviderId::RetiredGemini => unreachable!(),
    };
    command.current_dir(paths::home());
    crate::accounts::apply_profile(&profile, &mut command)
        .map_err(|_| CatalogError::new("failed"))?;
    let models = match agent {
        ProviderId::Claude => query_claude(launcher, &mut command, TIMEOUT)?,
        ProviderId::Codex => {
            query_codex(launcher, &mut command, TIMEOUT, env!("CARGO_PKG_VERSION"))?
        }
        ProviderId::Antigravity => {
            crate::antigravity::parse_catalog(&command_output(launcher, &mut command, TIMEOUT)?)?
        }
        ProviderId::RetiredGemini => unreachable!(),
    };
    Ok(ModelCatalog {
        models,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    })
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
