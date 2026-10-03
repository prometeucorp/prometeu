//! Shared catalog response shapes. A disconnected catalog does not hide or erase local hubs.
use crate::skills;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct Portable {
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Serialize)]
pub struct CatalogPlugin {
    #[serde(flatten)]
    pub item: Portable,
    pub local_id: String,
    pub installed: bool,
    pub source_changed: bool,
}
#[derive(Serialize)]
pub struct CatalogSkill {
    #[serde(flatten)]
    pub item: skills::Skill,
    pub local_id: String,
    pub installed: bool,
}
#[derive(Serialize, Default)]
pub struct CatalogState {
    pub connected: bool,
    pub revision: Option<u64>,
    pub plugins: Vec<CatalogPlugin>,
    pub projects: Vec<CatalogProject>,
    pub mcp: Vec<String>,
    pub skills: Vec<CatalogSkill>,
    pub shared: BTreeMap<String, String>,
    pub organization_items: Vec<OrganizationItem>,
}

#[derive(Serialize)]
pub struct OrganizationItem {
    pub organization: String,
    pub organization_name: String,
    pub revision: Option<u64>,
    pub kind: String,
    pub id: String,
    pub description: String,
    pub installed: bool,
    // The installed local item that satisfies this definition, linked or equivalent.
    pub local_id: Option<String>,
}

#[derive(Serialize)]
pub struct CatalogProject {
    #[serde(flatten)]
    pub item: Portable,
    pub organization: Option<String>,
    pub organization_name: Option<String>,
    pub revision: Option<u64>,
    pub local_path: Option<String>,
}

pub trait CatalogSource: Send + Sync {
    fn state(&self) -> Result<CatalogState, String>;
}
/// Explicit local-only composition. Cloud publication is not an available service here.
pub struct LocalCatalog;
impl CatalogSource for LocalCatalog {
    fn state(&self) -> Result<CatalogState, String> {
        Ok(CatalogState::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_state_keeps_the_existing_desktop_wire_shape() {
        assert_eq!(
            serde_json::to_value(LocalCatalog.state().unwrap()).unwrap(),
            serde_json::json!({
                "connected":false,"revision":null,"plugins":[],"projects":[],"mcp":[],"skills":[],"shared":{},"organization_items":[]
            })
        );
        let skill = CatalogSkill {
            item: skills::Skill {
                id: "review".into(),
                description: "Review code".into(),
                content: "Read the diff.".into(),
            },
            local_id: "local-review".into(),
            installed: true,
        };
        assert_eq!(
            serde_json::to_value(skill).unwrap(),
            serde_json::json!({"id":"review","description":"Review code","content":"Read the diff.","local_id":"local-review","installed":true})
        );
    }
}
