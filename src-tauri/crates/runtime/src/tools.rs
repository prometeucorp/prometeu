//! WSL composition of existing tool resolution, package preparation and MCP encoding.
use prometeu_core::{
    board::{Board, Workspace},
    selection::Tools,
    tool_resolution::{self as resolution, *},
    workspaces::CatalogStore,
};
use prometeu_tools::{
    mcp::{McpCatalog, McpFiles, McpSources, Server},
    packages::PackageCatalog,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct Selection {
    pub catalog: Arc<dyn CatalogStore>,
    pub settings: Arc<dyn prometeu_files::settings::RepositorySettings>,
    pub mcp: Arc<dyn McpCatalog>,
    pub packages: Arc<dyn PackageCatalog>,
}
impl Selection {
    pub fn declaration(&self, board: &Board, id: &str) -> Option<ProjectDeclaration> {
        let (worktree, repo) = board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| {
                let primary = w.primary();
                (primary.worktree, primary.path)
            })
            .or_else(|| {
                board
                    .projects
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| (p.path.clone(), p.path.clone()))
            })?;
        prometeu_files::tool_declarations::project_declaration(
            self.settings.as_ref(),
            Path::new(&worktree),
            Path::new(&repo),
        )
    }
    pub fn project(&self, board: &Board, id: &str) -> ProjectTools {
        match self.declaration(board, id) {
            Some(d) => ProjectTools {
                pending: !decided(&board.tool_trust, &d.repo, &d.hash),
                decision: board.tool_trust.iter().find(|t| t.repo == d.repo).cloned(),
                repo: d.repo,
                hash: d.hash,
                file: d.file,
                tools: d.tools,
            },
            None => ProjectTools {
                repo: String::new(),
                hash: String::new(),
                file: None,
                tools: Tools::default(),
                pending: false,
                decision: None,
            },
        }
    }
    fn hubs(&self) -> (Vec<String>, Vec<String>) {
        (
            self.mcp.load().into_iter().map(|s| s.id).collect(),
            self.packages.load().into_iter().map(|p| p.id).collect(),
        )
    }
    pub fn effective(&self, board: &Board, workspace: &Workspace) -> WorkspaceTools {
        let (mcp, packages) = self.hubs();
        let (project, gate) = self
            .declaration(board, &workspace.id)
            .map(|d| {
                let gate = gate_of(&board.tool_trust, &d.repo, &d.hash);
                (d.tools, gate)
            })
            .unwrap_or((Tools::default(), Gate::Trusted));
        WorkspaceTools {
            mcp: axis_provenance(
                &board.tools.mcp,
                &project.mcp,
                gate,
                &workspace.mcp,
                &[],
                &mcp,
            ),
            plugins: axis_provenance(
                &board.tools.plugins,
                &project.plugins,
                gate,
                &workspace.plugins,
                &[],
                &packages,
            ),
            skills: axis_provenance(
                &board.tools.skills,
                &project.skills,
                gate,
                &workspace.skills,
                &[],
                &packages,
            ),
        }
    }
}
impl ToolSelection for Selection {
    fn resolve(&self, workspace: &str) -> Result<ResolvedTools, String> {
        let board = self
            .catalog
            .load()?
            .ok_or("workspace_catalog_invalid")?
            .board;
        let Some(ws) = board.workspaces.iter().find(|w| w.id == workspace) else {
            // An application bootstrap owns a diagnostic context without registering a workspace.
            return Ok(ResolvedTools::default());
        };
        let project = self
            .declaration(&board, workspace)
            .filter(|d| approved(&board.tool_trust, &d.repo, &d.hash))
            .map(|d| d.tools)
            .unwrap_or_default();
        let (mcp, packages) = self.hubs();
        Ok(resolution::resolve_tools(
            &board.tools,
            &project,
            &ws.tools(),
            &[],
            &mcp,
            &packages,
        ))
    }
}
struct Sources(
    Arc<dyn McpCatalog>,
    Arc<dyn prometeu_tools::mcp_auth::Authorization>,
);
impl McpSources for Sources {
    fn claude_servers(&self, _: &str, _: &[String], _: &Path) -> Result<Vec<Server>, String> {
        Ok(self.0.load())
    }
    fn codex_servers(&self, _: &str, _: &[String]) -> Result<Vec<Server>, String> {
        Ok(self.0.load())
    }
    fn bearer(&self, id: &str) -> Option<String> {
        self.1.bearer(id)
    }
}
struct Files(PathBuf);
impl Files {
    fn write(&self, name: &str, body: &str) -> Result<PathBuf, String> {
        let path = self.0.join(name);
        prometeu_files::private::write_private(&path, body).map_err(|cause| {
            prometeu_core::error::with_args("err.mcp.session", &[("cause", cause)])
        })?;
        Ok(path)
    }
}
impl McpFiles for Files {
    fn claude_config(&self, session: &str, body: &str) -> Result<PathBuf, String> {
        self.write(&format!("{session}.json"), body)
    }
    fn codex_environment(
        &self,
        session: &str,
        server: &str,
        body: &str,
    ) -> Result<PathBuf, String> {
        self.write(
            &format!("{session}.{}.env", prometeu_tools::mcp::slug(server)),
            body,
        )
    }
}
pub fn native(
    root: &Path,
    executable: &Path,
    resources: &crate::resources::Resources,
) -> Arc<dyn prometeu_tools::StartupTools> {
    let namespace = "prometeu";
    Arc::new(prometeu_tools::NativeTools {
        packages: Arc::new(prometeu_tools::packages::NativePackages::new(
            root.join("codex-workspaces"),
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default(),
            namespace.into(),
            resources.packages.clone(),
            Arc::new(crate::resources::PrivateFiles),
            Arc::new(prometeu_tools::package_installer::CodexInstaller {
                executable: executable.into(),
                marketplace: namespace.into(),
            }),
            Arc::new(std::sync::Mutex::new(())),
        )),
        sources: Arc::new(Sources(resources.mcp.clone(), resources.auth.clone())),
        files: Arc::new(Files(root.join("mcp"))),
    })
}
pub fn profile() -> prometeu_profiles::Profile {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".codex")
        });
    prometeu_profiles::Profile {
        id: "codex".into(),
        provider: prometeu_core::board::ProviderId::Codex,
        home,
        managed: false,
        revision: 0,
    }
}
