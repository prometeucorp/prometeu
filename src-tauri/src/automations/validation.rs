//! Explicit validation programs execute only inside an OS-enforced sandbox.
//! Repository code is untrusted; the command allowlist is not the isolation boundary.
use prometeu_core::command::{CommandError, CommandPolicy, CommandRunner, OutputPolicy};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const LIMIT: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_COMMANDS: usize = 8;
const MAX_FILES: usize = 100_000;
const SAFE_PATH: &str = "/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin:/opt/local/bin";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidationCommand {
    pub executable: String,
    pub args: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ValidationCommandResult {
    pub executable: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ValidationResult {
    pub passed: bool,
    pub commands: Vec<ValidationCommandResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Linux,
    Mac,
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("prometeu-validation-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "automation_validation_temporary_directory")?;
        fs::create_dir(path.join("home"))
            .map_err(|_| "automation_validation_temporary_directory")?;
        Ok(Self(path.canonicalize().map_err(|_| {
            "automation_validation_temporary_directory"
        })?))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn validate_commands(commands: &[ValidationCommand]) -> Result<(), String> {
    if commands.is_empty() || commands.len() > MAX_COMMANDS {
        return Err("automation_validation_commands_required: Configure between one and eight validation commands".into());
    }
    for command in commands {
        if !matches!(
            command.executable.as_str(),
            "npm"
                | "pnpm"
                | "yarn"
                | "node"
                | "cargo"
                | "rustc"
                | "go"
                | "python3"
                | "pytest"
                | "make"
                | "cmake"
                | "ctest"
        ) {
            return Err("automation_validation_executable_denied: Select a supported validation executable by name".into());
        }
        if command.args.is_empty()
            || command.args.len() > 128
            || command
                .args
                .iter()
                .any(|arg| arg.contains('\0') || arg.len() > 16_384)
            || command.args.iter().map(String::len).sum::<usize>() > 64 * 1024
        {
            return Err("automation_validation_arguments_invalid".into());
        }
    }
    Ok(())
}

/// A shared two-minute deadline covers the probe and all configured commands.
pub(super) fn run(
    runner: &dyn CommandRunner<Command>,
    workspace: &Path,
    commands: &[ValidationCommand],
    dependency_source: Option<&Path>,
) -> Result<ValidationResult, String> {
    validate_commands(commands)?;
    let platform = if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "macos") {
        Platform::Mac
    } else {
        return Err("automation_validation_platform_unavailable".into());
    };
    let root = workspace
        .canonicalize()
        .map_err(|_| "automation_validation_workspace_missing")?;
    if !root.is_dir() || root.parent().is_none() {
        return Err("automation_validation_workspace_invalid".into());
    }
    let sandbox = match platform {
        Platform::Linux => PathBuf::from("/usr/bin/bwrap"),
        Platform::Mac => PathBuf::from("/usr/bin/sandbox-exec"),
    };
    if !sandbox.is_file() {
        return Err(unavailable(platform));
    }
    let programs: Vec<_> = commands
        .iter()
        .map(|command| resolve(&command.executable, &root))
        .collect::<Result<_, _>>()?;
    let mut protected = inspect(&root)?;
    let scratch = Scratch::new()?;
    let dependency = dependency(&root, dependency_source, commands, platform)?;
    if let Some(dependency) = &dependency {
        for item in inspect(&dependency.source)? {
            protected.push(Protected {
                relative: Path::new("node_modules").join(item.relative),
                directory: item.directory,
            });
        }
    }
    let mut filter = network_filter(platform, &scratch.0)?;
    let started = Instant::now();
    let probe_program = PathBuf::from("/usr/bin/true");
    let mut probe = sandbox_command(
        platform,
        &sandbox,
        &root,
        &scratch.0,
        &probe_program,
        &[],
        &protected,
        &[],
        filter.as_ref(),
        dependency.as_ref(),
    )?;
    let probe_result = runner
        .run(&mut probe, &[], policy(Duration::from_secs(10)))
        .map_err(|_| unavailable(platform))?;
    if !probe_result.success {
        return Err(unavailable(platform));
    }
    let mut results = Vec::new();
    for (spec, program) in commands.iter().zip(programs) {
        if let Some(filter) = filter.as_mut() {
            filter
                .rewind()
                .map_err(|_| "automation_validation_filter")?;
        }
        let remaining = TIMEOUT
            .checked_sub(started.elapsed())
            .ok_or("automation_validation_timeout")?;
        let mut command = sandbox_command(
            platform,
            &sandbox,
            &root,
            &scratch.0,
            &program.executable,
            &spec.args,
            &protected,
            &program.roots,
            filter.as_ref(),
            dependency.as_ref(),
        )?;
        let output =
            runner
                .run(&mut command, &[], policy(remaining))
                .map_err(|error| match error {
                    CommandError::Timeout => "automation_validation_timeout",
                    CommandError::OutputLimit => "automation_validation_output_limit",
                    CommandError::Unavailable => "automation_validation_sandbox_unavailable",
                    _ => "automation_validation_process_failed",
                })?;
        // Recheck limits independently so injected/native runners share the same contract.
        if output.stdout.len() > LIMIT || output.stderr.len() > LIMIT {
            return Err("automation_validation_output_limit".into());
        }
        let success = output.success;
        results.push(ValidationCommandResult {
            executable: spec.executable.clone(),
            success,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
        if !success {
            break;
        }
    }
    if let Some(dependency) = &dependency {
        dependency.verify(&root)?;
    }
    Ok(ValidationResult {
        passed: results.len() == commands.len() && results.iter().all(|result| result.success),
        commands: results,
    })
}

fn policy(timeout: Duration) -> CommandPolicy {
    CommandPolicy {
        timeout,
        stdout: OutputPolicy::Capture { limit: LIMIT },
        stderr: OutputPolicy::Capture { limit: LIMIT },
    }
}

fn unavailable(platform: Platform) -> String {
    match platform {
        Platform::Linux=>"automation_validation_sandbox_unavailable: Install bubblewrap at /usr/bin/bwrap and enable unprivileged user namespaces; validation never runs without isolation",
        Platform::Mac=>"automation_validation_sandbox_unavailable: macOS sandbox-exec could not start the restricted validation; validation never runs without isolation",
    }.into()
}

struct Program {
    executable: PathBuf,
    roots: Vec<PathBuf>,
}

struct Dependency {
    source: PathBuf,
    source_project: PathBuf,
    target: PathBuf,
    platform: Platform,
    identity: (u64, u64),
    manifests: Vec<(String, String)>,
}

impl Dependency {
    fn verify(&self, workspace: &Path) -> Result<(), String> {
        for (name, expected) in &self.manifests {
            if manifest_hash(&workspace.join(name))? != *expected
                || manifest_hash(&self.source_project.join(name))? != *expected
            {
                return Err("automation_validation_dependencies_changed: Package files changed during validation".into());
            }
        }
        Ok(())
    }
}
impl Drop for Dependency {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.target) else {
            return;
        };
        if (metadata.dev(), metadata.ino()) != self.identity {
            return;
        }
        match self.platform {
            Platform::Linux => {
                let _ = fs::remove_dir(&self.target);
            }
            Platform::Mac => {
                if fs::read_link(&self.target).ok().as_ref() == Some(&self.source) {
                    let _ = fs::remove_file(&self.target);
                }
            }
        }
    }
}

fn manifest_hash(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path).map_err(|_|"automation_validation_dependencies_unavailable: Matching package.json and npm lockfile are required")?;
    if !file
        .metadata()
        .map_err(|_| "automation_validation_dependencies_unavailable")?
        .is_file()
    {
        return Err("automation_validation_dependencies_unavailable".into());
    }
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "automation_validation_dependencies_unavailable")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("automation_validation_manifest_limit".into());
    }
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn dependency(
    root: &Path,
    source: Option<&Path>,
    commands: &[ValidationCommand],
    platform: Platform,
) -> Result<Option<Dependency>, String> {
    if !commands.iter().any(|command| {
        matches!(
            command.executable.as_str(),
            "npm" | "pnpm" | "yarn" | "node"
        )
    }) {
        return Ok(None);
    }
    let target = root.join("node_modules");
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.is_dir() {
            return Ok(None);
        }
        return Err("automation_validation_dependencies_unavailable: Existing node_modules must be a real directory in the worktree".into());
    }
    // Standalone Node scripts without a package manifest need no dependency mount.
    if !root.join("package.json").exists() {
        return Ok(None);
    }
    let source=source.ok_or("automation_validation_dependencies_unavailable: Install project dependencies in the source checkout first")?.canonicalize().map_err(|_|"automation_validation_dependencies_unavailable")?;
    let modules = source.join("node_modules");
    if !fs::symlink_metadata(&modules).is_ok_and(|metadata| metadata.is_dir()) {
        return Err("automation_validation_dependencies_unavailable: Install npm dependencies in the source checkout; linked external stores are unsupported".into());
    }
    let lock = if root.join("npm-shrinkwrap.json").is_file() {
        "npm-shrinkwrap.json"
    } else {
        "package-lock.json"
    };
    let mut manifests = Vec::new();
    for name in ["package.json", lock] {
        let expected = manifest_hash(&root.join(name))?;
        if manifest_hash(&source.join(name))? != expected {
            return Err("automation_validation_dependencies_mismatch: Source and worktree package files must match before reusing installed dependencies".into());
        }
        manifests.push((name.into(), expected));
    }
    fn confined_links(
        root: &Path,
        directory: &Path,
        count: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("automation_validation_tree_limit".into());
        }
        for entry in
            fs::read_dir(directory).map_err(|_| "automation_validation_dependencies_unavailable")?
        {
            let entry = entry.map_err(|_| "automation_validation_dependencies_unavailable")?;
            *count += 1;
            if *count > MAX_FILES {
                return Err("automation_validation_tree_limit".into());
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| "automation_validation_dependencies_unavailable")?;
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(&path)
                    .map_err(|_| "automation_validation_dependencies_unavailable")?;
                if target.is_absolute()
                    || !path.canonicalize().is_ok_and(|path| path.starts_with(root))
                {
                    return Err("automation_validation_dependency_link_denied: Dependencies must not link to external stores".into());
                }
            } else if metadata.is_dir() {
                confined_links(root, &path, count, depth + 1)?;
            }
        }
        Ok(())
    }
    confined_links(&modules, &modules, &mut 0, 0)?;
    match platform {
        Platform::Linux => fs::create_dir(&target),
        Platform::Mac => std::os::unix::fs::symlink(&modules, &target),
    }
    .map_err(|_| "automation_validation_dependencies_unavailable")?;
    let metadata = fs::symlink_metadata(&target)
        .map_err(|_| "automation_validation_dependencies_unavailable")?;
    Ok(Some(Dependency {
        source: modules,
        source_project: source,
        target,
        platform,
        identity: (metadata.dev(), metadata.ino()),
        manifests,
    }))
}

