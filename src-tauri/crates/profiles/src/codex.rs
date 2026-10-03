use crate::{io, Profile, ProfileFiles};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn account_env(command: &mut Command, profile: &Profile) {
    command.env("CODEX_HOME", &profile.home);
    if profile.managed {
        for key in [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "CODEX_ACCESS_TOKEN",
            "OPENAI_BASE_URL",
            "CODEX_CHATGPT_BASE_URL",
        ] {
            command.env_remove(key);
        }
    }
}

pub fn prepare_profile_at(
    files: &dyn ProfileFiles,
    base: &Path,
    home: &Path,
) -> Result<(), String> {
    // Share the same rollout and native index across account changes. Never link auth.json into the
    // profile.
    for name in [
        "sessions",
        "archived_sessions",
        "skills",
        "plugins",
        "packages",
        "agents",
        "rules",
        "memories",
        "thread-writer-locks",
    ] {
        crate::files::share(files, base, home, name, true)?;
    }
    for name in ["AGENTS.md", "hooks.json"] {
        crate::files::share(files, base, home, name, false)?;
    }
    let body = match std::fs::read_to_string(base.join("config.toml")) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io(error)),
    };
    let mut config: toml::Table = toml::from_str(&body).map_err(io)?;
    config.insert("cli_auth_credentials_store".into(), "file".into());
    config.insert("model_provider".into(), "openai".into());
    for name in ["openai_base_url", "chatgpt_base_url", "profile"] {
        config.remove(name);
    }
    if let Some(providers) = config
        .get_mut("model_providers")
        .and_then(toml::Value::as_table_mut)
    {
        providers.remove("openai");
    }
    let sqlite = config
        .get("sqlite_home")
        .and_then(toml::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| base.to_path_buf());
    let sqlite = if sqlite.is_absolute() {
        sqlite
    } else {
        base.join(sqlite)
    };
    config.insert(
        "sqlite_home".into(),
        sqlite.to_string_lossy().to_string().into(),
    );
    files.write_private(
        &home.join("config.toml"),
        &toml::to_string(&config).map_err(io)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_resume_the_same_rollout_with_separate_credentials() {
        let root =
            std::env::temp_dir().join(format!("prometeu-codex-accounts-{}", uuid::Uuid::new_v4()));
        let base = root.join("base");
        let first = root.join("first");
        let second = root.join("second");
        for home in [&base, &first, &second] {
            std::fs::create_dir_all(home).unwrap();
        }
        std::fs::write(base.join("auth.json"), "original login").unwrap();
        std::fs::write(base.join("config.toml"), "cli_auth_credentials_store = 'keyring'\nmodel_provider = 'custom'\n[features]\nplugins = true\n").unwrap();
        prepare_profile_at(&crate::tests::Files, &base, &first).unwrap();
        prepare_profile_at(&crate::tests::Files, &base, &second).unwrap();
        std::fs::write(first.join("auth.json"), "first account").unwrap();
        std::fs::write(second.join("auth.json"), "second account").unwrap();
        std::fs::write(first.join("sessions/rollout.jsonl"), "same thread").unwrap();
        prepare_profile_at(&crate::tests::Files, &base, &first).unwrap();
        assert_eq!(
            std::fs::read_to_string(second.join("sessions/rollout.jsonl")).unwrap(),
            "same thread"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("auth.json")).unwrap(),
            "original login"
        );
        assert_eq!(
            std::fs::read_to_string(first.join("auth.json")).unwrap(),
            "first account"
        );
        assert_eq!(
            std::fs::read_to_string(second.join("auth.json")).unwrap(),
            "second account"
        );
        let config: toml::Table =
            toml::from_str(&std::fs::read_to_string(first.join("config.toml")).unwrap()).unwrap();
        assert_eq!(config["sqlite_home"].as_str(), base.to_str());
        assert_eq!(config["cli_auth_credentials_store"].as_str(), Some("file"));
        assert_eq!(config["model_provider"].as_str(), Some("openai"));
        assert_eq!(config["features"]["plugins"].as_bool(), Some(true));
        std::fs::write(base.join("config.toml"), "sqlite_home = 'state'\n").unwrap();
        prepare_profile_at(&crate::tests::Files, &base, &second).unwrap();
        let config: toml::Table =
            toml::from_str(&std::fs::read_to_string(second.join("config.toml")).unwrap()).unwrap();
        assert_eq!(config["sqlite_home"].as_str(), base.join("state").to_str());
        std::fs::remove_dir_all(root).unwrap();
    }
}
