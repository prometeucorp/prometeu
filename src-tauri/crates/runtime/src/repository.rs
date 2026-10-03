//! WSL Git queries use the same reference-selection policy as desktop.
use prometeu_core::{
    command::{CommandPolicy, CommandRunner, OutputPolicy},
    repository::{self, Branches, RepositoryReferences},
};
use std::{cell::RefCell, path::Path, process::Command, sync::Arc, time::Duration};

pub struct GitReferences(pub Arc<dyn CommandRunner<Command>>);
impl GitReferences {
    fn query(&self, path: &str, args: &[&str]) -> Result<String, String> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0");
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            command.env_remove(key);
        }
        let output = self
            .0
            .run(
                &mut command,
                &[],
                CommandPolicy {
                    timeout: Duration::from_secs(10),
                    stdout: OutputPolicy::Capture { limit: 256 * 1024 },
                    stderr: OutputPolicy::Capture { limit: 64 * 1024 },
                },
            )
            .map_err(|error| format!("{error:?}"))?;
        // A missing ref or non-Git directory has no branch, as on desktop.
        match output.success {
            true => String::from_utf8(output.stdout).map_err(|e| e.to_string()),
            false => Ok(String::new()),
        }
    }
}
impl RepositoryReferences for GitReferences {
    fn head(&self, path: &str) -> Result<Option<String>, String> {
        self.query(path, &["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|output| repository::head_name(&output))
    }
    fn branches(&self, path: &str) -> Result<Branches, String> {
        let error = RefCell::new(None);
        let branches = repository::branches(
            |args| {
                self.query(path, args).unwrap_or_else(|cause| {
                    *error.borrow_mut() = Some(cause);
                    String::new()
                })
            },
            Path::new(path).join(".git").exists(),
        );
        match error.into_inner() {
            Some(error) => Err(error),
            None => Ok(branches),
        }
    }
}