fn resolve(name: &str, workspace: &Path) -> Result<Program, String> {
    // PATH is only a source of candidates. Resolve symlinks and accept known installation
    // roots, never the workspace, a relative directory, a shell shim, or an arbitrary PATH entry.
    let mut directories: Vec<PathBuf> =
        std::env::split_paths(std::env::var_os("PATH").as_deref().unwrap_or_default()).collect();
    directories.extend(SAFE_PATH.split(':').map(PathBuf::from));
    for directory in directories {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(name);
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if canonical.starts_with(workspace) {
            continue;
        }
        let Some(root) = installation_root(&canonical) else {
            continue;
        };
        let Ok(metadata) = canonical.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
            continue;
        }
        // rustup's multiplexer depends on user configuration. Resolve an installed
        // actual toolchain instead of handing its proxy access to the user's home.
        if canonical.file_name().is_some_and(|file| file == "rustup") {
            continue;
        }
        return Ok(Program {
            executable: canonical,
            roots: root.into_iter().collect(),
        });
    }
    if matches!(name, "cargo" | "rustc") {
        if let Some(home) = dirs::home_dir() {
            let toolchains = home.join(".rustup/toolchains");
            if let Ok(entries) = fs::read_dir(&toolchains) {
                let mut candidates: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
                candidates.sort_by_key(|path| {
                    (
                        !path
                            .file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with("stable-")),
                        path.clone(),
                    )
                });
                for root in candidates {
                    let Ok(executable) = root.join("bin").join(name).canonicalize() else {
                        continue;
                    };
                    if executable.starts_with(&root)
                        && executable.is_file()
                        && !executable.starts_with(workspace)
                    {
                        return Ok(Program {
                            executable,
                            roots: vec![root],
                        });
                    }
                }
            }
        }
    }
    Err(format!("automation_validation_executable_unavailable: Install {name} in a supported system or versioned toolchain directory"))
}

