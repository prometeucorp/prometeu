use crate::{io, ProfileFiles};
use prometeu_core::error::code;
use std::path::Path;

/// Link only explicitly shared data. Login, tokens, identity, and authentication caches remain
/// private to the profile.
pub(crate) fn share(
    files: &dyn ProfileFiles,
    base: &Path,
    home: &Path,
    name: &str,
    directory: bool,
) -> Result<(), String> {
    let source = base.join(name);
    let target = home.join(name);
    if directory {
        files.ensure_private_dir(&source)?;
    }
    if !source.exists() {
        return Ok(());
    }
    match std::fs::symlink_metadata(&target) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                && std::fs::read_link(&target).ok().as_ref() == Some(&source) =>
        {
            Ok(())
        }
        Ok(_) => Err(code("err.account.profile")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::os::unix::fs::symlink(&source, &target) {
                Ok(()) => Ok(()),
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && std::fs::read_link(&target).ok().as_ref() == Some(&source) =>
                {
                    Ok(())
                }
                Err(error) => Err(io(error)),
            }
        }
        Err(error) => Err(io(error)),
    }
}
