//! Native Codex cache installation, replaceable independently from package preparation.
use crate::packages::{InstalledPlugin, PackageInstaller};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub struct CodexInstaller {
    pub executable: PathBuf,
    pub marketplace: String,
}
impl CodexInstaller {
    pub fn command(&self, home: &Path) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .env("CODEX_HOME", home)
            .args(["--enable", "plugins", "--enable", "hooks"]);
        command
    }
}
impl PackageInstaller for CodexInstaller {
    fn installed(&self, home: &Path) -> Result<HashMap<String, InstalledPlugin>, String> {
        let output = self
            .command(home)
            .args([
                "plugin",
                "list",
                "--marketplace",
                &self.marketplace,
                "--available",
                "--json",
            ])
            .output()
            .map_err(|error| {
                prometeu_core::error::with_args(
                    "err.plugin.codex.cli",
                    &[("cause", error.to_string())],
                )
            })?;
        if !output.status.success() {
            return Err(prometeu_core::error::with_args(
                "err.plugin.codex.cli",
                &[("cause", last_line(&String::from_utf8_lossy(&output.stderr)))],
            ));
        }
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            prometeu_core::error::with_args("err.plugin.codex.cli", &[("cause", error.to_string())])
        })?;
        Ok(value["installed"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                Some((
                    entry["pluginId"].as_str()?.to_string(),
                    InstalledPlugin {
                        version: entry["version"].as_str().unwrap_or_default().to_string(),
                    },
                ))
            })
            .collect())
    }

    fn install(&self, home: &Path, canonical: &str) -> Result<(), String> {
        let output = self
            .command(home)
            .args(["plugin", "add", canonical, "--json"])
            .output()
            .map_err(|error| {
                prometeu_core::error::with_args(
                    "err.plugin.codex.cli",
                    &[("cause", error.to_string())],
                )
            })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(prometeu_core::error::with_args(
                "err.plugin.codex.cli",
                &[("cause", last_line(&String::from_utf8_lossy(&output.stderr)))],
            ))
        }
    }

    fn remove(&self, home: &Path, canonical: &str) {
        let _ = self
            .command(home)
            .args(["plugin", "remove", canonical, "--json"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
fn last_line(text: &str) -> String {
    text.trim()
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn native_installer_preserves_home_arguments_results_and_failure_details() {
        let root =
            std::env::temp_dir().join(format!("prometeu-installer-{}", uuid::Uuid::new_v4()));
        let home = root.join("derived home");
        std::fs::create_dir_all(&home).unwrap();
        let executable = root.join("codex-fixture");
        std::fs::write(&executable, r#"#!/bin/sh
printf '%s\n' "$@" > "$CODEX_HOME/arguments"
case "$6" in
list) printf '{"installed":[{"pluginId":"review@prometeu-dev","version":"1.2.3"},{"unrelated":true}]}' ;;
add) printf 'first detail\nlast detail\n' >&2; exit 9 ;;
remove) printf removed > "$CODEX_HOME/removed" ;;
esac
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let installer = CodexInstaller {
            executable,
            marketplace: "prometeu-dev".into(),
        };
        let installed = installer.installed(&home).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed["review@prometeu-dev"].version, "1.2.3");
        assert_eq!(std::fs::read_to_string(home.join("arguments")).unwrap(), "--enable\nplugins\n--enable\nhooks\nplugin\nlist\n--marketplace\nprometeu-dev\n--available\n--json\n");
        assert_eq!(
            installer.install(&home, "review@prometeu-dev"),
            Err(prometeu_core::error::with_args(
                "err.plugin.codex.cli",
                &[("cause", "last detail".into())]
            ))
        );
        assert_eq!(
            std::fs::read_to_string(home.join("arguments")).unwrap(),
            "--enable\nplugins\n--enable\nhooks\nplugin\nadd\nreview@prometeu-dev\n--json\n"
        );
        installer.remove(&home, "review@prometeu-dev");
        assert_eq!(
            std::fs::read_to_string(home.join("removed")).unwrap(),
            "removed"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