fn installation_root(path: &Path) -> Option<Option<PathBuf>> {
    if ["/usr", "/bin", "/sbin", "/lib", "/lib64"]
        .iter()
        .any(|root| path.starts_with(root))
    {
        return Some(None);
    }
    for root in [
        "/opt/homebrew",
        "/opt/local",
        "/Library/Developer/CommandLineTools",
        "/Applications/Xcode.app/Contents/Developer",
    ] {
        if path.starts_with(root) {
            return Some(Some(PathBuf::from(root)));
        }
    }
    let home = dirs::home_dir()?;
    for prefix in [
        ".nvm/versions/node",
        ".pyenv/versions",
        ".rustup/toolchains",
    ] {
        let base = home.join(prefix);
        if let Ok(relative) = path.strip_prefix(&base) {
            let version = relative.components().next()?;
            let root = base.join(version.as_os_str());
            if path.starts_with(root.join("bin")) || path.starts_with(root.join("lib")) {
                return Some(Some(root));
            }
        }
    }
    None
}

#[derive(Debug)]
struct Protected {
    relative: PathBuf,
    directory: bool,
}

fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with(".env")
        || matches!(
            name.as_str(),
            ".git"
                | ".ssh"
                | ".aws"
                | ".claude"
                | ".codex"
                | ".agents"
                | ".prometeu"
                | ".npmrc"
                | ".pypirc"
                | ".mcp.json"
        )
}

