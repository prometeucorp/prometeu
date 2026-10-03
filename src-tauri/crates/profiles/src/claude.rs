use crate::{io, Profile, ProfileFiles};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Claude stores MCP definitions beside the default home or inside a configured home.
pub fn config_file(home: &Path) -> PathBuf {
    let nested = home.join(".claude.json");
    if nested.exists() {
        nested
    } else {
        home.with_extension("json")
    }
}

const AUTH_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

pub fn account_env(command: &mut Command, profile: &Profile) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("CLAUDE") {
            command.env_remove(key);
        }
    }
    if profile.managed || std::env::var_os("CLAUDE_CONFIG_DIR").is_some() {
        command.env("CLAUDE_CONFIG_DIR", &profile.home);
    }
    if profile.managed {
        for key in AUTH_ENV {
            command.env_remove(key);
        }
    }
}

pub fn prepare_profile_at(
    files: &dyn ProfileFiles,
    base: &Path,
    home: &Path,
) -> Result<(), String> {
    for name in [
        "projects",
        "plugins",
        "skills",
        "commands",
        "agents",
        "plans",
        "tasks",
        "file-history",
        "session-env",
    ] {
        crate::files::share(files, base, home, name, true)?;
    }
    crate::files::share(files, base, home, "CLAUDE.md", false)?;
    let body = match std::fs::read_to_string(base.join("settings.json")) {
        Ok(body) => Some(body),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io(error)),
    };
    if let Some(body) = body {
        let mut settings: Value = serde_json::from_str(&body).map_err(io)?;
        settings
            .as_object_mut()
            .ok_or_else(|| prometeu_core::error::code("err.account.profile"))?
            .remove("apiKeyHelper");
        if let Some(env) = settings["env"].as_object_mut() {
            env.retain(|key, _| !AUTH_ENV.contains(&key.as_str()) && key != "CLAUDE_CONFIG_DIR");
        }
        files.write_private(&home.join("settings.json"), &settings.to_string())?;
    }
    // MCP configuration and project trust live outside settings.json. Preserve the profile's login
    // identity without copying the global identity.
    let source = config_file(base);
    if source.exists() {
        let global: Value =
            serde_json::from_str(&std::fs::read_to_string(source).map_err(io)?).map_err(io)?;
        let target = home.join(".claude.json");
        let mut local: Value = match std::fs::read_to_string(&target) {
            Ok(body) => serde_json::from_str(&body).map_err(io)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(error) => return Err(io(error)),
        };
        let local = local
            .as_object_mut()
            .ok_or_else(|| prometeu_core::error::code("err.account.profile"))?;
        for name in ["mcpServers", "projects"] {
            if let Some(value) = global.get(name) {
                local.insert(name.into(), value.clone());
            }
        }
        files.write_private(&target, &serde_json::to_string(local).map_err(io)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accounts_share_transcripts_and_plugins_without_copying_login() {
        let root =
            std::env::temp_dir().join(format!("prometeu-claude-accounts-{}", uuid::Uuid::new_v4()));
        let base = root.join("base");
        let first = root.join("first");
        let second = root.join("second");
        for home in [&base, &first, &second] {
            std::fs::create_dir_all(home).unwrap();
        }
        std::fs::write(base.join(".credentials.json"), "original login").unwrap();
        std::fs::write(base.join(".claude.json"), r#"{"oauthAccount":{"email":"original@example.com"},"mcpServers":{"local":{"command":"echo"}},"projects":{"/repo":{"hasTrustDialogAccepted":true}}}"#).unwrap();
        std::fs::write(base.join("settings.json"), r#"{"env":{"ANTHROPIC_API_KEY":"secret","CLAUDE_CONFIG_DIR":"/other-account","KEEP":"yes"},"apiKeyHelper":"other-key","enabledPlugins":{"test":true}}"#).unwrap();
        prepare_profile_at(&crate::tests::Files, &base, &first).unwrap();
        prepare_profile_at(&crate::tests::Files, &base, &second).unwrap();
        std::fs::write(first.join(".credentials.json"), "first account").unwrap();
        std::fs::write(
            first.join("projects/conversation.jsonl"),
            "complete transcript",
        )
        .unwrap();
        prepare_profile_at(&crate::tests::Files, &base, &first).unwrap();
        assert_eq!(
            std::fs::read_to_string(second.join("projects/conversation.jsonl")).unwrap(),
            "complete transcript"
        );
        assert_eq!(
            std::fs::read_link(second.join("plugins")).unwrap(),
            base.join("plugins")
        );
        assert_eq!(
            std::fs::read_to_string(first.join(".credentials.json")).unwrap(),
            "first account"
        );
        assert!(!second.join(".credentials.json").exists());
        assert_eq!(
            std::fs::read_to_string(base.join(".credentials.json")).unwrap(),
            "original login"
        );
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(first.join("settings.json")).unwrap())
                .unwrap();
        assert!(settings["apiKeyHelper"].is_null());
        assert!(settings["env"]["ANTHROPIC_API_KEY"].is_null());
        assert_eq!(settings["env"]["KEEP"], "yes");
        let config: Value =
            serde_json::from_str(&std::fs::read_to_string(first.join(".claude.json")).unwrap())
                .unwrap();
        assert!(config["oauthAccount"].is_null());
        assert_eq!(config["mcpServers"]["local"]["command"], "echo");
        std::fs::remove_dir_all(root).unwrap();
    }
}
