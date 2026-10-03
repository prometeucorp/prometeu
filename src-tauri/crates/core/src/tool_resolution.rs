//! Shared launch resolution and picker provenance; catalogs and repository reads stay injected.
use crate::{
    board::ToolTrust,
    selection::{Selection, Tools},
};
/// The project `[tools]` declaration and its trust state, for the interface (ADR 0045). `hash` and
/// `repo` are empty and `tools` inherits when the primary repository declares nothing.
#[derive(serde::Serialize)]
pub struct ProjectTools {
    /// Repository identity that keys the trust decision: `origin` URL or absolute clone path.
    pub repo: String,
    /// The settings file that declared the layer, if any.
    pub file: Option<String>,
    /// SHA-256 of the declared section; empty when the repository declares nothing.
    pub hash: String,
    /// The declared layer; every axis inherits when nothing is declared.
    pub tools: Tools,
    /// True when a declaration exists whose current hash has no decision, so the interface prompts.
    pub pending: bool,
    /// The stored decision for this repository, if any.
    pub decision: Option<ToolTrust>,
}

/// The effective tool selection of one workspace, per axis, each item labeled with where it came
/// from, so the picker shows the resolved result without reading the three layers (ADR 0045).
#[derive(serde::Serialize)]
pub struct WorkspaceTools {
    pub mcp: Vec<EffectiveItem>,
    pub plugins: Vec<EffectiveItem>,
    pub skills: Vec<EffectiveItem>,
}

/// The hub IDs a session injects, one axis at a time, after composing the layers. `None` on an axis
/// means no layer declared it, so the provider keeps its own configuration; `Some` is the resolved
/// set to materialize (possibly empty, which injects nothing from the hub).
#[derive(Default, Debug, PartialEq, Eq)]
pub struct ResolvedTools {
    pub mcp: Option<Vec<String>>,
    pub mcp_inherits_base: bool,
    pub plugins: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
}

/// Resolve one axis. When every layer inherits, the axis stays `None` so the provider's own
/// configuration is preserved; otherwise the composed set — over the CLI-inherited base, when one
/// applies — is what the session injects.
pub fn resolve_axis(
    global: &Option<Selection>,
    project: &Option<Selection>,
    workspace: &Option<Selection>,
    base: &[String],
    universe: &[String],
) -> Option<Vec<String>> {
    if global.is_none() && project.is_none() && workspace.is_none() {
        None
    } else {
        Some(crate::selection::resolve_with_base(
            base, global, project, workspace, universe,
        ))
    }
}

/// Compose the three layers into the IDs a launch injects, keeping only IDs the universe still has.
/// The hubs, the mcp inherited base (ADR 0046) and the project layer come from the caller so the
/// chain stays testable without disk. The project layer must already be gated on trust before it
/// reaches here (ADR 0045).
pub fn resolve_tools(
    global: &Tools,
    project: &Tools,
    workspace: &Tools,
    mcp_base: &[String],
    mcp_universe: &[String],
    plugin_hub: &[String],
) -> ResolvedTools {
    let mcp_layers = [
        global.mcp.as_ref(),
        project.mcp.as_ref(),
        workspace.mcp.as_ref(),
    ];
    ResolvedTools {
        mcp: resolve_axis(
            &global.mcp,
            &project.mcp,
            &workspace.mcp,
            mcp_base,
            mcp_universe,
        ),
        mcp_inherits_base: mcp_layers.iter().any(Option::is_some)
            && mcp_layers
                .iter()
                .flatten()
                .all(|selection| selection.base != crate::selection::Base::None),
        plugins: resolve_axis(
            &global.plugins,
            &project.plugins,
            &workspace.plugins,
            &[],
            plugin_hub,
        ),
        // Standalone skills ride the plugin hub as `skill-<id>`, so the skills axis filters too.
        skills: resolve_axis(
            &global.skills,
            &project.skills,
            &workspace.skills,
            &[],
            plugin_hub,
        ),
    }
}

/// The project `[tools]` declaration of a repository, with the identity and hash that key its trust
/// decision (ADR 0045). `project_declaration` returns `None` when the repository declares nothing,
/// so an absent layer needs no approval and simply inherits.
#[derive(Debug, PartialEq, Eq)]
pub struct ProjectDeclaration {
    /// Repository identity: the `origin` remote URL when one exists, else the clone's absolute path.
    pub repo: String,
    /// SHA-256 of the canonical form of the declared `[tools]` section.
    pub hash: String,
    /// The settings file that declared it, surfaced by the interface.
    pub file: Option<String>,
    /// The declared layer.
    pub tools: Tools,
}

/// True when the person approved this exact declaration for this repository.
pub fn approved(trust: &[ToolTrust], repo: &str, hash: &str) -> bool {
    trust
        .iter()
        .any(|t| t.repo == repo && t.hash == hash && t.approved)
}

