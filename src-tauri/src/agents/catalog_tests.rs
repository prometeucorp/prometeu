use super::*;
use prometeu_process::query::UnixQueryLauncher;
use serde_json::json;
use std::time::Instant;

fn fake(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
}

#[test]
fn claude_empty_is_success_but_malformed_and_provider_errors_are_not() {
    assert_eq!(
        parse_claude(&json!({"response":{"subtype":"success","response":{"models":[]}}})),
        Ok(vec![])
    );
    assert_eq!(
        parse_claude(&json!({"response":{"subtype":"error"}}))
            .unwrap_err()
            .code,
        "err.modelsCatalog.failed"
    );
    assert_eq!(
        parse_claude(&json!({"response":{"subtype":"success","response":{}}}))
            .unwrap_err()
            .code,
        "err.modelsCatalog.invalid"
    );
    assert!(parse_claude(
        &json!({"response":{"subtype":"success","response":{"models":[{"value":""}]}}})
    )
    .is_err());
}

#[test]
fn codex_catalog_uses_launch_model_efforts_and_hidden_visibility() {
    let (models, next) = parse_codex_page(&json!({"data":[{"id":"internal","model":"launch-model","displayName":"A model","hidden":true,"supportedReasoningEfforts":[{"reasoningEffort":"high","description":"High"}]}],"nextCursor":"next"})).unwrap();
    assert_eq!(models[0].id, "launch-model");
    assert_eq!(models[0].label, "A model");
    assert_eq!(models[0].efforts, ["high"]);
    assert!(models[0].additional);
    assert_eq!(next.as_deref(), Some("next"));
    assert!(parse_codex_page(&json!({"data":[],"nextCursor":null}))
        .unwrap()
        .0
        .is_empty());
    assert!(parse_codex_page(&json!({})).is_err());
    assert!(parse_codex_page(&json!({"data":[{"id":"not-a-launch-id"}]})).is_err());
}

#[test]
fn codex_queries_all_pages_and_never_starts_a_thread() {
    let mut command = fake(
        r#"
read init
case "$init" in *'"method":"initialize"'*) ;; *) exit 1;; esac
printf '%s\n' '{"id":1,"result":{}}'
read initialized
case "$initialized" in *'"method":"initialized"'*) ;; *) exit 2;; esac
read page
case "$page" in *'"includeHidden":true'*) ;; *) exit 3;; esac
printf '%s\n' '{"id":2,"result":{"data":[{"model":"first","displayName":"First","supportedReasoningEfforts":[]}],"nextCursor":"second"}}'
read page
case "$page" in *'"cursor":"second"'*) ;; *) exit 4;; esac
printf '%s\n' '{"id":3,"result":{"data":[{"model":"second","displayName":"Second","hidden":true,"supportedReasoningEfforts":[]}],"nextCursor":null}}'
read unexpected
exit 5
"#,
    );
    let models = query_codex(
        &UnixQueryLauncher,
        &mut command,
        Duration::from_secs(2),
        "test",
    )
    .unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn codex_repeated_cursor_fails_instead_of_looping() {
    let mut command = fake(
        r#"
read init
printf '%s\n' '{"id":1,"result":{}}'
read initialized
read page
printf '%s\n' '{"id":2,"result":{"data":[],"nextCursor":"same"}}'
read page
printf '%s\n' '{"id":3,"result":{"data":[],"nextCursor":"same"}}'
read unexpected
"#,
    );
    assert_eq!(
        query_codex(
            &UnixQueryLauncher,
            &mut command,
            Duration::from_secs(2),
            "test"
        )
        .unwrap_err()
        .code,
        "err.modelsCatalog.invalid"
    );
}

#[test]
fn process_timeout_kills_and_reaps_child() {
    let mut command = fake("echo $$; exec sleep 30");
    let mut process =
        CatalogProcess::spawn(&UnixQueryLauncher, &mut command, Duration::from_millis(40)).unwrap();
    let pid: i32 = process
        .read_line()
        .unwrap()
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        process.read_line().unwrap_err().code,
        "err.modelsCatalog.timeout"
    );
    drop(process);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
}

