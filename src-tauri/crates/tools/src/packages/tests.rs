use super::*;
#[test]
fn cleanup_finds_all_accounts_without_following_links() {
    let root =
        std::env::temp_dir().join(format!("prometeu-account-plugins-{}", uuid::Uuid::new_v4()));
    let home = root.join("workspace");
    let account = home.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&account).unwrap();
    std::fs::write(home.join("config.toml"), "").unwrap();
    std::fs::write(account.join("config.toml"), "").unwrap();
    std::os::unix::fs::symlink(&account, home.join(uuid::Uuid::new_v4().to_string())).unwrap();
    let homes = codex_homes_at(&root);
    assert_eq!(homes.len(), 2);
    assert!(homes.contains(&home));
    assert!(homes.contains(&account));
    std::fs::remove_dir_all(root).unwrap();
}

/// Generate a native marketplace entry and Codex manifest while retaining compatible hooks.
/// Content-hashed versions invalidate unchanged upstream version numbers.
#[test]
fn codex_marketplace_uses_the_same_plugin() {
    let root = std::env::temp_dir().join(format!("prometeu-codex-market-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let market = root.join("market");
    std::fs::create_dir_all(source.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(source.join("skills").join("short")).unwrap();
    std::fs::create_dir_all(source.join("commands")).unwrap();
    std::fs::write(
            source.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"short","description":"reply briefly","hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"sh ${CLAUDE_PLUGIN_ROOT}/hooks/short.sh"}]}]}}"#,
        )
        .unwrap();
    std::fs::write(
        source.join("skills/short/SKILL.md"),
        "---\nname: short\n---\n",
    )
    .unwrap();
    std::fs::write(
        source.join(".mcp.json"),
        r#"{"mcpServers":{"short":{"command":"node","args":["server.js"]}}}"#,
    )
    .unwrap();

    let prepared = host()
        .prepare_marketplace(
            &market,
            "prometeu-dev",
            &[plugin("short", &source.display().to_string())],
        )
        .unwrap();
    assert_eq!(prepared[0].id, "short");
    assert_eq!(prepared[0].canonical, "short@prometeu-dev");
    assert!(prepared[0].version.starts_with("0.0.0+prometeu."));
    assert!(prepared[0].hooks);

    let native = read_json(
        &market
            .join("plugins/short")
            .join(".codex-plugin/plugin.json"),
    )
    .unwrap();
    assert_eq!(native["name"], "short");
    assert_eq!(native["skills"], "./skills/");
    assert_eq!(native["commands"], "./commands/");
    assert_eq!(native["mcpServers"], "./.mcp.json");
    assert!(native["hooks"]["hooks"]["UserPromptSubmit"].is_array());
    assert_eq!(native["version"], prepared[0].version);

    let catalogue = read_json(&market.join(".agents/plugins/marketplace.json")).unwrap();
    assert_eq!(catalogue["name"], "prometeu-dev");
    assert_eq!(catalogue["plugins"][0]["source"]["source"], "local");
    assert_eq!(catalogue["plugins"][0]["source"]["path"], "./plugins/short");

    // An adapter revision must regenerate older snapshots whose source hash and inline hook
    // envelope used the previous format.
    let staged = market.join("plugins/short");
    std::fs::write(
        staged.join(".codex-plugin/plugin.json"),
        r#"{"name":"short","hooks":{"UserPromptSubmit":[]}}"#,
    )
    .unwrap();
    std::fs::write(
        market.join("plugins/.short.source-hash"),
        fingerprint(&source).unwrap(),
    )
    .unwrap();
    host()
        .prepare_marketplace(
            &market,
            "prometeu-dev",
            &[plugin("short", &source.display().to_string())],
        )
        .unwrap();
    let migrated = read_json(&staged.join(".codex-plugin/plugin.json")).unwrap();
    assert!(migrated["hooks"]["hooks"]["UserPromptSubmit"].is_array());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn detects_hooks_that_must_start_enabled() {
    let root = std::env::temp_dir().join(format!(
        "prometeu-codex-hook-detect-{}",
        uuid::Uuid::new_v4()
    ));
    let inline = root.join("inline");
    let conventional = root.join("conventional");
    let declared_but_broken = root.join("declared-broken");
    let plain = root.join("plain");

    for plugin in [&inline, &conventional, &declared_but_broken, &plain] {
        std::fs::create_dir_all(plugin.join(".codex-plugin")).unwrap();
    }
    std::fs::write(
        inline.join(".codex-plugin/plugin.json"),
        r#"{"name":"inline","hooks":{"SessionStart":[{"hooks":[]}]}}"#,
    )
    .unwrap();
    std::fs::write(
        conventional.join(".codex-plugin/plugin.json"),
        r#"{"name":"conventional"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(conventional.join("hooks")).unwrap();
    std::fs::write(conventional.join("hooks/hooks.json"), r#"{"hooks":{}}"#).unwrap();
    std::fs::write(
        declared_but_broken.join(".codex-plugin/plugin.json"),
        r#"{"name":"declared-broken","hooks":"./missing.json"}"#,
    )
    .unwrap();
    std::fs::write(
        plain.join(".codex-plugin/plugin.json"),
        r#"{"name":"plain"}"#,
    )
    .unwrap();

    assert!(plugin_has_hooks(&inline));
    assert!(plugin_has_hooks(&conventional));
    assert!(plugin_has_hooks(&declared_but_broken));
    assert!(!plugin_has_hooks(&plain));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn freeform_claude_versions_do_not_break_the_codex_cache() {
    let root =
        std::env::temp_dir().join(format!("prometeu-plugin-version-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin/plugin.json"),
        r#"{"name":"x","version":"v-next"}"#,
    )
    .unwrap();
    assert_eq!(
        portable_version(&root, "0123456789abcdef0123456789abcdef"),
        "0.0.0+prometeu.0123456789abcdef"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn codex_home_belongs_to_the_workspace_not_the_working_directory() {
    assert_eq!(
        host().codex_workspace_home("workspace-a"),
        host().codex_workspace_home("workspace-a")
    );
    assert_ne!(
        host().codex_workspace_home("workspace-a"),
        host().codex_workspace_home("workspace-b")
    );
}

#[cfg(unix)]
#[test]
fn deleting_derived_home_does_not_follow_links_to_the_real_home() {
    let root = std::env::temp_dir().join(format!("prometeu-codex-remove-{}", uuid::Uuid::new_v4()));
    let homes = root.join("homes");
    let home = homes.join("workspace");
    let real = root.join("real");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("auth.json"), "account").unwrap();
    std::os::unix::fs::symlink(real.join("auth.json"), home.join("auth.json")).unwrap();

    remove_codex_home(&homes, &home);

    assert!(!home.exists());
    assert_eq!(
        std::fs::read_to_string(real.join("auth.json")).unwrap(),
        "account"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn derived_home_preserves_state_without_changing_global_configuration() {
    let root = std::env::temp_dir().join(format!("prometeu-codex-home-{}", uuid::Uuid::new_v4()));
    let base = root.join("base");
    let home = root.join("workspace");
    let marketplace = home.join("marketplace");
    std::fs::create_dir_all(base.join("plugins")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(base.join("auth.json"), "account").unwrap();
    let global = r#"
[projects."/tmp/project"]
trust_level = "trusted"

[plugins."global@other"]
enabled = true

[hooks.state.global]
trusted_hash = "sha256:global"
"#;
    std::fs::write(base.join("config.toml"), global).unwrap();
    let selected = format!("new@{}", "prometeu-dev");
    let old = format!("old@{}", "prometeu-dev");
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"
[hooks.state.workspace]
trusted_hash = "sha256:workspace"

[plugins."{old}"]
enabled = true
option = "retained"
"#
        ),
    )
    .unwrap();

    host()
        .prepare_codex_home(&base, &home, &marketplace, std::slice::from_ref(&selected))
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(base.join("config.toml")).unwrap(),
        global
    );
    let config = read_toml(&home.join("config.toml")).unwrap();
    assert_eq!(
        config["projects"]["/tmp/project"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(
        config["plugins"]["global@other"]["enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(config["cli_auth_credentials_store"].as_str(), Some("file"));
    assert_eq!(
        config["plugins"][&selected]["enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(config["plugins"][&old]["enabled"].as_bool(), Some(false));
    assert_eq!(config["plugins"][&old]["option"].as_str(), Some("retained"));
    assert_eq!(
        config["hooks"]["state"]["global"]["trusted_hash"].as_str(),
        Some("sha256:global")
    );
    assert_eq!(
        config["hooks"]["state"]["workspace"]["trusted_hash"].as_str(),
        Some("sha256:workspace")
    );
    assert_eq!(
        config["marketplaces"]["prometeu-dev"]["source"].as_str(),
        Some(marketplace.to_string_lossy().as_ref())
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_link(home.join("auth.json")).unwrap(),
        base.join("auth.json")
    );
    std::fs::remove_dir_all(root).ok();
}

struct Files;
impl PackageFiles for Files {
    fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
        std::fs::create_dir_all(path).map_err(|e| e.to_string())
    }
    fn write_private(&self, path: &Path, body: &str) -> Result<(), String> {
        self.ensure_private_dir(path.parent().unwrap())?;
        std::fs::write(path, body).map_err(|e| e.to_string())
    }
}
struct Catalog(Vec<Plugin>);
impl PackageCatalog for Catalog {
    fn load(&self) -> Vec<Plugin> {
        self.0.clone()
    }
}
#[derive(Default)]
struct Installer {
    calls: Mutex<Vec<String>>,
    current: Mutex<HashMap<String, InstalledPlugin>>,
    failure: Option<String>,
}
impl PackageInstaller for Installer {
    fn installed(&self, _: &Path) -> Result<HashMap<String, InstalledPlugin>, String> {
        self.calls.lock().unwrap().push("list".into());
        Ok(self.current.lock().unwrap().clone())
    }
    fn install(&self, home: &Path, canonical: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("add:{canonical}"));
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => {
                let id = canonical.split_once('@').unwrap().0;
                let manifest = read_json(
                    &home
                        .join("marketplace/plugins")
                        .join(id)
                        .join(".codex-plugin/plugin.json"),
                )
                .unwrap();
                self.current.lock().unwrap().insert(
                    canonical.into(),
                    InstalledPlugin {
                        version: manifest["version"].as_str().unwrap().into(),
                    },
                );
                Ok(())
            }
        }
    }
    fn remove(&self, _: &Path, canonical: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("remove:{canonical}"));
    }
}
fn host() -> NativePackages {
    NativePackages::new(
        "/unused/homes".into(),
        "/unused/user".into(),
        "prometeu-dev".into(),
        Arc::new(Catalog(vec![])),
        Arc::new(Files),
        Arc::new(Installer::default()),
        Arc::new(Mutex::new(())),
    )
}
fn plugin(id: &str, source: &str) -> Plugin {
    Plugin {
        id: id.into(),
        source: source.into(),
        note: String::new(),
        made: false,
        from: String::new(),
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("prometeu-packages-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn package(&self) -> Plugin {
        let source = self.0.join("source");
        Files
            .write_private(
                &source.join(".claude-plugin/plugin.json"),
                r#"{"name":"review","version":"1.0.0","hooks":"./hooks.json"}"#,
            )
            .unwrap();
        plugin("review", source.to_str().unwrap())
    }
    fn profile(&self) -> prometeu_profiles::Profile {
        prometeu_profiles::Profile {
            id: uuid::Uuid::new_v4().to_string(),
            provider: prometeu_core::board::ProviderId::Codex,
            managed: true,
            home: self.0.join("account"),
            revision: 0,
        }
    }
    fn host(&self, packages: Vec<Plugin>, installer: Arc<dyn PackageInstaller>) -> NativePackages {
        NativePackages::new(
            self.0.join("homes"),
            self.0.clone(),
            "prometeu-dev".into(),
            Arc::new(Catalog(packages)),
            Arc::new(Files),
            installer,
            Arc::new(Mutex::new(())),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn injected_installer_reuses_versions_and_reinstalls_changed_packages() {
    let fixture = Fixture::new();
    let package = fixture.package();
    let installer = Arc::new(Installer::default());
    let host = fixture.host(vec![package.clone()], installer.clone());
    let profile = fixture.profile();
    let selection = ["review".into(), "review".into(), "deleted".into()];
    let prepared = host.codex("workspace", Some(&selection), &profile).unwrap();
    assert_eq!(prepared.ids, ["review@prometeu-dev"]);
    assert_eq!(prepared.hook_ids, prepared.ids);
    assert_eq!(
        prepared.home,
        Some(host.codex_workspace_home("workspace").join(&profile.id))
    );
    host.codex("workspace", Some(&selection), &profile).unwrap();
    assert_eq!(
        *installer.calls.lock().unwrap(),
        ["list", "add:review@prometeu-dev", "list"]
    );
    std::fs::write(
        Path::new(&package.source).join("new-content.txt"),
        "updated",
    )
    .unwrap();
    host.codex("workspace", Some(&selection), &profile).unwrap();
    assert_eq!(
        *installer.calls.lock().unwrap(),
        [
            "list",
            "add:review@prometeu-dev",
            "list",
            "list",
            "add:review@prometeu-dev"
        ]
    );
    assert!(!Path::new(&package.source).join(".codex-plugin").exists());
}

#[test]
fn defaults_skip_all_dependencies_and_explicit_empty_selection_keeps_a_derived_home() {
    struct Unavailable;
    impl PackageCatalog for Unavailable {
        fn load(&self) -> Vec<Plugin> {
            panic!("defaults must not read the catalog")
        }
    }
    let fixture = Fixture::new();
    let installer = Arc::new(Installer::default());
    let mut host = fixture.host(vec![], installer.clone());
    host.catalog = Arc::new(Unavailable);
    let profile = fixture.profile();
    assert!(host.claude(None).is_empty());
    assert!(host
        .codex("workspace", None, &profile)
        .unwrap()
        .home
        .is_none());
    assert!(!host.root.exists());
    host.catalog = Arc::new(Catalog(vec![]));
    let explicit = host.codex("workspace", Some(&[]), &profile).unwrap();
    assert!(explicit.home.unwrap().join("config.toml").is_file());
    assert!(explicit.ids.is_empty());
    assert!(installer.calls.lock().unwrap().is_empty());
    assert_eq!(
        args_from(
            &[
                plugin("local", "~/package"),
                plugin("remote", "https://example.test/package.zip")
            ],
            &["local".into(), "remote".into(), "deleted".into()],
            &fixture.0
        ),
        [
            "--plugin-dir",
            fixture.0.join("package").to_str().unwrap(),
            "--plugin-url",
            "https://example.test/package.zip"
        ]
    );
}

#[test]
fn installation_failure_cleans_the_failed_id_and_preserves_the_original_error() {
    let fixture = Fixture::new();
    let installer = Arc::new(Installer {
        failure: Some("installation refused".into()),
        ..Installer::default()
    });
    let host = fixture.host(vec![fixture.package()], installer.clone());
    assert_eq!(
        host.codex("workspace", Some(&["review".into()]), &fixture.profile())
            .err(),
        Some("installation refused".into())
    );
    assert_eq!(
        *installer.calls.lock().unwrap(),
        [
            "list",
            "add:review@prometeu-dev",
            "remove:review@prometeu-dev"
        ]
    );
}

#[test]
fn private_write_failure_prevents_installation_and_keeps_error_context() {
    struct Denied;
    impl PackageFiles for Denied {
        fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
            Files.ensure_private_dir(path)
        }
        fn write_private(&self, _: &Path, _: &str) -> Result<(), String> {
            Err("denied".into())
        }
    }
    let fixture = Fixture::new();
    let installer = Arc::new(Installer::default());
    let mut host = fixture.host(vec![fixture.package()], installer.clone());
    host.files = Arc::new(Denied);
    assert_eq!(
        host.codex("workspace", Some(&["review".into()]), &fixture.profile())
            .err(),
        Some(prometeu_core::error::with_args(
            "err.plugin.codex.prepare",
            &[("cause", "denied".into())]
        ))
    );
    assert!(installer.calls.lock().unwrap().is_empty());
}

#[test]
fn catalog_retains_legacy_defaults_and_invalid_file_behavior() {
    let fixture = Fixture::new();
    let catalog = FilePackageCatalog {
        path: fixture.0.join("plugins.json"),
    };
    assert!(catalog.load().is_empty());
    std::fs::write(&catalog.path, "invalid").unwrap();
    assert!(catalog.load().is_empty());
    std::fs::write(
        &catalog.path,
        r#"[{"id":"review","source":"~/review","unknown":42}]"#,
    )
    .unwrap();
    let plugins = catalog.load();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "review");
    assert_eq!(plugins[0].source, "~/review");
    assert_eq!(plugins[0].note, "");
    assert!(!plugins[0].made);
    assert_eq!(plugins[0].from, "");
}

#[test]
fn shared_preparation_gate_blocks_writes_until_the_host_releases_it() {
    use std::sync::mpsc;
    use std::time::Duration;
    struct ObservedCatalog(Mutex<mpsc::Sender<()>>);
    impl PackageCatalog for ObservedCatalog {
        fn load(&self) -> Vec<Plugin> {
            self.0.lock().unwrap().send(()).unwrap();
            vec![]
        }
    }
    let fixture = Fixture::new();
    let (entered, observed) = mpsc::channel();
    let (done, completed) = mpsc::channel();
    let gate = Arc::new(Mutex::new(()));
    let guard = gate.lock().unwrap();
    let mut host = fixture.host(vec![], Arc::new(Installer::default()));
    host.catalog = Arc::new(ObservedCatalog(Mutex::new(entered)));
    host.gate = gate.clone();
    let root = host.root.clone();
    let profile = fixture.profile();
    let worker = std::thread::spawn(move || {
        let prepared = host.codex("workspace", Some(&[]), &profile).unwrap();
        done.send(prepared.home.unwrap()).unwrap();
    });
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        completed.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(!root.exists());
    drop(guard);
    assert!(completed
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .join("config.toml")
        .is_file());
    worker.join().unwrap();
}
