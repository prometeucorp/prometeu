//! Default WSL startup over injected discovery and installation dependencies.
use crate::{wsl_command::WslCommands, Target};
use prometeu_core::error::{code, with_args};
use std::{sync::Arc, time::Duration};

pub struct Environment {
    pub home: String,
    pub codex: String,
    pub architecture: String,
}
pub trait WslEnvironment: Send + Sync {
    fn default_environment(&self) -> Result<(String, Environment), String>;
}
pub trait RuntimeInstaller: Send + Sync {
    fn install(&self, distribution: &str, environment: &Environment) -> Result<String, String>;
}
/// Reads the identity owned by an existing runtime root without changing its data.
pub trait RuntimeRoots: Send + Sync {
    fn workdir(&self, distribution: &str, root: &str) -> Result<Option<String>, String>;
}
pub struct Bootstrap {
    pub environment: Arc<dyn WslEnvironment>,
    pub installer: Arc<dyn RuntimeInstaller>,
    pub roots: Arc<dyn RuntimeRoots>,
    pub root: Option<String>,
}
impl Bootstrap {
    pub fn prepare(&self, previous: Option<Target>) -> Result<Target, String> {
        let (distribution, environment) = self.environment.default_environment()?;
        validate_distribution(&distribution)?;
        let previous =
            previous.filter(|target| target.distribution.eq_ignore_ascii_case(&distribution));
        let mut target = Target {
            distribution,
            executable: "/pending".into(),
            root: self
                .root
                .clone()
                .or_else(|| previous.as_ref().map(|t| t.root.clone()))
                .unwrap_or_else(|| {
                    format!("{}/.local/share/prometeu-windows/state", environment.home)
                }),
            workdir: environment.home.clone(),
            codex: environment.codex.clone(),
        };
        target.validate()?;
        // WebView storage can be absent or stale after installation. The WSL root
        // owns the identity; its store still validates and locks it at attachment.
        target.workdir = self
            .roots
            .workdir(&target.distribution, &target.root)?
            .or_else(|| {
                previous
                    .filter(|t| t.root == target.root)
                    .map(|t| t.workdir)
            })
            .unwrap_or(environment.home.clone());
        target.validate()?;
        target.executable = self.installer.install(&target.distribution, &environment)?;
        target.validate()?;
        Ok(target)
    }
}
fn validate_distribution(distribution: &str) -> Result<(), String> {
    if distribution.trim().is_empty()
        || distribution.starts_with('-')
        || distribution.contains(['\0', '\r', '\n'])
    {
        return Err(code("err.windows.distribution"));
    }
    Ok(())
}
pub struct NativeEnvironment(pub Arc<dyn WslCommands>);
const PROBE: &str = r#"set -eu
cli=$(command -v codex || true)
if [ -z "$cli" ] && [ -x "$HOME/.local/bin/codex" ]; then cli="$HOME/.local/bin/codex"; fi
if [ -z "$cli" ]; then printf 'Codex is not installed in this distribution' >&2; exit 1; fi
printf '\036prometeu-bootstrap\000%s\000%s\000%s\000%s\000' "$WSL_DISTRO_NAME" "$HOME" "$cli" "$(uname -m)"
"#;
fn failure(cause: String) -> String {
    with_args("err.windows.setup", &[("cause", cause)])
}
pub struct NativeRoots(pub Arc<dyn WslCommands>);
const READ_ROOT: &str = r#"set -eu
if [ -e "$1/runtime.json" ]; then cat -- "$1/runtime.json"; else printf 'null'; fi
"#;
impl RuntimeRoots for NativeRoots {
    fn workdir(&self, distribution: &str, root: &str) -> Result<Option<String>, String> {
        #[derive(serde::Deserialize)]
        struct Identity {
            v: u32,
            provider: String,
            workdir: String,
        }
        let bytes = self
            .0
            .run(
                &[
                    "--distribution",
                    distribution,
                    "--exec",
                    "/bin/sh",
                    "-c",
                    READ_ROOT,
                    "prometeu-root",
                    root,
                ],
                vec![],
                Duration::from_secs(20),
            )
            .map_err(failure)?;
        let identity: Option<Identity> = serde_json::from_slice(&bytes)
            .map_err(|e| failure(format!("invalid runtime metadata: {e}")))?;
        identity
            .map(|identity| {
                if identity.v != 1 || identity.provider != "codex" {
                    return Err(failure(
                        "runtime root version or provider does not match".into(),
                    ));
                }
                Ok(identity.workdir)
            })
            .transpose()
    }
}
impl WslEnvironment for NativeEnvironment {
    fn default_environment(&self) -> Result<(String, Environment), String> {
        // Omitting --distribution is WSL's own default-selection contract.
        let bytes = self
            .0
            .run(
                &["--exec", "/bin/sh", "-lc", PROBE],
                vec![],
                Duration::from_secs(20),
            )
            .map_err(failure)?;
        let text = String::from_utf8(bytes).map_err(|e| failure(e.to_string()))?;
        let fields: Vec<_> = text
            .rsplit_once("\u{1e}prometeu-bootstrap\0")
            .ok_or_else(|| code("err.windows.environment"))?
            .1
            .split('\0')
            .collect();
        if fields.len() != 5 || !fields[4].is_empty() {
            return Err(code("err.windows.environment"));
        }
        validate_distribution(fields[0])?;
        for path in &fields[1..3] {
            if !path.starts_with('/') || path.contains(['\n', '\r']) {
                return Err(code("err.windows.environment"));
            }
        }
        Ok((
            fields[0].into(),
            Environment {
                home: fields[1].into(),
                codex: fields[2].into(),
                architecture: fields[3].into(),
            },
        ))
    }
}
pub struct RuntimePackage {
    pub digest: String,
    pub bytes: Vec<u8>,
}
pub struct NativeInstaller {
    pub commands: Arc<dyn WslCommands>,
    pub package: Option<RuntimePackage>,
}
const INSTALL: &str = include_str!("install-runtime.sh");
impl RuntimeInstaller for NativeInstaller {
    fn install(&self, distribution: &str, environment: &Environment) -> Result<String, String> {
        validate_distribution(distribution)?;
        if environment.architecture != "x86_64" {
            return Err(code("err.windows.architecture"));
        }
        let package = self
            .package
            .as_ref()
            .ok_or_else(|| code("err.windows.package"))?;
        if package.digest.len() != 64 || !package.digest.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(code("err.windows.package"));
        }
        let base = format!("{}/.local/share/prometeu-windows", environment.home);
        let bytes = self
            .commands
            .run(
                &[
                    "--distribution",
                    distribution,
                    "--exec",
                    "/bin/sh",
                    "-c",
                    INSTALL,
                    "prometeu-install",
                    &base,
                    &package.digest,
                ],
                package.bytes.clone(),
                Duration::from_secs(30),
            )
            .map_err(failure)?;
        let executable = String::from_utf8(bytes).map_err(|e| failure(e.to_string()))?;
        let expected = format!("{base}/runtime/{}/prometeu-runtime", package.digest);
        if executable.trim_end_matches(['\n', '\r']) != expected {
            return Err(code("err.windows.package"));
        }
        Ok(expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Reply(Vec<u8>);
    impl WslCommands for Reply {
        fn run(&self, args: &[&str], _: Vec<u8>, _: Duration) -> Result<Vec<u8>, String> {
            assert_eq!(&args[..4], &["--exec", "/bin/sh", "-lc", PROBE]);
            Ok(self.0.clone())
        }
    }
    #[test]
    fn native_discovery_uses_wsls_default_and_preserves_unicode_paths() {
        let (distribution, environment) = NativeEnvironment(Arc::new(Reply(
            b"Login banner\n\x1eprometeu-bootstrap\0Default Linux\0/home/a ' b\0/home/a ' b/bin/codex\0x86_64\0".into()
        ))).default_environment().unwrap();
        assert_eq!(distribution, "Default Linux");
        assert_eq!(environment.home, "/home/a ' b");
        assert!(NativeEnvironment(Arc::new(Reply(b"invalid".into())))
            .default_environment()
            .is_err());
    }
    struct EnvironmentStub;
    impl WslEnvironment for EnvironmentStub {
        fn default_environment(&self) -> Result<(String, Environment), String> {
            Ok((
                "Default Linux".into(),
                Environment {
                    home: "/home/a".into(),
                    codex: "/bin/codex".into(),
                    architecture: "x86_64".into(),
                },
            ))
        }
    }
    struct InstallStub;
    impl RuntimeInstaller for InstallStub {
        fn install(&self, distribution: &str, _: &Environment) -> Result<String, String> {
            assert_eq!(distribution, "Default Linux");
            Ok("/installed/runtime".into())
        }
    }
    struct RootsStub(Option<String>);
    impl RuntimeRoots for RootsStub {
        fn workdir(&self, _: &str, _: &str) -> Result<Option<String>, String> {
            Ok(self.0.clone())
        }
    }
    #[test]
    fn default_startup_needs_no_project_and_retains_only_matching_distribution_data() {
        let service = Bootstrap {
            environment: Arc::new(EnvironmentStub),
            installer: Arc::new(InstallStub),
            roots: Arc::new(RootsStub(None)),
            root: None,
        };
        let mut target = service.prepare(None).unwrap();
        assert_eq!(target.workdir, "/home/a");
        assert_eq!(target.root, "/home/a/.local/share/prometeu-windows/state");
        target.root = "/previous/state".into();
        target.workdir = "/previous/project".into();
        target.distribution = "default linux".into();
        let retained = service.prepare(Some(target.clone())).unwrap();
        assert_eq!(retained.root, target.root);
        assert_eq!(retained.workdir, target.workdir);
        target.distribution = "Another Linux".into();
        let switched = service.prepare(Some(target)).unwrap();
        assert_eq!(switched.distribution, "Default Linux");
        assert_eq!(switched.workdir, "/home/a");
        assert_eq!(switched.root, "/home/a/.local/share/prometeu-windows/state");
    }
    #[test]
    fn persisted_root_identity_survives_missing_or_stale_webview_storage() {
        let mut service = Bootstrap {
            environment: Arc::new(EnvironmentStub),
            installer: Arc::new(InstallStub),
            roots: Arc::new(RootsStub(Some("/".into()))),
            root: None,
        };
        let restored = service.prepare(None).unwrap();
        assert_eq!(restored.workdir, "/");
        let mut cached = restored.clone();
        cached.workdir = "/home/a".into();
        assert_eq!(service.prepare(Some(cached)).unwrap(), restored);

        service.root = Some("/isolated/new-root".into());
        service.roots = Arc::new(RootsStub(None));
        let isolated = service.prepare(Some(restored)).unwrap();
        assert_eq!(isolated.root, "/isolated/new-root");
        assert_eq!(isolated.workdir, "/home/a");
    }
    #[test]
    fn invalid_root_identity_never_reaches_installation() {
        struct NoInstall;
        impl RuntimeInstaller for NoInstall {
            fn install(&self, _: &str, _: &Environment) -> Result<String, String> {
                panic!("invalid roots must fail before installation")
            }
        }
        let service = Bootstrap {
            environment: Arc::new(EnvironmentStub),
            installer: Arc::new(NoInstall),
            roots: Arc::new(RootsStub(Some("relative/path".into()))),
            root: None,
        };
        assert!(service.prepare(None).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn native_root_discovery_is_read_only_and_rejects_incompatible_metadata() {
        struct Local;
        impl WslCommands for Local {
            fn run(&self, args: &[&str], _: Vec<u8>, _: Duration) -> Result<Vec<u8>, String> {
                let output = std::process::Command::new(args[3])
                    .args(&args[4..])
                    .output()
                    .unwrap();
                match output.status.success() {
                    true => Ok(output.stdout),
                    false => Err(String::from_utf8_lossy(&output.stderr).into()),
                }
            }
        }
        let root = std::env::temp_dir().join(format!(
            "prometeu-root-{}-{} ' ação",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let service = NativeRoots(Arc::new(Local));
        let path = root.to_str().unwrap();
        assert_eq!(service.workdir("Default Linux", path).unwrap(), None);
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        let metadata = root.join("runtime.json");
        let saved = br#"{"v":1,"workdir":"/","provider":"codex","provider_session":"retained"}"#;
        std::fs::write(&metadata, saved).unwrap();
        assert_eq!(
            service.workdir("Default Linux", path).unwrap(),
            Some("/".into())
        );
        assert_eq!(std::fs::read(&metadata).unwrap(), saved);
        for invalid in [
            r#"{"v":2,"workdir":"/","provider":"codex"}"#,
            r#"{"v":1,"workdir":"/","provider":"other"}"#,
            "{broken",
            "",
        ] {
            std::fs::write(&metadata, invalid).unwrap();
            assert!(service.workdir("Default Linux", path).is_err());
            assert_eq!(std::fs::read_to_string(&metadata).unwrap(), invalid);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn native_install_checks_content_and_preserves_an_existing_binary_on_failure() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        struct Local;
        impl WslCommands for Local {
            fn run(&self, args: &[&str], input: Vec<u8>, _: Duration) -> Result<Vec<u8>, String> {
                let mut child = Command::new(args[3])
                    .args(&args[4..])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&input).unwrap();
                let output = child.wait_with_output().unwrap();
                if !output.status.success() {
                    return Err(String::from_utf8_lossy(&output.stderr).into());
                }
                Ok(output.stdout)
            }
        }
        let home = std::env::temp_dir().join(format!(
            "prometeu-install-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).unwrap();
        let bytes = b"#!/bin/sh\nexit 0\n".to_vec();
        let input = home.join("input");
        std::fs::write(&input, &bytes).unwrap();
        let hash = Command::new("sha256sum").arg(&input).output().unwrap();
        let digest = String::from_utf8(hash.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned();
        let environment = Environment {
            home: home.to_str().unwrap().into(),
            codex: "/codex".into(),
            architecture: "x86_64".into(),
        };
        let mut installer = NativeInstaller {
            commands: Arc::new(Local),
            package: Some(RuntimePackage {
                digest,
                bytes: bytes.clone(),
            }),
        };
        let executable = installer.install("Ubuntu", &environment).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), bytes);
        installer.package.as_mut().unwrap().bytes = b"corrupt".to_vec();
        assert!(installer.install("Ubuntu", &environment).is_err());
        assert_eq!(std::fs::read(&executable).unwrap(), bytes);
        assert_eq!(
            std::fs::read_dir(std::path::Path::new(&executable).parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