fn inspect(root: &Path) -> Result<Vec<Protected>, String> {
    fn walk(
        root: &Path,
        directory: &Path,
        depth: usize,
        count: &mut usize,
        protected: &mut Vec<Protected>,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("automation_validation_tree_limit".into());
        }
        for entry in fs::read_dir(directory).map_err(|_| "automation_validation_tree_unreadable")? {
            let entry = entry.map_err(|_| "automation_validation_tree_unreadable")?;
            *count += 1;
            if *count > MAX_FILES {
                return Err("automation_validation_tree_limit".into());
            }
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| "automation_validation_tree_unreadable")?;
            if secret_name(&entry.file_name().to_string_lossy()) {
                protected.push(Protected {
                    relative: path
                        .strip_prefix(root)
                        .map_err(|_| "automation_validation_scope")?
                        .to_owned(),
                    directory: metadata.is_dir(),
                });
                continue;
            }
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                walk(root, &path, depth + 1, count, protected)?;
            } else if !metadata.is_file() || metadata.nlink() != 1 {
                return Err("automation_validation_special_file_denied: Remove sockets, devices, FIFOs and externally shared hard links from the validation worktree".into());
            }
        }
        Ok(())
    }
    let mut protected = Vec::new();
    walk(root, root, 0, &mut 0, &mut protected)?;
    Ok(protected)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep every independently resolved authority explicit at the audited sandbox boundary"
)]
fn sandbox_command(
    platform: Platform,
    sandbox: &Path,
    root: &Path,
    scratch: &Path,
    executable: &Path,
    args: &[String],
    protected: &[Protected],
    toolchains: &[PathBuf],
    filter: Option<&File>,
    dependency: Option<&Dependency>,
) -> Result<Command, String> {
    let mut command = Command::new(sandbox);
    command.env_clear();
    let mut path = Vec::new();
    for toolchain in toolchains {
        path.push(toolchain.join("bin").to_string_lossy().into_owned());
    }
    path.push(SAFE_PATH.into());
    let path = path.join(":");
    match platform {
        Platform::Linux => {
            command.args([
                "--unshare-all",
                "--unshare-user",
                "--new-session",
                "--die-with-parent",
                "--disable-userns",
                "--cap-drop",
                "ALL",
                "--clearenv",
            ]);
            for directory in ["/usr", "/bin", "/sbin", "/lib", "/lib64"] {
                if Path::new(directory).exists() {
                    command.arg("--ro-bind").arg(directory).arg(directory);
                }
            }
            for toolchain in toolchains {
                command.arg("--ro-bind").arg(toolchain).arg(toolchain);
            }
            command.args([
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--tmpfs",
                "/tmp",
                "--dir",
                "/tmp/home",
            ]);
            command.arg("--bind").arg(root).arg("/workspace");
            if let Some(dependency) = dependency {
                command
                    .arg("--ro-bind")
                    .arg(&dependency.source)
                    .arg("/workspace/node_modules");
            }
            for item in protected {
                let target = Path::new("/workspace").join(&item.relative);
                if item.directory {
                    command
                        .arg("--tmpfs")
                        .arg(&target)
                        .arg("--remount-ro")
                        .arg(&target);
                } else {
                    command.arg("--ro-bind").arg("/dev/null").arg(&target);
                }
            }
            #[cfg(target_os = "linux")]
            {
                use std::os::fd::AsRawFd;
                let filter = filter.ok_or("automation_validation_network_filter_missing")?;
                command.arg("--seccomp").arg(filter.as_raw_fd().to_string());
            }
            #[cfg(not(target_os = "linux"))]
            let _ = filter;
            for (key, value) in [
                ("PATH", path.as_str()),
                ("HOME", "/tmp/home"),
                ("TMPDIR", "/tmp"),
                ("CARGO_HOME", "/tmp/cargo"),
                ("GOCACHE", "/tmp/go-cache"),
                ("GOPATH", "/tmp/go"),
                ("npm_config_cache", "/tmp/npm"),
                ("CI", "1"),
                ("LANG", "C.UTF-8"),
            ] {
                command.arg("--setenv").arg(key).arg(value);
            }
            command
                .args(["--chdir", "/workspace", "--"])
                .arg(executable)
                .args(args);
            command.current_dir(scratch);
        }
        Platform::Mac => {
            command
                .arg("-p")
                .arg(mac_profile(
                    root, scratch, protected, toolchains, dependency,
                )?)
                .arg(executable)
                .args(args);
            command.current_dir(root);
            command
                .env("PATH", path)
                .env("HOME", scratch.join("home"))
                .env("TMPDIR", scratch)
                .env("CARGO_HOME", scratch.join("cargo"))
                .env("GOCACHE", scratch.join("go-cache"))
                .env("GOPATH", scratch.join("go"))
                .env("npm_config_cache", scratch.join("npm"))
                .env("CI", "1")
                .env("LANG", "en_US.UTF-8");
        }
    }
    Ok(command)
}

