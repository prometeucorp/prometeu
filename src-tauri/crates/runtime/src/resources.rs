//! Execution-side composition of local resource libraries. No Cloud publication is implied.
use prometeu_tools::{
    mcp::{LocalMcpLibrary, McpLibrary},
    packages::{self, FilePackageCatalog, PackageCatalog, PackageFiles, Plugin},
    skills::{SkillLibrary, SkillPackages, Skills},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct Resources {
    pub auth: Arc<dyn prometeu_tools::mcp_auth::Authorization>,
    pub mcp_jobs: crate::mcp::Jobs,
    pub catalog: Arc<dyn prometeu_tools::catalog::CatalogSource>,
    pub skills: Arc<dyn Skills>,
    pub packages: Arc<dyn PackageCatalog>,
    pub mcp_discovery: PathBuf,
    pub mcp: Arc<dyn McpLibrary>,
    pub plugins: Arc<dyn prometeu_tools::plugins::Plugins>,
}
pub(crate) struct PrivateFiles;
impl PackageFiles for PrivateFiles {
    fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
        prometeu_files::private::ensure_private_dir(path)
    }
    fn write_private(&self, path: &Path, body: &str) -> Result<(), String> {
        prometeu_files::private::write_private(path, body)
    }
}
/// Skill removal unregisters its generated package; it never deletes package or user files.
struct Packages {
    root: PathBuf,
    files: Arc<dyn PackageFiles>,
    catalog: Arc<dyn PackageCatalog>,
}
impl Packages {
    fn store(&self, hub: &[Plugin]) -> Result<(), String> {
        self.files
            .write_private(
                &self.root.join("plugins.json"),
                &serde_json::to_string_pretty(hub).map_err(|e| e.to_string())?,
            )
            .map_err(|cause| {
                prometeu_core::error::with_args("err.plugin.save", &[("cause", cause)])
            })
    }
}
impl SkillPackages for Packages {
    fn load(&self) -> Vec<Plugin> {
        self.catalog.load()
    }
    fn save(&self, plugin: Plugin) -> Result<(), String> {
        let mut hub = self.load();
        packages::register(&mut hub, plugin);
        self.store(&hub)
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        let mut hub = self.load();
        hub.retain(|entry| entry.id != id);
        self.store(&hub)
    }
}
pub fn native(root: &Path) -> Resources {
    let files: Arc<dyn PackageFiles> = Arc::new(PrivateFiles);
    let packages: Arc<dyn PackageCatalog> = Arc::new(FilePackageCatalog {
        path: root.join("plugins.json"),
    });
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    Resources {
        mcp_discovery: prometeu_profiles::claude::config_file(&claude_home),
        auth: Arc::new(prometeu_tools::mcp_auth::McpAuthorization::new(Arc::new(
            prometeu_tools::mcp_auth::PrivateAuthStorage {
                path: root.join("mcp-auth.json"),
                files: files.clone(),
            },
        ))),
        mcp_jobs: crate::mcp::Jobs::default(),
        plugins: Arc::new(prometeu_tools::plugins::PluginLibrary {
            root: root.into(),
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default(),
            catalog: packages.clone(),
            files: files.clone(),
            runner: Arc::new(prometeu_process::command::UnixCommandRunner),
        }),
        catalog: Arc::new(prometeu_tools::catalog::LocalCatalog),
        skills: Arc::new(SkillLibrary {
            root: root.into(),
            files: files.clone(),
            packages: Arc::new(Packages {
                root: root.into(),
                files: files.clone(),
                catalog: packages.clone(),
            }),
        }),
        packages,
        mcp: Arc::new(LocalMcpLibrary {
            path: root.join("mcp.json"),
            files,
            reserved: vec!["prometeu".into()],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_tools::skills::Skill;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn local_resources_reopen_existing_formats_and_preserve_files_on_removal() {
        let root = std::env::temp_dir().join(format!("resources-{}", uuid::Uuid::new_v4()));
        let files = PrivateFiles;
        files.ensure_private_dir(&root).unwrap();
        let other = serde_json::json!({"id":"other", "source":"/user/other", "note":"Unrelated package", "made":false,"from":""});
        files
            .write_private(
                &root.join("plugins.json"),
                &serde_json::json!([other]).to_string(),
            )
            .unwrap();
        let server = serde_json::json!({"id":"docs", "config":{"url":"https://example.invalid/mcp"}, "note":"Read only"});
        files
            .write_private(
                &root.join("mcp.json"),
                &serde_json::json!([server]).to_string(),
            )
            .unwrap();
        let resources = native(&root);
        assert_eq!(
            serde_json::to_value(resources.mcp.load()).unwrap(),
            serde_json::json!([server])
        );
        let edited_server = serde_json::json!({"id":" docs ", "config":{"command":"/usr/bin/env"}, "note":"Updated"});
        resources
            .mcp
            .save(serde_json::from_value(edited_server).unwrap())
            .unwrap();
        assert_eq!(native(&root).mcp.load()[0].note, "Updated");
        assert_eq!(
            std::fs::metadata(root.join("mcp.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(resources
            .mcp
            .save(
                serde_json::from_value(serde_json::json!({"id":"docs","config":[],"note":""}))
                    .unwrap()
            )
            .is_err());
        assert!(resources.mcp.remove("prometeu").is_err());
        assert_eq!(resources.mcp.load().len(), 1);
        resources.mcp.remove("docs").unwrap();
        assert!(native(&root).mcp.load().is_empty());
        let skill = Skill {
            id: "review".into(),
            description: "Before publishing".into(),
            content: "Read the diff.".into(),
        };
        assert!(resources
            .skills
            .save(Skill {
                id: "../escape".into(),
                ..skill.clone()
            })
            .is_err());
        assert!(!root.join("skills.json").exists());
        resources.skills.save(skill.clone()).unwrap();
        let package = root.join("skills-packages/review");
        let body = package.join("skills/review/SKILL.md");
        for path in [
            root.join("skills.json"),
            root.join("plugins.json"),
            body.clone(),
            package.join(".codex-plugin/plugin.json"),
            package.join(".claude-plugin/plugin.json"),
        ] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(native(&root).skills.load()[0].content, skill.content);
        assert_eq!(native(&root).packages.load().len(), 2);
        let edited = Skill {
            content: "Read the diff and run tests.".into(),
            ..skill
        };
        resources.skills.save(edited.clone()).unwrap();
        assert_eq!(native(&root).skills.load().len(), 1);
        assert!(std::fs::read_to_string(&body)
            .unwrap()
            .ends_with("Read the diff and run tests.\n"));
        resources.skills.remove(&edited.id).unwrap();
        assert!(native(&root).skills.load().is_empty());
        assert_eq!(
            serde_json::to_value(native(&root).packages.load()).unwrap(),
            serde_json::json!([other])
        );
        assert!(body.exists());
        let occupied =
            serde_json::json!({"id":"skill-review", "source":"/user/package", "made":false});
        files
            .write_private(
                &root.join("plugins.json"),
                &serde_json::json!([occupied]).to_string(),
            )
            .unwrap();
        assert!(resources.skills.save(edited).is_err());
        assert!(native(&root).skills.load().is_empty());
        assert_eq!(native(&root).packages.load()[0].source, "/user/package");
        std::fs::remove_dir_all(root).unwrap();
    }
}
