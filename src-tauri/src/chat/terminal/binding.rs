//! Durable native path binding learned only from an authenticated root hook.
use crate::{paths, state::ProviderId};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 65_536;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Binding {
    version: u8,
    session: String,
    provider: ProviderId,
    native_session: String,
    pub transcript: PathBuf,
}

impl Binding {
    pub fn new(
        session: &str,
        provider: ProviderId,
        native_session: &str,
        transcript: &Path,
    ) -> Self {
        Self {
            version: 1,
            session: session.into(),
            provider,
            native_session: native_session.into(),
            transcript: transcript.into(),
        }
    }

    /// Account changes may share native history. The observer reauthorizes this exact path
    /// against the newly captured account's roots; an old account id never grants access.
    pub fn load(
        root: &Path,
        session: &str,
        provider: ProviderId,
        native_session: &str,
    ) -> Result<Option<Self>, ()> {
        let value = Self::peek(root, session, provider)?;
        if value
            .as_ref()
            .is_some_and(|value| value.native_session != native_session)
        {
            return Err(());
        }
        Ok(value)
    }

    pub(super) fn bound_native(
        root: &Path,
        session: &str,
        provider: ProviderId,
    ) -> Result<Option<String>, ()> {
        Ok(Self::peek(root, session, provider)?.map(|value| value.native_session))
    }

    fn peek(root: &Path, session: &str, provider: ProviderId) -> Result<Option<Self>, ()> {
        let path = file(root, session)?;
        let source = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(()),
        };
        let metadata = source.metadata().map_err(|_| ())?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(());
        }
        let mut bytes = Vec::new();
        source
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() as u64 > LIMIT {
            return Err(());
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| ())?;
        if value.version != 1
            || value.session != session
            || value.provider != provider
            || value.native_session.is_empty()
            || value.native_session.len() > 256
            || value.native_session.chars().any(char::is_control)
            || !value.transcript.is_absolute()
        {
            return Err(());
        }
        Ok(Some(value))
    }

    pub fn save(&self, root: &Path) -> Result<(), ()> {
        let body = serde_json::to_string(self).map_err(|_| ())?;
        if body.len() as u64 > LIMIT {
            return Err(());
        }
        paths::write_private(&file(root, &self.session)?, &body).map_err(|_| ())
    }
}

fn file(root: &Path, session: &str) -> Result<PathBuf, ()> {
    if session.is_empty()
        || session.len() > 256
        || !session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(());
    }
    Ok(root
        .join("terminal-bindings")
        .join(format!("{session}.json")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn bindings_require_exact_native_and_app_identity_and_private_storage() {
        let root = std::env::temp_dir().join(format!("prometeu-binding-{}", uuid::Uuid::new_v4()));
        assert!(
            Binding::load(&root, "session", ProviderId::Antigravity, "native")
                .unwrap()
                .is_none()
        );
        let binding = Binding::new(
            "session",
            ProviderId::Antigravity,
            "native",
            Path::new("/native/history.jsonl"),
        );
        binding.save(&root).unwrap();
        assert_eq!(
            Binding::load(&root, "session", ProviderId::Antigravity, "native").unwrap(),
            Some(binding)
        );
        assert!(Binding::load(&root, "session", ProviderId::Antigravity, "other").is_err());
        assert!(Binding::load(&root, "session", ProviderId::Cursor, "native").is_err());
        assert!(Binding::load(&root, "../session", ProviderId::Antigravity, "native").is_err());
        let path = file(&root, "session").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        saved["version"] = 2.into();
        std::fs::write(path, saved.to_string()).unwrap();
        assert!(Binding::load(&root, "session", ProviderId::Antigravity, "native").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
