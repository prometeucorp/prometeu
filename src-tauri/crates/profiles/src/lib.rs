//! Native Unix account profiles. Hosts supply roots and private file writes; no desktop is required.
#![cfg(unix)]
use prometeu_core::{
    accounts::{key, Account},
    board::ProviderId,
    error::code,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

pub mod claude;
pub mod codex;
mod files;

#[derive(Clone)]
pub struct Profile {
    pub id: String,
    pub provider: ProviderId,
    pub home: PathBuf,
    pub managed: bool,
    pub revision: u64,
}

/// Local execution-side interface. Native paths and commands never cross a desktop/runtime bridge.
pub trait ProfileBackend: Send + Sync {
    fn resolve(&self, account: &Account) -> Result<Profile, String>;
    fn prepare(&self, profile: &Profile) -> Result<(), String>;
    fn apply(&self, profile: &Profile, command: &mut Command) -> Result<(), String>;
}

/// Implementations return application-encoded errors and preserve private permissions/atomic writes.
pub trait ProfileFiles: Send + Sync {
    fn ensure_private_dir(&self, path: &Path) -> Result<(), String>;
    fn write_private(&self, path: &Path, body: &str) -> Result<(), String>;
}

type Prepare = fn(&dyn ProfileFiles, &Path, &Path) -> Result<(), String>;
struct Adapter {
    provider: ProviderId,
    home: PathBuf,
    prepare: Prepare,
    apply: fn(&mut Command, &Profile) -> Result<(), String>,
}

pub struct NativeProfiles {
    root: PathBuf,
    adapters: Vec<Adapter>,
    files: Arc<dyn ProfileFiles>,
}
impl NativeProfiles {
    pub fn new(
        root: PathBuf,
        claude: PathBuf,
        codex: PathBuf,
        external: PathBuf,
        files: Arc<dyn ProfileFiles>,
    ) -> Self {
        Self {
            root,
            files,
            adapters: vec![
                Adapter {
                    provider: ProviderId::Claude,
                    home: claude,
                    prepare: claude::prepare_profile_at,
                    apply: |command, profile| {
                        claude::account_env(command, profile);
                        Ok(())
                    },
                },
                Adapter {
                    provider: ProviderId::Codex,
                    home: codex,
                    prepare: codex::prepare_profile_at,
                    apply: |command, profile| {
                        codex::account_env(command, profile);
                        Ok(())
                    },
                },
                Adapter {
                    provider: ProviderId::Antigravity,
                    home: external,
                    prepare: |_, _, _| Err(code("err.account.external")),
                    apply: |_, profile| {
                        if profile.managed {
                            Err(code("err.account.external"))
                        } else {
                            Ok(())
                        }
                    },
                },
            ],
        }
    }
    fn adapter(&self, provider: ProviderId) -> Result<&Adapter, String> {
        self.adapters
            .iter()
            .find(|adapter| adapter.provider == provider)
            .ok_or_else(|| code("err.account.external"))
    }
}
impl ProfileBackend for NativeProfiles {
    fn resolve(&self, account: &Account) -> Result<Profile, String> {
        let adapter = self.adapter(account.provider)?;
        let managed = account.id != key(account.provider);
        Ok(Profile {
            id: account.id.clone(),
            provider: account.provider,
            revision: account.revision,
            managed,
            home: if managed {
                self.root.join("accounts").join(&account.id)
            } else {
                adapter.home.clone()
            },
        })
    }
    fn prepare(&self, profile: &Profile) -> Result<(), String> {
        if !profile.managed {
            return Ok(());
        }
        self.files.ensure_private_dir(&profile.home)?;
        let adapter = self.adapter(profile.provider)?;
        (adapter.prepare)(self.files.as_ref(), &adapter.home, &profile.home)
    }
    fn apply(&self, profile: &Profile, command: &mut Command) -> Result<(), String> {
        (self.adapter(profile.provider)?.apply)(command, profile)
    }
}
fn io(cause: impl ToString) -> String {
    prometeu_core::error::with_args("err.io", &[("cause", cause.to_string())])
}

#[cfg(test)]
mod tests;