#[test]
fn subprocess_failure_is_not_an_empty_catalog() {
    assert_eq!(
        command_output(
            &UnixQueryLauncher,
            &mut fake("exit 2"),
            Duration::from_secs(1)
        )
        .unwrap_err()
        .code,
        "err.modelsCatalog.failed"
    );
    assert_eq!(
        command_output(
            &UnixQueryLauncher,
            &mut fake("exit 0"),
            Duration::from_secs(1)
        ),
        Ok(String::new())
    );
}

#[test]
fn missing_account_fails_before_any_provider_process() {
    for provider in [
        ProviderId::Claude,
        ProviderId::Codex,
        ProviderId::Antigravity,
    ] {
        assert_eq!(
            fetch_for_profile(
                &UnixQueryLauncher,
                provider,
                Err("no active account".into())
            )
            .unwrap_err()
            .code,
            "err.modelsCatalog.noAccount"
        );
    }
}

#[test]
fn claude_correlates_response_and_cleans_up_after_success_or_invalid_data() {
    for (response, expected) in [
        (
            r#"{"type":"control_response","response":{"request_id":"models","subtype":"success","response":{"models":[]}}}"#,
            None,
        ),
        (
            r#"{"type":"control_response","response":{"request_id":"models","subtype":"success","response":{}}}"#,
            Some("err.modelsCatalog.invalid"),
        ),
    ] {
        let pid_file =
            std::env::temp_dir().join(format!("prometeu-catalog-{}.pid", uuid::Uuid::new_v4()));
        let script = format!("echo $$ > '{}'\nread request\nprintf '%s\\n' '{{\"type\":\"control_response\",\"response\":{{\"request_id\":\"other\",\"subtype\":\"error\"}}}}' '{}'\nread unexpected", pid_file.display(), response);
        let result = query_claude(
            &UnixQueryLauncher,
            &mut fake(&script),
            Duration::from_secs(2),
        );
        match expected {
            None => assert!(result.unwrap().is_empty()),
            Some(code) => assert_eq!(result.unwrap_err().code, code),
        }
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let _ = std::fs::remove_file(&pid_file);
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
    }
}

#[test]
fn codex_timeout_applies_to_whole_query_not_each_page() {
    let mut command = fake(
        r#"
read init
sleep 0.1
printf '%s\n' '{"id":1,"result":{}}'
read initialized
read page
sleep 0.1
printf '%s\n' '{"id":2,"result":{"data":[],"nextCursor":"next"}}'
read page
sleep 0.1
printf '%s\n' '{"id":3,"result":{"data":[],"nextCursor":null}}'
"#,
    );
    assert_eq!(
        query_codex(
            &UnixQueryLauncher,
            &mut command,
            Duration::from_millis(250),
            "test"
        )
        .unwrap_err()
        .code,
        "err.modelsCatalog.timeout"
    );
}

#[test]
fn catalog_model_deserialization_defaults_additional_for_older_payloads() {
    let model: Model =
        serde_json::from_value(json!({"id":"custom","label":"Custom","efforts":[]})).unwrap();
    assert!(!model.additional);
}

#[test]
fn a_provider_that_does_not_read_cannot_block_the_catalog_deadline() {
    let mut process = CatalogProcess::spawn(
        &UnixQueryLauncher,
        &mut fake("sleep 30"),
        Duration::from_millis(40),
    )
    .unwrap();
    assert_eq!(
        process
            .send(json!({"large": "x".repeat(1_048_576)}))
            .unwrap_err()
            .code,
        "err.modelsCatalog.timeout"
    );
}

#[test]
fn unavailable_executable_and_invalid_protocol_are_distinct_errors() {
    let mut missing = Command::new("/prometeu-test-no-such-provider");
    assert_eq!(
        query_codex(
            &UnixQueryLauncher,
            &mut missing,
            Duration::from_secs(1),
            "test"
        )
        .unwrap_err()
        .code,
        "err.modelsCatalog.unavailable"
    );
    assert_eq!(
        query_codex(
            &UnixQueryLauncher,
            &mut fake("read init; printf '%s\\n' 'invalid json'; read wait"),
            Duration::from_secs(1),
            "test"
        )
        .unwrap_err()
        .code,
        "err.modelsCatalog.invalid"
    );
}

