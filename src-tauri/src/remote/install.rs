//! Installing the host on a machine: one immutable directory per build, verified by SHA-256
//! before it becomes reachable, with the previous build kept for rollback (ADR 0101).
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The only build the app ships today. `uname -sm` must match before anything is uploaded.
pub const PLATFORM: &str = "Linux x86_64";
const BINARY_NAME: &str = "prometeu-host-x86_64-linux";

pub struct Binary {
    pub bytes: Vec<u8>,
    /// The first 16 hex digits of the SHA-256: the install directory name.
    pub hash: String,
    pub sha256: String,
}

/// Development and release builds keep separate remote roots, as they keep separate boards.
pub fn remote_root() -> String {
    if cfg!(debug_assertions) {
        format!("{}-dev", prometeu_host::ROOT)
    } else {
        prometeu_host::ROOT.to_string()
    }
}

fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("PROMETEU_REMOTE_HOST") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(contents) = exe.parent().and_then(|dir| dir.parent()) {
            paths.push(contents.join("Resources/remote-host").join(BINARY_NAME));
        }
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("remote-host").join(BINARY_NAME));
        }
    }
    if cfg!(debug_assertions) {
        paths.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/x86_64-unknown-linux-musl/release/prometeu-host"),
        );
    }
    paths
}

/// The Linux host build bundled with this app, read once.
pub fn local_binary() -> Option<&'static Binary> {
    static BINARY: OnceLock<Option<Binary>> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let bytes = candidates()
                .into_iter()
                .find_map(|path| std::fs::read(path).ok())?;
            let sha256 = format!("{:x}", Sha256::digest(&bytes));
            Some(Binary {
                hash: sha256[..16].to_string(),
                sha256,
                bytes,
            })
        })
        .as_ref()
}

/// A short SSH invocation. It never prompts (the person's keys and agent authenticate) and
/// bypasses connection sharing: a master connection left stale by sleep must not hang it.
pub fn ssh(target: &str) -> Command {
    let mut command = Command::new("ssh");
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        // Detect a connection that died with sleep or a network change within about 15 s.
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=3",
        "--",
        target,
    ]);
    command
}

/// Run a shell script on the machine with optional input, bounded by `timeout`.
pub fn run(target: &str, script: &str, input: &[u8], timeout: Duration) -> Result<String, String> {
    let mut child = ssh(target)
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| crate::i18n::ta("err.remote.ssh", &[("detail", e.to_string())]))?;
    let input = input.to_vec();
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).to_string()
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(crate::i18n::ta(
                    "err.remote.ssh",
                    &[("detail", "timeout".to_string())],
                ));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if status.success() {
        Ok(stdout)
    } else {
        let detail = stderr.trim().lines().last().unwrap_or("").to_string();
        Err(crate::i18n::ta("err.remote.ssh", &[("detail", detail)]))
    }
}

pub struct Probe {
    pub platform: String,
}

/// Reach the machine and report what installation needs to know.
pub fn probe(target: &str) -> Result<Probe, String> {
    let out = run(target, "uname -sm", b"", Duration::from_secs(30))?;
    Ok(Probe {
        platform: out.lines().next().unwrap_or("").trim().to_string(),
    })
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Upload and verify this app's host build, keep the previous one, and enable linger so the
/// person's systemd manager (and the limits it enforces) outlives their last SSH session.
pub fn install(target: &str) -> Result<Probe, String> {
    let binary = local_binary().ok_or_else(|| crate::i18n::t("err.remote.noBuild"))?;
    let probe = probe(target)?;
    if probe.platform != PLATFORM {
        return Err(crate::i18n::ta(
            "err.remote.platform",
            &[("platform", probe.platform)],
        ));
    }
    let root = remote_root();
    let script = format!(
        r#"set -e
root="$HOME/{root}"
dir="$root/host/{hash}"
mkdir -p "$dir"
chmod 700 "$root"
tmp="$dir/prometeu-host.$$.tmp"
cat > "$tmp"
sum=$( (sha256sum "$tmp" 2>/dev/null || shasum -a 256 "$tmp") | cut -d' ' -f1)
if [ "$sum" != {sha} ]; then rm -f "$tmp"; echo "checksum mismatch" >&2; exit 3; fi
chmod 700 "$tmp"
mv -f "$tmp" "$dir/prometeu-host"
if [ -L "$root/current" ]; then
  previous=$(readlink "$root/current")
  if [ "$previous" != "$dir" ]; then ln -sfn "$previous" "$root/previous"; fi
fi
ln -sfn "$dir" "$root/current"
keep_previous=$(readlink "$root/previous" 2>/dev/null || true)
# A build stays while a host runs it or a conversation's hook settings still name it.
in_use() {{
  pgrep -f "$1/prometeu-host" >/dev/null 2>&1 && return 0
  grep -qsF "$1/prometeu-host" "$root"/sessions/*/settings.json
}}
for old in "$root"/host/*; do
  [ "$old" = "$dir" ] || [ "$old" = "$keep_previous" ] || in_use "$old" || rm -rf "$old"
done
loginctl enable-linger >/dev/null 2>&1 || true
"#,
        root = root,
        hash = binary.hash,
        sha = quote(&binary.sha256),
    );
    run(target, &script, &binary.bytes, Duration::from_secs(300))?;
    Ok(probe)
}
