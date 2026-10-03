//! Import native directory selections without teaching application screens about WSL paths.
use crate::wsl_command::{NativeWslCommands, WslCommands};
use crate::Target;
use prometeu_core::error::{code, with_args};
use std::{process::Command, sync::Arc, time::Duration};

pub trait ApplicationPaths: Send + Sync {
    fn linux(&self, target: &Target, path: &str) -> Result<String, String>;
    fn windows(&self, target: &Target, path: &str) -> Result<String, String>;
}
pub trait PathQuery: Send + Sync {
    fn translate(&self, distribution: &str, path: &str) -> Result<String, String>;
    fn windows(&self, distribution: &str, path: &str) -> Result<String, String>;
}
pub struct WslPaths(pub Arc<dyn PathQuery>);
impl ApplicationPaths for WslPaths {
    fn windows(&self, target: &Target, path: &str) -> Result<String, String> {
        target.validate()?;
        // Explorer must never reinterpret a Linux name as a separator or another entry.
        if !path.starts_with('/')
            || path.split('/').any(|part| {
                part.ends_with(['.', ' '])
                    || part
                        .chars()
                        .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
            })
        {
            return Err(code("err.windows.path"));
        }
        let translated = self.0.windows(&target.distribution, path)?;
        if translated.starts_with('/') {
            return Err(code("err.windows.path"));
        }
        // Reuse import admission and the distribution's own mount conversion in reverse.
        if self.linux(target, &translated)?.trim_end_matches('/') != path.trim_end_matches('/') {
            return Err(code("err.windows.path"));
        }
        Ok(translated)
    }
    fn linux(&self, target: &Target, path: &str) -> Result<String, String> {
        target.validate()?;
        if path.is_empty() || path.contains(['\0', '\n', '\r']) {
            return Err(code("err.windows.path"));
        }
        if path.starts_with('/') && !path.starts_with("//") {
            return Ok(path.into());
        }
        let normalized = path.replace('\\', "/");
        if let Some(unc) = normalized.strip_prefix("//") {
            let mut parts = unc.splitn(3, '/');
            let server = parts.next().unwrap_or_default();
            let distribution = parts.next().unwrap_or_default();
            if !["wsl.localhost", "wsl$"]
                .iter()
                .any(|s| server.eq_ignore_ascii_case(s))
            {
                return Err(code("err.windows.path"));
            }
            if !distribution.eq_ignore_ascii_case(&target.distribution) {
                return Err(code("err.windows.otherDistribution"));
            }
            let rel = parts.next().unwrap_or_default();
            if rel.split('/').any(|p| matches!(p, "." | "..")) {
                return Err(code("err.windows.path"));
            }
            return Ok(format!("/{rel}"));
        }
        let bytes = normalized.as_bytes();
        if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":/" {
            return Err(code("err.windows.path"));
        }
        let translated = self.0.translate(&target.distribution, path)?;
        if !translated.starts_with('/') || translated.contains(['\0', '\n', '\r']) {
            return Err(code("err.windows.path"));
        }
        Ok(translated)
    }
}
/// A fixed WSL executable and argument array, with bounded output and child lifetime.
pub struct NativePathQuery;
impl NativePathQuery {
    pub fn command(distribution: &str, path: &str) -> Command {
        NativeWslCommands::command(&[
            "--distribution",
            distribution,
            "--exec",
            "wslpath",
            "-u",
            path,
        ])
    }
    fn convert(distribution: &str, direction: &str, path: &str) -> Result<String, String> {
        let failure = |cause: String| with_args("err.windows.pathConversion", &[("cause", cause)]);
        let output = NativeWslCommands
            .run(
                &[
                    "--distribution",
                    distribution,
                    "--exec",
                    "wslpath",
                    direction,
                    path,
                ],
                vec![],
                Duration::from_secs(10),
            )
            .map_err(failure)?;
        String::from_utf8(output)
            .map(|s| s.trim_end_matches(['\r', '\n']).into())
            .map_err(|e| failure(e.to_string()))
    }
}
impl PathQuery for NativePathQuery {
    fn translate(&self, distribution: &str, path: &str) -> Result<String, String> {
        Self::convert(distribution, "-u", path)
    }
    fn windows(&self, distribution: &str, path: &str) -> Result<String, String> {
        Self::convert(distribution, "-w", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Convert;
    impl PathQuery for Convert {
        fn windows(&self, distribution: &str, path: &str) -> Result<String, String> {
            assert_eq!(distribution, "Ubuntu-24.04");
            Ok(format!(
                "\\\\wsl.localhost\\{distribution}{}",
                path.replace('/', "\\")
            ))
        }
        fn translate(&self, distribution: &str, path: &str) -> Result<String, String> {
            assert_eq!(distribution, "Ubuntu-24.04");
            assert_eq!(path, r"D:\source ' ação\project");
            Ok("/custom-drives/d/source ' ação/project".into())
        }
    }
    fn target() -> Target {
        Target {
            distribution: "Ubuntu-24.04".into(),
            executable: "/runtime".into(),
            root: "/root".into(),
            workdir: "/project".into(),
            codex: "/codex".into(),
        }
    }
    #[test]
    fn imported_paths_preserve_selected_distribution_and_delegate_drive_mounts() {
        let paths = WslPaths(Arc::new(Convert));
        for path in [
            r"\\wsl.localhost\Ubuntu-24.04\home\source ' ação",
            r"\\wsl$\ubuntu-24.04\home\source ' ação",
        ] {
            assert_eq!(paths.linux(&target(), path).unwrap(), "/home/source ' ação");
        }
        assert_eq!(paths.linux(&target(), r"/home/a\b").unwrap(), r"/home/a\b");
        assert_eq!(
            paths.linux(&target(), r"D:\source ' ação\project").unwrap(),
            "/custom-drives/d/source ' ação/project"
        );
        for path in [
            r"\\wsl.localhost\Debian\home\project",
            r"\\server\share",
            "relative",
            "C:relative",
            "C:/bad\npath",
            r"\\wsl$\Ubuntu-24.04\..\Debian",
        ] {
            assert!(paths.linux(&target(), path).is_err(), "{path}");
        }
        let command = NativePathQuery::command("Ubuntu-24.04", r"C:\a ' $(echo nope)");
        assert_eq!(command.get_args().last().unwrap(), r"C:\a ' $(echo nope)");
        assert_eq!(command.get_program(), "wsl.exe");
    }
    #[test]
    fn revealed_paths_preserve_unicode_and_reject_names_explorer_cannot_represent() {
        let paths = WslPaths(Arc::new(Convert));
        assert_eq!(
            paths
                .windows(&target(), "/home/source ' ação/file,one.txt")
                .unwrap(),
            r"\\wsl.localhost\Ubuntu-24.04\home\source ' ação\file,one.txt"
        );
        for path in [
            "relative",
            "/home/a\\b",
            "/home/a:b",
            "/home/a\n",
            "/home/trailing.",
            "/home/../other",
        ] {
            assert!(paths.windows(&target(), path).is_err(), "{path:?}");
        }
    }
}
