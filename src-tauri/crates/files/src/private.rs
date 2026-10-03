//! Private atomic file writes shared by execution-side hosts.
use std::path::Path;
/// Reaffirm 0700 on state, transcript, and credential directories independently of umask; these
/// directories are not shareable workspace content.
pub fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Atomically replace private 0600 files without exposing a truncated intermediate file. Callers
/// translate raw I/O errors.
pub fn write_private(target: &Path, body: &str) -> Result<(), String> {
    write_private_bytes(target, body.as_bytes())
}

/// Apply the same private atomic write to bytes, preserving imported snapshots and transcripts
/// exactly.
pub fn write_private_bytes(target: &Path, body: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some(dir) = target.parent() {
        ensure_private_dir(dir)?;
    }
    let filename = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("private");
    let tmp = target.with_file_name(format!(".{filename}.{}.tmp", uuid::Uuid::new_v4()));
    let mut opts = std::fs::OpenOptions::new();
    // Use create_new to reject temporary-name collisions, including symlinks, rather than
    // truncating existing entries.
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
    if let Err(error) = file.write_all(body).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error.to_string());
    }
    drop(file);
    if let Err(error) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        if let Some(dir) = target.parent() {
            std::fs::File::open(dir)
                .and_then(|directory| directory.sync_all())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
