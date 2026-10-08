//! Serialize real backend types and adapter output for the TypeScript consumer checks.

use crate::{agents, chat, claude, codex, state};
use serde_json::{json, Value};

fn board() -> state::Board {
    let mut board: state::Board = serde_json::from_value(json!({
        "stages": ["Working", "Done"],
        "projects": [{"id":"project","name":"Project","path":"/contract/repo"}],
        "workspaces": [{
            "id":"workspace","title":"Contract task","repo":"/contract/repo","repo_name":"Project",
            "branch":"task","worktree":"/contract/worktree","stage":"Working",
            "repos":[{"path":"/contract/repo","name":"Project","worktree":"/contract/worktree","base":"main","pr":{"number":7,"title":"Review","state":"OPEN","isDraft":true}}],
            "port":1420,"audience":["member"],"shared":true,"share_team":"team",
            "rights":{"send":["member"],"control":[]},
            "mcp":{"base":"none","add":[],"remove":[]},
            "plugins":{"base":"inherit","add":["plugin"],"remove":["disabled"]},
            "tabs":[
                {"id":"running","title":"Build","status":"rodando","note":"Testing","tokens":42,"choice":{"agent":"codex","model":"model","effort":"high"},"kickoff":"package/skill"},
                {"id":"waiting","title":"Review","status":"querendo","permission":"ask","plan":true},
                {"id":"ready","title":"Ready","status":"pronta","permission":"auto"},
                {"id":"stopped","title":"Stopped","status":"desligada","pending_prompt":"Continue","agent_session":"native-thread"}
            ],"active":"running"
        }, {
            "id":"empty","title":"Empty","repo":"/contract/repo","repo_name":"Project",
            "branch":"empty","worktree":"/contract/empty","stage":"Working"
        }]
    })).unwrap();
    // Defaults include translated action profiles; this fixture exercises the board wire shape
    // independently of locale and the person's action registry.
    board.actions = serde_json::from_value(json!({"profiles":[],"commands":[]})).unwrap();
    board
}

#[test]
fn serialized_boundary_fixture_is_current() {
    let mut streams = serde_json::Map::new();
    for (provider, events) in [
        ("claude", claude::contract::events()),
        ("codex", codex::contract::events()),
    ] {
        streams.insert(provider.into(), json!(chat::contract_public_events(events)));
    }
    let providers = [
        state::ProviderId::Claude,
        state::ProviderId::Codex,
        state::ProviderId::Antigravity,
        state::ProviderId::RetiredGemini,
    ];
    let capabilities: serde_json::Map<String, Value> = providers
        .iter()
        .map(|provider| {
            (
                serde_json::to_value(provider)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .into(),
                serde_json::to_value(agents::capabilities(*provider)).unwrap(),
            )
        })
        .collect();
    let models = agents::ModelCatalog {
        models: vec![
            agents::Model {
                id: "model".into(),
                label: "Model".into(),
                efforts: vec!["high".into()],
                additional: false,
            },
            agents::Model {
                id: "additional".into(),
                label: "Additional".into(),
                efforts: vec![],
                additional: true,
            },
        ],
        fetched_at: 1234,
    };
    let events = streams["claude"].as_array().unwrap();
    let snapshot = chat::Snapshot {
        text: events.iter().map(|event| format!("{event}\n")).collect(),
        seq: events.len() as u64,
    };
    let fixture = json!({
        "board":board(), "providers":providers, "capabilities":capabilities, "models":models,
        "snapshot":snapshot, "events":streams,
    });
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/backend-contract.json");
    let serialized = serde_json::to_string_pretty(&fixture).unwrap() + "\n";
    if std::env::var_os("PROMETEU_UPDATE_CONTRACT_FIXTURES").is_some() {
        std::fs::write(path, serialized).unwrap();
    } else {
        assert_eq!(std::fs::read_to_string(path).expect("run npm run contracts:update"), serialized,
            "Backend serialization changed. Review the contract and run npm run contracts:update, then npm run test:contracts.");
    }
}
