use super::*;

pub(crate) struct Files;
impl ProfileFiles for Files {
    fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
        std::fs::create_dir_all(path).map_err(io)
    }
    fn write_private(&self, path: &Path, body: &str) -> Result<(), String> {
        std::fs::write(path, body).map_err(io)
    }
}

fn account(provider: ProviderId, id: &str) -> Account {
    Account {
        id: id.into(),
        provider,
        revision: 4,
        auth_method: None,
        key_suffix: None,
        identity: Default::default(),
    }
}
fn backend(root: &Path, files: Arc<dyn ProfileFiles>) -> NativeProfiles {
    NativeProfiles::new(
        root.join("app"),
        root.join("claude"),
        root.join("codex"),
        root.join("external"),
        files,
    )
}
fn temporary() -> PathBuf {
    std::env::temp_dir().join(format!("prometeu-profile-{}", uuid::Uuid::new_v4()))
}

#[test]
fn profiles_resolve_under_explicit_roots_and_capture_the_selected_revision() {
    let root = temporary();
    let profiles = backend(&root, Arc::new(Files));
    let mut selected = account(ProviderId::Claude, "00000000-0000-4000-8000-000000000001");
    let captured = profiles.resolve(&selected).unwrap();
    selected.revision += 1;
    assert_eq!(captured.revision, 4);
    assert_eq!(profiles.resolve(&selected).unwrap().revision, 5);
    assert_eq!(captured.home, root.join("app/accounts").join(&selected.id));
    for (provider, name) in [
        (ProviderId::Claude, "claude"),
        (ProviderId::Codex, "codex"),
        (ProviderId::Antigravity, "external"),
    ] {
        let profile = profiles.resolve(&account(provider, key(provider))).unwrap();
        assert!(!profile.managed);
        assert_eq!(profile.home, root.join(name));
        profiles.prepare(&profile).unwrap();
    }
    assert!(
        !root.exists(),
        "resolving or preparing external profiles must not create files"
    );
}

#[test]
fn managed_child_environments_remove_alternative_credentials_without_rewriting_caller_values() {
    let root = temporary();
    let backend = backend(&root, Arc::new(Files));
    for (provider, home_key, secrets) in [
        (
            ProviderId::Claude,
            "CLAUDE_CONFIG_DIR",
            vec![
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_BASE_URL",
                "CLAUDE_CODE_OAUTH_TOKEN",
            ],
        ),
        (
            ProviderId::Codex,
            "CODEX_HOME",
            vec![
                "OPENAI_API_KEY",
                "CODEX_API_KEY",
                "CODEX_ACCESS_TOKEN",
                "OPENAI_BASE_URL",
                "CODEX_CHATGPT_BASE_URL",
            ],
        ),
    ] {
        let profile = backend
            .resolve(&account(provider, "00000000-0000-4000-8000-000000000001"))
            .unwrap();
        let mut command = Command::new("unused-fixture-command");
        command.env("KEEP", "caller value");
        for secret in &secrets {
            command.env(secret, "must not reach child");
        }
        backend.apply(&profile, &mut command).unwrap();
        let environment: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment[std::ffi::OsStr::new(home_key)],
            Some(profile.home.as_os_str())
        );
        assert_eq!(
            environment[std::ffi::OsStr::new("KEEP")],
            Some(std::ffi::OsStr::new("caller value"))
        );
        for secret in secrets {
            assert_eq!(environment[std::ffi::OsStr::new(secret)], None);
        }
    }
}

#[test]
fn external_antigravity_keeps_its_environment_and_rejects_managed_preparation() {
    let root = temporary();
    let backend = backend(&root, Arc::new(Files));
    let mut profile = backend
        .resolve(&account(ProviderId::Antigravity, "antigravity"))
        .unwrap();
    let mut command = Command::new("unused-fixture-command");
    command.env("EXTERNAL_AUTH", "unchanged");
    backend.apply(&profile, &mut command).unwrap();
    assert_eq!(command.get_envs().count(), 1);
    profile.managed = true;
    assert_eq!(
        backend.apply(&profile, &mut command).err(),
        Some(code("err.account.external"))
    );
    assert_eq!(
        backend.prepare(&profile).err(),
        Some(code("err.account.external"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_file_failures_propagate_without_double_encoding_or_replacing_credentials() {
    struct FailingFiles {
        directory: bool,
    }
    impl ProfileFiles for FailingFiles {
        fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
            if self.directory {
                Err(code("directory.failed"))
            } else {
                Files.ensure_private_dir(path)
            }
        }
        fn write_private(&self, _: &Path, _: &str) -> Result<(), String> {
            Err(code("write.failed"))
        }
    }
    let root = temporary();
    for directory in [true, false] {
        let backend = backend(&root, Arc::new(FailingFiles { directory }));
        let profile = backend
            .resolve(&account(
                ProviderId::Codex,
                "00000000-0000-4000-8000-000000000001",
            ))
            .unwrap();
        std::fs::create_dir_all(&profile.home).unwrap();
        std::fs::write(profile.home.join("auth.json"), "existing private login").unwrap();
        let expected = if directory {
            "directory.failed"
        } else {
            "write.failed"
        };
        assert_eq!(backend.prepare(&profile).err(), Some(code(expected)));
        assert_eq!(
            std::fs::read_to_string(profile.home.join("auth.json")).unwrap(),
            "existing private login"
        );
        assert!(!profile.home.join("config.toml").exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn conflicting_shared_paths_are_refused_without_replacing_existing_data() {
    let root = temporary();
    let backend = backend(&root, Arc::new(Files));
    let profile = backend
        .resolve(&account(
            ProviderId::Claude,
            "00000000-0000-4000-8000-000000000001",
        ))
        .unwrap();
    std::fs::create_dir_all(&profile.home).unwrap();
    let conflict = profile.home.join("projects");
    std::fs::write(&conflict, "owned history").unwrap();
    assert_eq!(
        backend.prepare(&profile).err(),
        Some(code("err.account.profile"))
    );
    assert_eq!(std::fs::read_to_string(&conflict).unwrap(), "owned history");
    std::fs::remove_dir_all(root).unwrap();
}
