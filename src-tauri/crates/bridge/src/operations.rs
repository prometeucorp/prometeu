//! Preserve application results while releasing the native connection between bounded polls.
use crate::application::ApplicationClient;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub(crate) struct Deferred<'a>(pub &'a mut dyn ApplicationClient);
impl ApplicationClient for Deferred<'_> {
    fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
        let deferred = matches!(
            command.as_str(),
            "workspace_git_status"
                | "workspace_git_diff"
                | "workspace_git_action"
                | "workspace_git_history"
                | "workspace_git_branches"
                | "workspace_git_conflict"
                | "workspace_git_resolve"
                | "file_base"
                | "tree_git_status"
                | "tree_restore"
                | "workspace_branch"
                | "list_branches"
                | "create_workspace"
        );
        let initialization = matches!(command.as_str(), "agent_models" | "accounts")
            || command == "open_dock" && args["kind"] == "setup";
        if !(deferred && self.0.operations_supported()
            || initialization && self.0.initialization_supported())
        {
            return self.0.application(command, args);
        }
        let started = self.0.application(
            "application_operation_start".into(),
            json!({"command":command,"args":args}),
        )?;
        let job = started["job"]
            .as_str()
            .ok_or("Invalid application operation")?;
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            let result = self
                .0
                .application("application_operation_poll".into(), json!({"job":job}))?;
            match result["done"].as_bool() {
                Some(true) => {
                    return result
                        .get("result")
                        .cloned()
                        .ok_or("Invalid operation result".into())
                }
                Some(false) => {}
                None => return Err("Invalid operation status".into()),
            }
            if Instant::now() >= deadline {
                return Err("Operation timed out; its outcome is unknown".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Client {
        supported: bool,
        initialization: bool,
        fail: Option<&'static str>,
        calls: Vec<String>,
    }
    impl ApplicationClient for Client {
        fn initialization_supported(&self) -> bool {
            self.initialization
        }
        fn operations_supported(&self) -> bool {
            self.supported
        }
        fn application(&mut self, command: String, _: Value) -> Result<Value, String> {
            self.calls.push(command.clone());
            if self.fail == Some(command.as_str()) {
                return Err("outcome unknown".into());
            }
            Ok(match command.as_str() {
                "application_operation_start" => json!({"job":"one"}),
                "application_operation_poll" => json!({"done":true,"result":{"id":"created"}}),
                _ => json!({"id":"created"}),
            })
        }
    }
    #[test]
    fn capability_selects_transport_before_any_effect_and_preserves_results() {
        for supported in [false, true] {
            let mut client = Client {
                supported,
                initialization: false,
                fail: None,
                calls: vec![],
            };
            assert_eq!(
                Deferred(&mut client)
                    .application("create_workspace".into(), Value::Null)
                    .unwrap(),
                json!({"id":"created"})
            );
            assert_eq!(
                client.calls,
                match supported {
                    true => vec!["application_operation_start", "application_operation_poll"],
                    false => vec!["create_workspace"],
                }
            );
            Deferred(&mut client)
                .application("load_board".into(), Value::Null)
                .unwrap();
            assert_eq!(client.calls.last().unwrap(), "load_board");
        }
    }
    #[test]
    fn initialization_negotiates_separately_and_leaves_shell_commands_unchanged() {
        for initialization in [false, true] {
            for (command, args, eligible) in [
                ("agent_models", json!({"agent":"codex"}), true),
                ("accounts", Value::Null, true),
                ("open_dock", json!({"kind":"setup"}), true),
                ("open_dock", json!({"kind":"terminal"}), false),
                ("open_dock", json!({"kind":"run"}), false),
            ] {
                let mut client = Client {
                    supported: true,
                    initialization,
                    fail: None,
                    calls: vec![],
                };
                Deferred(&mut client)
                    .application(command.into(), args)
                    .unwrap();
                assert_eq!(
                    client.calls[0],
                    match initialization && eligible {
                        true => "application_operation_start",
                        false => command,
                    }
                );
            }
        }
    }
    #[test]
    fn uncertain_start_or_completion_never_replays_the_original_mutation() {
        for fail in ["application_operation_start", "application_operation_poll"] {
            let mut client = Client {
                supported: true,
                initialization: false,
                fail: Some(fail),
                calls: vec![],
            };
            assert_eq!(
                Deferred(&mut client)
                    .application("workspace_git_action".into(), Value::Null)
                    .unwrap_err(),
                "outcome unknown"
            );
            assert_eq!(
                client
                    .calls
                    .iter()
                    .filter(|c| *c == "application_operation_start")
                    .count(),
                1
            );
            assert!(!client.calls.iter().any(|c| c == "workspace_git_action"));
        }
    }
}
