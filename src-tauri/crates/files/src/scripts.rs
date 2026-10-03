//! Native workspace preparation shared by desktop and WSL hosts.
use crate::settings::safe;
use std::path::Path;
pub trait WorkspacePreparation: Send + Sync {
    fn hydrate(&self, worktree: &Path, repo: &Path, list: &[String]) -> Vec<Copied>;
    fn allocate_port(&self, worktree: &Path, taken: &[u16]) -> Option<u16>;
}
pub struct NativePreparation;
impl WorkspacePreparation for NativePreparation {
    fn hydrate(&self, worktree: &Path, repo: &Path, list: &[String]) -> Vec<Copied> {
        hydrate(worktree, repo, list)
    }
    fn allocate_port(&self, worktree: &Path, taken: &[u16]) -> Option<u16> {
        alloc_port(worktree, taken)
    }
}
/* Worktree hydration */

/// Report each copy result in the Setup header so missing or failed preparation remains visible.
pub enum Copied {
    Made(String),
    /// Report existing worktree files too, explaining why local edits are preserved instead of
    /// replaced by clone copies.
    Kept(String),
    Failed(String, String),
}

/// Copy only missing files before setup. Preserve committed or manually edited destination files so
/// rerunning setup is safe.
pub fn hydrate(worktree: &Path, repo: &Path, list: &[String]) -> Vec<Copied> {
    if worktree == repo {
        return Vec::new();
    }
    list.iter()
        .filter_map(|rel| {
            let path = safe(rel)?;
            let to = worktree.join(&path);
            if to.exists() {
                return Some(Copied::Kept(rel.clone()));
            }
            Some(match copy_into(&repo.join(&path), &to) {
                Ok(()) => Copied::Made(rel.clone()),
                Err(e) => Copied::Failed(rel.clone(), e),
            })
        })
        .collect()
}

/// Copy files or complete directories while retaining permissions required by private keys.
fn copy_into(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(crate::io)?;
    }
    if !from.is_dir() {
        return std::fs::copy(from, to).map(|_| ()).map_err(crate::io);
    }
    std::fs::create_dir_all(to).map_err(crate::io)?;
    for entry in std::fs::read_dir(from).map_err(crate::io)? {
        let entry = entry.map_err(crate::io)?;
        copy_into(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

/// Omit the Setup copy header when there are no entries.
pub fn report(notes: &[Copied], pick: impl Fn(&str, &str) -> String) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let mut out = String::new();
    for note in notes {
        out.push_str(&match note {
            Copied::Made(path) => {
                format!(
                    "\x1b[32m→\x1b[0m {path} {}\r\n",
                    pick("veio do clone", "copied from the clone")
                )
            }
            Copied::Kept(path) => {
                format!(
                    "\x1b[2m· {path} {}\x1b[0m\r\n",
                    pick("já estava aqui", "already here")
                )
            }
            Copied::Failed(path, why) => format!("\x1b[31m✗\x1b[0m {path}: {why}\r\n"),
        });
    }
    out.push_str("\r\n");
    Some(out)
}

/// Expose both Prometeu and Conductor environment names for compatible scripts. Also set
/// conventional PORT so common development servers started directly from the dock use the workspace
/// port.
pub fn env(worktree: &Path, repo: &Path, name: &str, port: Option<u16>) -> Vec<(String, String)> {
    let mut pairs = vec![
        ("WORKSPACE_PATH", worktree.display().to_string()),
        ("ROOT_PATH", repo.display().to_string()),
        ("WORKSPACE_NAME", name.to_string()),
    ];
    if let Some(port) = port {
        pairs.push(("PORT", port.to_string()));
    }
    let mut out: Vec<(String, String)> = pairs
        .into_iter()
        .flat_map(|(key, value)| {
            [
                (format!("PROMETEU_{key}"), value.clone()),
                (format!("CONDUCTOR_{key}"), value),
            ]
        })
        .collect();
    if let Some(port) = port {
        out.push(("PORT".into(), port.to_string()));
    }
    out
}

/// Reserve ten consecutive ports per workspace, aligned to multiples of ten. Exclude saved
/// reservations even when their servers are stopped. Begin probing at a stable hash of the worktree
/// path to reduce collisions across independent app boards. Hash collisions remain possible within
/// the 690 ranges; actual binds provide the final check.
const FIRST: u16 = 3100;
pub const SLOTS: u16 = (9990 - FIRST) / 10 + 1;

/// Derive the initial port range only from the worktree path so it is stable across boards and
/// machine activity.
pub fn port_start(worktree: &Path) -> u16 {
    (fnv1a(&worktree.to_string_lossy()) % u64::from(SLOTS)) as u16
}

pub fn alloc_port(worktree: &Path, taken: &[u16]) -> Option<u16> {
    let start = port_start(worktree);
    (0..SLOTS)
        .map(|i| FIRST + ((start + i) % SLOTS) * 10)
        .find(|base| !taken.contains(base) && (0..10).all(|i| usable(base + i) && free(base + i)))
}

/// Exclude browser-blocked Fetch ports within the allocator range. A server may answer curl while
/// Chromium or WebKit refuses its port.
const BAD: &[u16] = &[
    3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6697, 10080,
];

/// Check whether browsers permit the localhost port.
pub fn usable(port: u16) -> bool {
    !BAD.contains(&port)
}

/// Check both IPv4 and IPv6 loopbacks. Only address-in-use errors prove a conflict; unavailable
/// IPv6 does not.
fn free(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener};
    [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ]
    .into_iter()
    .all(|ip| match TcpListener::bind((ip, port)) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::AddrInUse,
    })
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Preserve the desktop warning attached after an unsuccessful setup.
pub fn setup_warning(code: Option<u32>, pick: impl Fn(&str, &str) -> String) -> Option<String> {
    match code {
        Some(0) => None,
        Some(n) => Some(pick(
            &format!("(O setup deste worktree saiu com código {n} — veja a aba Setup; pode faltar dependência.) "),
            &format!("(This worktree's setup exited with code {n} — see the Setup tab; a dependency may be missing.) "),
        )),
        None => Some(pick(
            "(O setup deste worktree foi encerrado antes de terminar — veja a aba Setup; pode faltar dependência.) ",
            "(This worktree's setup was stopped before it finished — see the Setup tab; a dependency may be missing.) ",
        )),
    }
}

/// Renaming a card must not rename script resources such as containers or databases.
pub fn workspace_env(workspace: &prometeu_core::board::Workspace) -> Vec<(String, String)> {
    let name = Path::new(&workspace.worktree)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| workspace.branch.replace('/', "-"));
    let primary = workspace.primary();
    env(
        Path::new(&primary.worktree),
        Path::new(&primary.path),
        &name,
        workspace.port,
    )
}
