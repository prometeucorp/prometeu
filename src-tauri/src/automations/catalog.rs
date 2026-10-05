//! Shared declarative catalog also consumed by the browser development backend.
use prometeu_core::automation::{OperationDescriptor, Workflow};
use serde::Deserialize;

#[derive(Deserialize)]
struct Catalog {
    registry: Vec<OperationDescriptor>,
    templates: Vec<Workflow>,
}
fn catalog() -> Catalog {
    serde_json::from_str(include_str!("../../../src/automation-catalog.json"))
        .expect("static automation catalog")
}
pub(super) fn registry() -> Vec<OperationDescriptor> {
    catalog().registry
}
pub(super) fn templates() -> Vec<Workflow> {
    catalog().templates
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_templates_are_valid_disabled_definitions_without_write_grants() {
        let catalog = catalog();
        for workflow in catalog.templates {
            assert!(!workflow.enabled);
            assert!(!workflow.policy.allow_writes);
            assert!(!workflow.policy.allow_commit);
            assert!(!workflow.policy.allow_push);
            assert!(workflow.policy.require_publish_approval);
            assert!(workflow.policy.require_merge_approval);
            let issues = prometeu_core::automation::validate_workflow(&workflow, &catalog.registry);
            assert!(issues.is_empty(), "{}: {issues:?}", workflow.id);
        }
    }

    #[test]
    fn incomplete_diff_never_reaches_risk_inference_or_merge() {
        use prometeu_core::automation::{simulate, SimulationFixture, SimulationStatus};
        let catalog = catalog();
        let workflow = catalog
            .templates
            .iter()
            .find(|w| w.id == "template-pr-risk")
            .unwrap();
        let fixture = SimulationFixture {
            event: serde_json::json!({"number": 12, "headSha": "abc"}),
            outputs: [(
                "status".into(),
                serde_json::json!({"riskContext":{"complete":false}}),
            )]
            .into(),
        };
        let result = simulate(workflow, &catalog.registry, &fixture).unwrap();
        for id in ["classify", "merge"] {
            assert_eq!(
                result
                    .steps
                    .iter()
                    .find(|s| s.node_id == id)
                    .unwrap()
                    .status,
                SimulationStatus::Skipped
            );
        }
        assert!(result
            .steps
            .iter()
            .any(|s| s.node_id == "manual" && s.status != SimulationStatus::Skipped));
    }
}
