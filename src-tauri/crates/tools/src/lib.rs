//! Native Unix tool artifacts and startup ports, independent of desktop composition.
#![cfg(unix)]
use prometeu_profiles::Profile;
use std::path::{Path, PathBuf};

pub mod catalog;
pub mod mcp;
pub mod mcp_auth;
pub mod mcp_discovery;
pub mod mcp_probe;
pub mod package_installer;
pub mod packages;
pub mod plugins;
pub mod skills;

pub struct ClaudeTools {
    pub mcp_config: Option<PathBuf>,
    pub plugin_args: Vec<String>,
}

/// Selected canonical IDs and the subset declaring hooks. Only those IDs may be trusted
/// during the handshake; the derived home remains local to the execution host.
pub struct CodexPlugins {
    pub home: Option<PathBuf>,
    pub ids: Vec<String>,
    pub hook_ids: Vec<String>,
}

/// Provider startup receives materialized artifacts without accessing desktop catalogs.
/// None preserves CLI defaults; Some(empty) is an explicit empty selection.
/// Packages contain the resolved plugin and standalone-skill selections together.
pub trait StartupTools: Send + Sync {
    fn claude(
        &self,
        session: &str,
        workdir: &Path,
        mcp: Option<&[String]>,
        packages: Option<&[String]>,
    ) -> Result<ClaudeTools, String>;
    fn codex_plugins(
        &self,
        scope: &str,
        packages: Option<&[String]>,
        profile: &Profile,
    ) -> Result<CodexPlugins, String>;
    fn codex_mcp(
        &self,
        session: &str,
        chosen: Option<&[String]>,
    ) -> Result<Option<mcp::CodexMcp>, String>;
}

/// Both execution hosts compose the same startup implementation with their catalogs and files.
pub struct NativeTools {
    pub packages: std::sync::Arc<dyn packages::PackageBackend>,
    pub sources: std::sync::Arc<dyn mcp::McpSources>,
    pub files: std::sync::Arc<dyn mcp::McpFiles>,
}
impl StartupTools for NativeTools {
    fn claude(
        &self,
        session: &str,
        workdir: &Path,
        mcp: Option<&[String]>,
        packages: Option<&[String]>,
    ) -> Result<ClaudeTools, String> {
        Ok(ClaudeTools {
            mcp_config: mcp::McpMaterializer {
                sources: self.sources.as_ref(),
                files: self.files.as_ref(),
            }
            .claude_config(session, mcp, workdir)?,
            plugin_args: self.packages.claude(packages),
        })
    }
    fn codex_plugins(
        &self,
        scope: &str,
        packages: Option<&[String]>,
        profile: &Profile,
    ) -> Result<CodexPlugins, String> {
        self.packages.codex(scope, packages, profile)
    }
    fn codex_mcp(
        &self,
        session: &str,
        chosen: Option<&[String]>,
    ) -> Result<Option<mcp::CodexMcp>, String> {
        mcp::McpMaterializer {
            sources: self.sources.as_ref(),
            files: self.files.as_ref(),
        }
        .codex_config(session, chosen)
    }
}

#[cfg(test)]
mod tests;
