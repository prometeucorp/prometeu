//! Rust-generated wire payload checked by the TypeScript contract suite.
use prometeu_core::automation::{
    simulate, AutomationState, OperationDescriptor, RunStatus, SimulationFixture, Workflow,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[test]
fn automation_wire_fixture_matches_actual_serialization() {
    let registry: Vec<OperationDescriptor> = serde_json::from_value(json!([
        {"id":"fixture.inspect","version":1,"kind":"query","title":"Inspect","requiredInputs":["repository"],"inputSchema":{"repository":"string"},"outputPorts":["next","error"],"effect":"read"},
        {"id":"fixture.classify","version":1,"kind":"jev","title":"Classify","requiredInputs":[],"inputSchema":{},"outputPorts":["next","uncertain","error"],"effect":"read"},
        {"id":"fixture.merge","version":1,"kind":"action","title":"Merge","requiredInputs":[],"inputSchema":{},"outputPorts":["next","error"],"effect":"merge"}
    ])).unwrap();
    let configs = [
        json!({"type":"trigger","event":"manual","intervalSeconds":300,"baseline":"ignoreExisting"}),
        json!({"type":"query","operation":"fixture.inspect","version":1,"inputs":{"repository":{"$ref":"event.repository"}}}),
        json!({"type":"condition","path":"nodes.query.ready","operator":"equals","value":true}),
        json!({"type":"jev","operation":"fixture.classify","version":1,"inputs":{}}),
        json!({"type":"agent","prompt":"Inspect within the selected scope","provider":"claude","context":{"issue":{"$ref":"event.issue"}},"tools":["read_file","list_files","run_checks"],"checks":[{"executable":"cargo","args":["test","--locked"]}],"outputSchema":{"type":"object","properties":{"result":{"type":"string","enum":["done","blocked"]}},"required":["result"],"additionalProperties":false}}),
        json!({"type":"approval","message":"Approve the frozen result"}),
        json!({"type":"wait","seconds":60}),
        json!({"type":"action","operation":"fixture.merge","version":1,"inputs":{}}),
    ];
    let ids = [
        "trigger",
        "query",
        "condition",
        "jev",
        "agent",
        "approval",
        "wait",
        "action",
    ];
    let nodes: Vec<Value> = ids.iter().zip(configs).enumerate().map(|(index,(id,config))| json!({"id":id,"label":id,"position":{"x":120.0,"y":index as f64 * 100.0},"config":config})).collect();
    let edges: Vec<Value> = ids.windows(2).map(|pair| json!({"from":pair[0],"to":pair[1],"port":if pair[0] == "condition" {"true"} else {"next"}})).collect();
    let definition: Workflow = serde_json::from_value(json!({
        "id":"fixture-workflow","name":"Fixture workflow","revision":0,"enabled":true,
        "nodes":nodes,"edges":edges,
        "policy":{"maxConcurrentRuns":1,"requireMergeApproval":true,"allowWrites":false,"maxRetries":0,"maxAgentTurns":2,"maxCostUsd":2.0},
        "scope":{"projectId":"project-one","repository":"example/repository","identity":"connected-user","linearProjectId":"linear-project","targets":[{"projectId":"project-one","repository":"example/repository","identity":"connected-user"},{"projectId":"project-two"}]}
    })).unwrap();
    let event = json!({"repository":"example/repository","headSha":"fixture-head","issue":null,"target":{"projectId":"project-one"}});
    let fixture = SimulationFixture {
        event: event.clone(),
        outputs: BTreeMap::from([
            ("query".into(), json!({"ready":true})),
            ("jev".into(), json!({"classification":"ready"})),
            ("agent".into(), json!({"result":"done"})),
        ]),
    };
    let simulation = simulate(&definition, &registry, &fixture).unwrap();
    let mut state = AutomationState::default();
    let workflow = state.save_workflow(definition, None, &registry).unwrap();
    let run = state
        .enqueue(
            &workflow.id,
            event,
            "fixture-event",
            Some("example/repository#7".into()),
            100,
        )
        .unwrap();
    state.transition(&run.id, RunStatus::Running, 101).unwrap();
    for step in simulation.steps.iter().take(5) {
        state
            .complete_node(
                &run.id,
                &step.node_id,
                step.port.as_deref().unwrap(),
                step.output.clone(),
                &registry,
                102,
            )
            .unwrap();
    }
    state.record_cost(&run.id, Some(0.25), 102).unwrap();
    state
        .transition(&run.id, RunStatus::AwaitingApproval, 103)
        .unwrap();
    state
        .approve(
            &run.id,
            "approval",
            Some("fixture-head".into()),
            "local-user",
            104,
        )
        .unwrap();
    let mut revision = workflow.clone();
    revision.name = "Updated fixture workflow".into();
    state.save_workflow(revision, Some(1), &registry).unwrap();
    // UUIDs are nondeterministic identities, not part of serialization behavior.
    state.runs[0].id = "fixture-run".into();
    for id in state.dedup.values_mut() {
        *id = "fixture-run".into();
    }
    for id in state.locks.values_mut() {
        *id = "fixture-run".into();
    }
    let payload = json!({"workflow":workflow,"registry":registry,"simulation":simulation,"run":state.runs[0],"state":state});
    let serialized = serde_json::to_string_pretty(&payload).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/automation-contract.json");
    if std::env::var_os("PROMETEU_UPDATE_CONTRACT_FIXTURES").is_some() {
        std::fs::write(path, serialized).unwrap();
    } else {
        assert_eq!(std::fs::read_to_string(path).expect("regenerate the automation contract fixture"),serialized,"Automation wire serialization changed; review the shared TypeScript contract and regenerate with PROMETEU_UPDATE_CONTRACT_FIXTURES=1");
    }
}