fn quote(path: &Path) -> Result<String, String> {
    let text = path.to_str().ok_or("automation_validation_path_invalid")?;
    if text.chars().any(char::is_control) {
        return Err("automation_validation_path_invalid".into());
    }
    serde_json::to_string(text).map_err(|_| "automation_validation_path_invalid".into())
}

fn mac_profile(
    root: &Path,
    scratch: &Path,
    protected: &[Protected],
    toolchains: &[PathBuf],
    dependency: Option<&Dependency>,
) -> Result<String, String> {
    let mut profile=String::from("(version 1)\n(deny default)\n(allow process-exec)\n(allow process-fork)\n(allow process-info* (target same-sandbox))\n(allow signal (target same-sandbox))\n(allow sysctl-read)\n");
    let mut read: BTreeSet<PathBuf> = [
        "/System",
        "/usr",
        "/bin",
        "/sbin",
        "/Library/Developer/CommandLineTools",
        "/Applications/Xcode.app/Contents/Developer",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    read.extend(toolchains.iter().cloned());
    read.insert(root.to_owned());
    read.insert(scratch.to_owned());
    if let Some(dependency) = dependency {
        read.insert(dependency.source.clone());
    }
    for path in read {
        profile.push_str(&format!("(allow file-read* (subpath {}))\n", quote(&path)?));
    }
    for path in [root, scratch] {
        profile.push_str(&format!("(allow file-write* (subpath {}))\n", quote(path)?));
    }
    profile.push_str("(allow file-read* (literal \"/dev/null\") (literal \"/dev/zero\") (literal \"/dev/random\") (literal \"/dev/urandom\"))\n(allow file-write-data (literal \"/dev/null\"))\n(deny network*)\n");
    // Denies follow the workspace allow, including credentials nested in packages.
    for item in protected {
        profile.push_str(&format!(
            "(deny file-read* file-write* (subpath {}))\n",
            quote(&root.join(&item.relative))?
        ));
    }
    if let Some(dependency) = dependency {
        profile.push_str(&format!(
            "(deny file-write* (subpath {}))\n(deny file-write* (literal {}))\n",
            quote(&dependency.source)?,
            quote(&root.join("node_modules"))?
        ));
        for item in protected {
            if let Ok(relative) = item.relative.strip_prefix("node_modules") {
                profile.push_str(&format!(
                    "(deny file-read* file-write* (subpath {}))\n",
                    quote(&dependency.source.join(relative))?
                ));
            }
        }
    }
    Ok(profile)
}

#[cfg(target_os = "linux")]
fn network_filter(platform: Platform, scratch: &Path) -> Result<Option<File>, String> {
    if platform != Platform::Linux {
        return Ok(None);
    }
    use std::os::fd::AsRawFd;
    #[cfg(target_arch = "x86_64")]
    let arch = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    let arch = 0xc00000b7;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let arch = 0;
    if arch == 0 {
        return Err("automation_validation_architecture_unavailable".into());
    }
    // Classic BPF instructions for seccomp_data.arch and nr. Reject alternate ABIs,
    // Unix sockets, io_uring network operations and namespace changes.
    let mut instructions: Vec<(u16, u8, u8, u32)> = vec![
        (0x20, 0, 0, 4),
        (0x15, 1, 0, arch),
        (0x06, 0, 0, 0x80000000),
        (0x20, 0, 0, 0),
        (0x45, 0, 1, 0x40000000),
        (0x06, 0, 0, 0x80000000),
    ];
    // IPv4/IPv6 can only reach this sandbox's private loopback interface.
    instructions.extend([
        (0x15, 0, 5, libc::SYS_socket as u32),
        (0x20, 0, 0, 16),
        (0x15, 2, 0, libc::AF_INET as u32),
        (0x15, 1, 0, libc::AF_INET6 as u32),
        (0x06, 0, 0, 0x00050000 | libc::EPERM as u32),
        (0x06, 0, 0, 0x7fff0000),
    ]);
    for syscall in [
        libc::SYS_io_uring_setup,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
    ] {
        instructions.push((0x15, 0, 1, syscall as u32));
        instructions.push((0x06, 0, 0, 0x00050000 | libc::EPERM as u32));
    }
    instructions.push((0x06, 0, 0, 0x7fff0000));
    let path = scratch.join("network-filter");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|_| "automation_validation_filter")?;
    for (code, jt, jf, k) in instructions {
        output
            .write_all(&code.to_ne_bytes())
            .and_then(|_| output.write_all(&[jt, jf]))
            .and_then(|_| output.write_all(&k.to_ne_bytes()))
            .map_err(|_| "automation_validation_filter")?;
    }
    drop(output);
    let file = File::open(path).map_err(|_| "automation_validation_filter")?;
    // Only this non-secret, read-only descriptor is intentionally inherited by bwrap.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, 0) } < 0 {
        return Err("automation_validation_filter".into());
    }
    Ok(Some(file))
}