/// Requires installed CLIs and the accounts selected in PROMETEU_ROOT (debug defaults to
/// ~/.prometeu-dev). This performs catalog queries only, without creating inference sessions.
#[test]
#[ignore = "queries the selected accounts through installed CLIs"]
fn installed_selected_account_catalog_smoke() {
    let mut queried = 0;
    let mut failures = Vec::new();
    for provider in crate::agents::agents().providers {
        if !provider.installed || accounts::active(provider.id).is_err() {
            println!(
                "{}: skipped (not installed or no selected account)",
                provider.label
            );
            continue;
        }
        queried += 1;
        match fetch(&UnixQueryLauncher, provider.id) {
            Ok(catalog) => {
                let efforts: std::collections::BTreeSet<_> = catalog
                    .models
                    .iter()
                    .flat_map(|model| model.efforts.iter())
                    .collect();
                println!(
                    "{}: {} models; efforts: {:?}",
                    provider.label,
                    catalog.models.len(),
                    efforts
                );
            }
            Err(error) => failures.push(format!("{}: {}", provider.label, error.code)),
        }
    }
    assert!(queried > 0, "no installed provider with a selected account");
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

#[test]
fn oversized_unterminated_stdout_is_invalid_before_the_deadline() {
    let result = command_output(
        &UnixQueryLauncher,
        &mut fake("head -c 1048577 /dev/zero; sleep 30"),
        Duration::from_secs(2),
    );
    assert_eq!(result.unwrap_err().code, "err.modelsCatalog.invalid");
}

#[test]
fn stdout_limit_applies_across_many_small_lines() {
    let result = command_output(
        &UnixQueryLauncher,
        &mut fake("awk 'BEGIN { for (i = 0; i < 8193; i++) printf \"%0127d\\n\", 0 }'; sleep 30"),
        Duration::from_secs(2),
    );
    assert_eq!(result.unwrap_err().code, "err.modelsCatalog.invalid");
}

#[test]
fn dropping_catalog_process_terminates_its_descendant() {
    let mut process = CatalogProcess::spawn(
        &UnixQueryLauncher,
        &mut fake("sleep 30 & printf '%s\\n' \"$!\"; wait"),
        Duration::from_secs(2),
    )
    .unwrap();
    let descendant: i32 = process
        .read_line()
        .unwrap()
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(descendant, 0) }, 0);
    drop(process);
    // An orphan is reaped asynchronously by the system after its process group is killed.
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(descendant, 0) } == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let terminated = unsafe { libc::kill(descendant, 0) } == -1;
    if !terminated {
        unsafe {
            libc::kill(descendant, libc::SIGKILL);
        }
    }
    assert!(terminated, "catalog descendant survived process cleanup");
}

#[test]
fn managed_profile_catalog_uses_selected_home_without_inherited_credentials() {
    let home =
        std::env::temp_dir().join(format!("prometeu-catalog-profile-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("catalog.json"), r#"{"id":2,"result":{"data":[{"model":"selected-account-model","displayName":"Selected account","supportedReasoningEfforts":[]}],"nextCursor":null}}"#).unwrap();
    let profile = accounts::Profile {
        id: uuid::Uuid::new_v4().to_string(),
        provider: ProviderId::Codex,
        home: home.clone(),
        managed: true,
        revision: 3,
    };
    let mut command = fake(
        r#"
[ "$CODEX_HOME" = "$EXPECTED_ACCOUNT_HOME" ] || exit 1
[ -z "${OPENAI_API_KEY+x}${CODEX_API_KEY+x}${CODEX_ACCESS_TOKEN+x}${OPENAI_BASE_URL+x}${CODEX_CHATGPT_BASE_URL+x}" ] || exit 2
read init
printf '%s\n' '{"id":1,"result":{}}'
read initialized
read page
cat "$CODEX_HOME/catalog.json"
printf '\n'
read unexpected
"#,
    );
    command
        .env("EXPECTED_ACCOUNT_HOME", &home)
        .env("CODEX_HOME", "/incorrect-account");
    for key in [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "OPENAI_BASE_URL",
        "CODEX_CHATGPT_BASE_URL",
    ] {
        command.env(key, "synthetic-inherited-value");
    }
    crate::accounts::apply_profile(&profile, &mut command).unwrap();
    let result = query_codex(
        &UnixQueryLauncher,
        &mut command,
        Duration::from_secs(2),
        "test",
    );
    std::fs::remove_dir_all(home).unwrap();
    assert_eq!(result.unwrap()[0].id, "selected-account-model");
}
