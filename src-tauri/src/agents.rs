//! Discover installed CLIs and their account-specific model catalogs. The UI shares one
//! conversation model; provider adapters translate process protocols into canonical events. Query
//! each CLI's own catalog so new models appear without an app release: Codex uses
//! app-server model/list, while Claude uses list_models.

use crate::state::ProviderId;
use serde_json::Value;
use std::path::PathBuf;

mod catalog;
pub use catalog::{CatalogError, ModelCatalog};

/// Discover both CLIs through one login shell so user-defined PATH locations are respected without
/// paying the shell startup cost twice.
fn installed() -> (bool, bool) {
    let out = std::process::Command::new("sh")
        .args([
            "-lc",
            "command -v claude && echo TEM_CLAUDE; command -v codex && echo TEM_CODEX; true",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    (out.contains("TEM_CLAUDE"), out.contains("TEM_CODEX"))
}

/// The catalog belongs to the selected account. Selection failures must not silently query the
/// terminal account.
fn home() -> Option<PathBuf> {
    crate::accounts::active(ProviderId::Codex)
        .ok()
        .map(|profile| profile.home)
}

pub use prometeu_core::agents::{AgentCapabilities, AgentDescriptor, Agents, AuthMethod, Model};

pub(crate) fn capabilities(id: ProviderId) -> AgentCapabilities {
    let common = AgentCapabilities {
        initial_plan_mode: false,
        workspace_mcp_selection: true,
        workspace_plugin_selection: true,
        resume: true,
        compact: true,
        context_report: true,
        approvals: true,
        user_questions: true,
        // The app injects local file paths into messages. Both runtimes can read the same worktree;
        // no provider upload or binary payload is involved.
        attachments: true,
    };
    match id {
        ProviderId::Claude => AgentCapabilities {
            initial_plan_mode: true,
            ..common
        },
        ProviderId::Codex => common,
        ProviderId::Antigravity | ProviderId::RetiredGemini => AgentCapabilities {
            initial_plan_mode: false,
            approvals: false,
            workspace_mcp_selection: false,
            workspace_plugin_selection: false,
            compact: false,
            context_report: false,
            user_questions: false,
            ..common
        },
    }
}

fn descriptor(id: ProviderId, installed: bool, models: Vec<Model>) -> AgentDescriptor {
    AgentDescriptor {
        id,
        label: match id {
            ProviderId::Claude => "Claude".into(),
            ProviderId::Codex => "Codex".into(),
            ProviderId::Antigravity => "Antigravity".into(),
            ProviderId::RetiredGemini => "Gemini CLI".into(),
        },
        installed,
        models,
        capabilities: capabilities(id),
        unavailable_reason: (id == ProviderId::Antigravity && !installed)
            .then(|| crate::i18n::t("err.antigravity.version")),
        account_notice: (id == ProviderId::Antigravity)
            .then(|| crate::i18n::t("account.external.notice")),
        auth_methods: match id {
            ProviderId::RetiredGemini => vec![],
            ProviderId::Antigravity => vec![AuthMethod {
                id: "external".into(),
                kind: "external".into(),
                label: crate::i18n::t("account.external.attach"),
            }],
            other => vec![AuthMethod {
                id: "browser".into(),
                kind: "browser".into(),
                label: if other == ProviderId::Claude {
                    "Claude"
                } else {
                    "Codex"
                }
                .into(),
            }],
        },
    }
}

/// Discover installations independently of account-specific model catalogs.
#[tauri::command]
pub fn agents() -> Agents {
    let (claude, codex) = installed();
    Agents {
        providers: vec![
            descriptor(
                ProviderId::Antigravity,
                crate::antigravity::installed(),
                vec![],
            ),
            descriptor(ProviderId::Claude, claude, vec![]),
            descriptor(ProviderId::Codex, codex, vec![]),
        ],
    }
}

/// Query the selected account without creating a conversation or performing inference.
#[tauri::command]
pub async fn agent_models(
    state: tauri::State<'_, crate::AppState>,
    agent: ProviderId,
) -> Result<ModelCatalog, CatalogError> {
    let launcher = state.query_launcher.clone();
    tauri::async_runtime::spawn_blocking(move || catalog::fetch(launcher.as_ref(), agent))
        .await
        .map_err(|_| CatalogError::new("failed"))?
}

/// Use the model with the largest priority value for cheap workspace naming; the catalog orders
/// flagship models before smaller ones. Without a catalog, fall back to the workspace model.
pub fn codex_namer_model() -> String {
    let Some(home) = home() else {
        return String::new();
    };
    let Ok(raw) = std::fs::read_to_string(home.join("models_cache.json")) else {
        return String::new();
    };
    let Ok(cache) = serde_json::from_str::<Value>(&raw) else {
        return String::new();
    };
    cache["models"]
        .as_array()
        .and_then(|models| {
            models
                .iter()
                .filter(|m| m["visibility"].as_str() == Some("list"))
                .max_by_key(|m| m["priority"].as_u64().unwrap_or(0))
                .and_then(|m| m["slug"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// Map the UI's top effort level ultracode to Codex's native ultra value.
pub fn effort(level: &str) -> &str {
    match level.trim() {
        "ultracode" => "ultra",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_models_publish_additional_visibility() {
        let models = catalog::parse_claude(&serde_json::json!({"response":{"subtype":"success","response":{"models":[{"value":"opus","displayName":"Opus"}]}}})).unwrap();
        assert_eq!(
            serde_json::to_value(&models[0]).unwrap()["additional"],
            false
        );
    }

    #[test]
    fn ultracode_maps_to_ultra() {
        assert_eq!(effort("ultracode"), "ultra");
        assert_eq!(effort("max"), "max");
    }

    // A shortened Claude 2.1.251 response excludes Default and disabled advertisements while
    // preserving usable models.
    #[test]
    fn reads_the_claude_catalog() {
        let line = r#"{"type":"control_response","response":{"subtype":"success","request_id":"models","response":{"models":[
            {"value":"default","resolvedModel":"claude-opus-5[1m]","displayName":"Default (recommended)","supportsEffort":true,"supportedEffortLevels":["low","medium","high","xhigh","max"]},
            {"value":"opus[1m]","resolvedModel":"claude-opus-5[1m]","displayName":"Opus (1M context)","supportsEffort":true,"supportedEffortLevels":["low","medium","high","xhigh","max"]},
            {"value":"haiku","resolvedModel":"claude-haiku-4-5","displayName":"Haiku"},
            {"value":"cc-update-required-1","resolvedModel":"cc-update-required-1","displayName":"Fable 5.1 (disabled)","disabled":true}
        ]}}}"#;
        let models = catalog::parse_claude(&serde_json::from_str(line).unwrap()).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "opus[1m]");
        assert_eq!(models[0].label, "Opus (1M context)");
        assert_eq!(models[0].efforts, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(models[1].id, "haiku");
        assert!(models[1].efforts.is_empty());
    }

    #[test]
    fn capabilities_belong_to_the_descriptor_not_the_view() {
        let claude = descriptor(ProviderId::Claude, true, vec![]);
        let codex = descriptor(ProviderId::Codex, true, vec![]);

        assert!(claude.capabilities.initial_plan_mode);
        assert!(claude.capabilities.workspace_plugin_selection);
        assert!(!codex.capabilities.initial_plan_mode);
        assert!(codex.capabilities.workspace_plugin_selection);
        assert!(codex.capabilities.workspace_mcp_selection);
        assert!(codex.capabilities.resume);

        let json = serde_json::to_value(codex).unwrap();
        assert_eq!(json["id"], "codex");
        assert_eq!(json["capabilities"]["initialPlanMode"], false);
        assert_eq!(json["capabilities"]["workspaceMcpSelection"], true);
    }
    #[test]
    fn antigravity_advertises_only_supported_controls_and_external_account() {
        let g = descriptor(ProviderId::Antigravity, false, vec![]);
        assert!(!g.capabilities.initial_plan_mode);
        assert!(g.capabilities.resume);
        assert!(!g.capabilities.approvals);
        assert!(!g.capabilities.compact);
        assert!(!g.capabilities.context_report);
        assert!(!g.capabilities.user_questions);
        assert!(!g.capabilities.workspace_mcp_selection);
        assert!(!g.capabilities.workspace_plugin_selection);
        let v = serde_json::to_value(g).unwrap();
        assert_eq!(v["authMethods"][0]["id"], "external");
        assert_eq!(v["authMethods"].as_array().unwrap().len(), 1);
        assert!(v["unavailableReason"]
            .as_str()
            .unwrap()
            .contains("err.antigravity.version"));
        assert_eq!(
            serde_json::to_value(descriptor(ProviderId::Claude, true, vec![])).unwrap()
                ["authMethods"][0]["id"],
            "browser"
        );
    }
}