#[cfg(not(target_os = "linux"))]
fn network_filter(_: Platform, _: &Path) -> Result<Option<File>, String> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::command::CommandOutput;
    use std::sync::Mutex;

    fn spec(executable: &str, args: &[&str]) -> ValidationCommand {
        ValidationCommand {
            executable: executable.into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
        }
    }

    #[test]
    fn validation_uses_exact_names_and_literal_bounded_argv() {
        for name in [
            "/bin/sh", "sh", "bash", "git", "curl", "../node", "-p", "npm test",
        ] {
            assert!(validate_commands(&[spec(name, &["test"])]).is_err());
        }
        assert!(validate_commands(&[]).is_err());
        assert!(validate_commands(&[spec("node", &[])]).is_err());
        assert!(validate_commands(&[spec("node", &["a\0b"])]).is_err());
        assert!(
            validate_commands(&[spec("npm", &["run", "test", "--", "literal; $(nothing)"])])
                .is_ok()
        );
        assert!(installation_root(Path::new("/tmp/evil/node")).is_none());
        assert!(installation_root(Path::new("/workspace/repo/bin/node")).is_none());
    }

    #[test]
    fn private_profiles_never_grant_home_network_or_unrelated_writes() {
        let root = Scratch::new().unwrap();
        let scratch = Scratch::new().unwrap();
        let protected = vec![Protected {
            relative: PathBuf::from(".env"),
            directory: false,
        }];
        let profile = mac_profile(&root.0, &scratch.0, &protected, &[], None).unwrap();
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(deny network*)"));
        assert!(!profile.contains("(allow network"));
        assert!(!profile.contains("(allow file-read*)"));
        assert!(!profile.contains("(allow file-write*)"));
        assert!(!profile.contains("SecurityServer"));
        assert!(profile.contains(&format!(
            "(deny file-read* file-write* (subpath {}))",
            quote(&root.0.join(".env")).unwrap()
        )));
        let command = sandbox_command(
            Platform::Mac,
            Path::new("/usr/bin/sandbox-exec"),
            &root.0,
            &scratch.0,
            Path::new("/usr/bin/python3"),
            &["-c".into(), "literal; $(not a shell)".into()],
            &protected,
            &[],
            None,
            None,
        )
        .unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.last().unwrap(), "literal; $(not a shell)");
        let keys: Vec<_> = command
            .get_envs()
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        assert!(!keys
            .iter()
            .any(|key| key.contains("TOKEN") || key.contains("API_KEY")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_wrapper_has_private_namespaces_and_readonly_system_mounts() {
        let root = Scratch::new().unwrap();
        let scratch = Scratch::new().unwrap();
        let filter = network_filter(Platform::Linux, &scratch.0).unwrap();
        let mut encoded = Vec::new();
        filter
            .as_ref()
            .unwrap()
            .try_clone()
            .unwrap()
            .read_to_end(&mut encoded)
            .unwrap();
        for syscall in [libc::SYS_keyctl, libc::SYS_add_key, libc::SYS_request_key] {
            assert!(encoded
                .chunks_exact(8)
                .any(
                    |instruction| u16::from_ne_bytes(instruction[..2].try_into().unwrap()) == 0x15
                        && u32::from_ne_bytes(instruction[4..].try_into().unwrap())
                            == syscall as u32
                ));
        }
        let command = sandbox_command(
            Platform::Linux,
            Path::new("/usr/bin/bwrap"),
            &root.0,
            &scratch.0,
            Path::new("/usr/bin/python3"),
            &["test.py".into()],
            &[],
            &[],
            filter.as_ref(),
            None,
        )
        .unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        for flag in [
            "--unshare-all",
            "--new-session",
            "--die-with-parent",
            "--clearenv",
            "--disable-userns",
            "--seccomp",
        ] {
            assert!(args.iter().any(|arg| arg == flag));
        }
        assert!(!args
            .iter()
            .any(|arg| arg == "--share-net" || arg == "--unshare-user-try"));
        assert!(args
            .windows(3)
            .any(|args| args == ["--ro-bind", "/usr", "/usr"]));
        assert!(args.windows(2).any(|args| args == ["--tmpfs", "/tmp"]));
        assert_eq!(command.get_envs().count(), 0);
    }

    #[test]
    fn secrets_are_masked_and_shared_hardlinks_and_sockets_are_rejected() {
        let root = Scratch::new().unwrap();
        fs::write(root.0.join(".env"), "fake secret").unwrap();
        fs::create_dir(root.0.join(".git")).unwrap();
        let protected = inspect(&root.0).unwrap();
        assert!(protected
            .iter()
            .any(|item| item.relative == Path::new(".env")));
        assert!(protected
            .iter()
            .any(|item| item.relative == Path::new(".git") && item.directory));
        let outside = Scratch::new().unwrap();
        fs::write(outside.0.join("file"), "outside").unwrap();
        fs::hard_link(outside.0.join("file"), root.0.join("linked")).unwrap();
        assert!(inspect(&root.0).is_err());
        fs::remove_file(root.0.join("linked")).unwrap();
        // macOS's per-user temporary prefix can exceed sockaddr_un.sun_path.
        let socket_root = Scratch(Path::new("/tmp").join(format!("pv-{}", uuid::Uuid::new_v4())));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&socket_root.0)
            .unwrap();
        let _socket =
            std::os::unix::net::UnixListener::bind(socket_root.0.join("host.sock")).unwrap();
        assert!(inspect(&socket_root.0).is_err());
    }

    #[test]
    fn dependencies_require_matching_lockfiles_and_owned_cleanup() {
        let root = Scratch::new().unwrap();
        let source = Scratch::new().unwrap();
        for directory in [&root.0, &source.0] {
            fs::write(directory.join("package.json"), "{}").unwrap();
            fs::write(
                directory.join("package-lock.json"),
                "{\"lockfileVersion\":3}",
            )
            .unwrap();
        }
        fs::create_dir(source.0.join("node_modules")).unwrap();
        let commands = [spec("npm", &["test"])];
        let mount = dependency(&root.0, Some(&source.0), &commands, Platform::Linux)
            .unwrap()
            .unwrap();
        assert_eq!(mount.source, source.0.join("node_modules"));
        assert!(root.0.join("node_modules").is_dir());
        mount.verify(&root.0).unwrap();
        drop(mount);
        assert!(!root.0.join("node_modules").exists());
        fs::write(root.0.join("package-lock.json"), "changed").unwrap();
        assert!(dependency(&root.0, Some(&source.0), &commands, Platform::Linux).is_err());
    }

    struct FakeRunner {
        calls: Mutex<Vec<Vec<String>>>,
        replies: Mutex<Vec<Result<CommandOutput, CommandError>>>,
    }
    impl CommandRunner<Command> for FakeRunner {
        fn run(
            &self,
            command: &mut Command,
            input: &[u8],
            policy: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            assert!(input.is_empty());
            assert!(policy.timeout <= TIMEOUT);
            assert_eq!(policy.stdout, OutputPolicy::Capture { limit: LIMIT });
            assert_eq!(policy.stderr, OutputPolicy::Capture { limit: LIMIT });
            self.calls.lock().unwrap().push(
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            );
            self.replies.lock().unwrap().remove(0)
        }
    }
    fn reply(success: bool) -> Result<CommandOutput, CommandError> {
        Ok(CommandOutput {
            success,
            stdout: b"result".to_vec(),
            stderr: Vec::new(),
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_sandbox_never_falls_back_and_failures_stop_the_sequence() {
        if !Path::new("/usr/bin/bwrap").is_file() || !Path::new("/usr/bin/python3").is_file() {
            return;
        }
        let root = Scratch::new().unwrap();
        let commands = [
            spec("python3", &["-c", "print(1)"]),
            spec("python3", &["-c", "print(2)"]),
        ];
        let fake = FakeRunner {
            calls: Mutex::default(),
            replies: Mutex::new(vec![reply(false)]),
        };
        assert!(run(&fake, &root.0, &commands, None)
            .unwrap_err()
            .contains("sandbox_unavailable"));
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
        let fake = FakeRunner {
            calls: Mutex::default(),
            replies: Mutex::new(vec![reply(true), reply(false)]),
        };
        let result = run(&fake, &root.0, &commands, None).unwrap();
        assert!(!result.passed);
        assert_eq!(result.commands.len(), 1);
        let fake = FakeRunner {
            calls: Mutex::default(),
            replies: Mutex::new(vec![reply(true), Err(CommandError::OutputLimit)]),
        };
        assert!(run(&fake, &root.0, &commands, None)
            .unwrap_err()
            .contains("output_limit"));
    }

    /// Execute explicitly on a Linux host that permits unprivileged namespaces.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a Linux host permitting bubblewrap user/network namespaces"]
    fn live_linux_validation_isolates_host_and_allows_private_loopback() {
        let root = Scratch::new().unwrap();
        let outside = Scratch::new().unwrap();
        fs::write(root.0.join(".env"), "fake-secret").unwrap();
        fs::write(outside.0.join("secret"), "host-secret").unwrap();
        let host = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = host.local_addr().unwrap().port();
        let script = format!(
            r#"import os,pathlib,socket
assert os.environ['HOME']=='/tmp/home'
assert not any(k.endswith('_TOKEN') or k.endswith('_API_KEY') for k in os.environ)
try:
 assert pathlib.Path('.env').read_text()!='fake-secret'
except OSError: pass
assert not pathlib.Path({secret}).exists()
try:
 pathlib.Path({outside}).write_text('escape')
 raise AssertionError('outside write succeeded')
except OSError: pass
try:
 socket.socket(socket.AF_UNIX)
 raise AssertionError('host Unix socket capability available')
except OSError: pass
remote=socket.socket();remote.settimeout(0.2)
try:
 remote.connect(('127.0.0.1',{port}))
 raise AssertionError('host network reached')
except OSError: pass
server=socket.socket();server.bind(('127.0.0.1',0));server.listen(1)
client=socket.socket();client.connect(server.getsockname())
accepted,_=server.accept();client.sendall(b'ok');assert accepted.recv(2)==b'ok'
pathlib.Path('checked.txt').write_text('isolated')
print('isolated validation passed')
"#,
            secret = serde_json::to_string(&outside.0.join("secret")).unwrap(),
            outside = serde_json::to_string(&outside.0.join("escaped")).unwrap()
        );
        let result = run(
            &prometeu_process::command::UnixCommandRunner,
            &root.0,
            &[spec("python3", &["-c", &script])],
            None,
        )
        .unwrap();
        assert!(result.passed, "{result:?}");
        assert_eq!(
            fs::read_to_string(root.0.join("checked.txt")).unwrap(),
            "isolated"
        );
        assert!(!outside.0.join("escaped").exists());
        assert_eq!(
            fs::read_to_string(outside.0.join("secret")).unwrap(),
            "host-secret"
        );
    }
}
