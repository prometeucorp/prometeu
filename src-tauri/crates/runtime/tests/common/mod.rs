//! Shared assertions for runtime integration tests.
use prometeu_bridge::application::ApplicationClient;
use serde_json::Value;

/// Assert that the request loop answers `load_board` while every `(poll command, job)` still
/// waits on a gate that only the test releases, after this returns.
///
/// The order is the assertion. A loop blocked by an operation never answers, so the client's
/// reply deadline fails the test; an operation that ignored its gate reports `done`.
pub fn assert_served_while_held(client: &mut dyn ApplicationClient, held: &[(&str, &Value)]) {
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert!(board["workspaces"].is_array(), "{board}");
    for (poll, job) in held {
        assert_eq!(
            client.application((*poll).into(), (*job).clone()).unwrap()["done"],
            false,
            "{poll} {job} finished before its gate was released"
        );
    }
}