/// True when a decision (approval or rejection) exists for this exact declaration; a rejection
/// quiets the prompt until the hash changes (ADR 0045).
pub fn decided(trust: &[ToolTrust], repo: &str, hash: &str) -> bool {
    trust.iter().any(|t| t.repo == repo && t.hash == hash)
}

/// Whether a declared project layer may activate, for provenance labeling.
#[derive(Clone, Copy, PartialEq)]
pub enum Gate {
    Trusted,
    /// Nobody decided on the current hash yet, so the interface prompts.
    Pending,
    /// The current hash was explicitly rejected; resolved yet not injected, and no prompt.
    Rejected,
}

pub fn gate_of(trust: &[ToolTrust], repo: &str, hash: &str) -> Gate {
    match trust.iter().find(|t| t.repo == repo && t.hash == hash) {
        Some(decision) if decision.approved => Gate::Trusted,
        Some(_) => Gate::Rejected,
        None => Gate::Pending,
    }
}

/// Where one effective item came from in the chain, so the picker shows the result without
/// opening each layer (ADR 0045). `Removed` and `Pending` items are listed but not injected.
#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    /// Active because a layer above the workspace selected it.
    Inherited,
    /// Active because the workspace layer added it.
    Added,
    /// Inactive because the workspace layer removed an otherwise-inherited item.
    Removed,
    /// Declared by the project but not yet trusted, so resolved yet not injected.
    Pending,
    /// Declared by the project and explicitly rejected for its current hash; resolved yet not
    /// injected, and the interface stops prompting until the declaration changes.
    Rejected,
    /// Active because the person's CLI configuration loads it, not a Prometeu hub choice; the
    /// visible inherited base of the mcp axis (ADR 0046).
    Cli,
}

/// One item of the axis universe with its provenance.
#[derive(serde::Serialize)]
pub struct EffectiveItem {
    pub id: String,
    pub provenance: Provenance,
}

/// Classify every id of the axis universe for the picker. `project` is the declared layer and
/// `gate` says whether it may activate; while gated, the items it declares appear as `Pending` or
/// `Rejected`. The workspace layer is the person's own action, so an item it adds is active even
/// while the project declaration is still gated. `base` holds the CLI-inherited ids (ADR 0046):
/// an active one is labeled `Cli`, and the workspace can drop it with a removal like any inherited
/// item. Items with no story (off and untouched) are omitted.
pub fn axis_provenance(
    global: &Option<Selection>,
    project: &Option<Selection>,
    gate: Gate,
    workspace: &Option<Selection>,
    base: &[String],
    universe: &[String],
) -> Vec<EffectiveItem> {
    let gated = (gate == Gate::Trusted).then(|| project.clone()).flatten();
    let effective = crate::selection::resolve_with_base(base, global, &gated, workspace, universe);
    // What the project layer alone would contribute, to label gated items.
    let declared = crate::selection::resolve(&None, project, &None, universe);
    universe
        .iter()
        .filter_map(|id| {
            let on = effective.contains(id);
            let added = workspace.as_ref().is_some_and(|s| s.add.contains(id));
            let removed = workspace.as_ref().is_some_and(|s| s.remove.contains(id));
            let provenance = if on && added {
                Provenance::Added
            } else if on && base.contains(id) {
                Provenance::Cli
            } else if on {
                Provenance::Inherited
            } else if removed {
                Provenance::Removed
            } else if gate != Gate::Trusted && declared.contains(id) {
                match gate {
                    Gate::Rejected => Provenance::Rejected,
                    _ => Provenance::Pending,
                }
            } else {
                return None;
            };
            Some(EffectiveItem {
                id: id.clone(),
                provenance,
            })
        })
        .collect()
}

impl ResolvedTools {
    /// Plugins and standalone skills share the plugin-package pipeline, so a spawn materializes the
    /// two resolved axes together. `None` on both preserves the CLI's own plugins; otherwise the
    /// selected packages are the union, in plugin-then-skill order.
    pub fn plugin_packages(&self) -> Option<Vec<String>> {
        match (&self.plugins, &self.skills) {
            (None, None) => None,
            (plugins, skills) => {
                let mut merged = plugins.clone().unwrap_or_default();
                if let Some(skills) = skills {
                    merged.extend(skills.iter().cloned());
                }
                Some(merged)
            }
        }
    }
}

pub trait ToolSelection: Send + Sync {
    fn resolve(&self, workspace: &str) -> Result<ResolvedTools, String>;
}

pub fn record_trust(
    trust: &mut Vec<ToolTrust>,
    declaration: ProjectDeclaration,
    hash: &str,
    approved: bool,
    at: u64,
) -> Result<(), String> {
    if declaration.hash != hash {
        return Err(crate::error::code("err.tools.changed"));
    }
    let decision = ToolTrust {
        repo: declaration.repo,
        hash: declaration.hash,
        approved,
        at,
    };
    match trust.iter_mut().find(|t| t.repo == decision.repo) {
        Some(existing) => *existing = decision,
        None => trust.push(decision),
    }
    Ok(())
}
