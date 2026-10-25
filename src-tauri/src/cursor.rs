//! Cursor owns authentication and native history; Prometeu stores only canonical presentation.
use crate::{accounts, i18n, paths};
use prometeu_core::session::launch::Launch;
use std::{path::Path, process::Command, sync::Arc, time::Duration};

mod quota;
pub use quota::quota;

pub fn installed() -> bool {
    // Use Cursor's unambiguous executable name, not another application's `agent` command.
    prometeu_protocols::catalog::command_output(
        &prometeu_process::query::UnixQueryLauncher,
        Command::new("cursor-agent").args(["acp", "--help"]),
        Duration::from_secs(5),
    )
    .is_ok_and(|help| help.contains("Agent Client Protocol"))
}

pub fn prepare(
    id: &str,
    cwd: &Path,
    resume: Option<String>,
    launch: &Launch,
    profiles: &dyn prometeu_profiles::ProfileBackend,
) -> Result<crate::agent_launch::PreparedAgent, String> {
    if [&launch.mcp, &launch.plugins, &launch.skills]
        .iter()
        .any(|selection| selection.as_ref().is_some_and(|ids| !ids.is_empty()))
    {
        return Err(i18n::t("err.cursor.unsupported"));
    }
    let profile = profiles.resolve(&accounts::selected(crate::state::ProviderId::Cursor)?)?;
    let mut command = Command::new("cursor-agent");
    command.arg("acp").current_dir(cwd);
    profiles.apply(&profile, &mut command)?;
    let start = prometeu_protocols::cursor::Start {
        cwd: cwd.to_string_lossy().into_owned(),
        resume,
        model: launch.model.clone(),
        plan: launch.plan,
        bypass: launch.permission != Some(prometeu_core::actions::Permission::Ask),
        instructions: launch.instructions.clone(),
    };
    let resumed = start.resume.is_some();
    let log = paths::chat_log(id);
    Ok(crate::agent_launch::PreparedAgent {
        command,
        store: Arc::new(crate::transcript_store::FileTranscriptStore::new(
            log.clone(),
            log,
        )),
        profile,
        connect: Box::new(move |stdin, _| prometeu_protocols::cursor::connect(stdin, start)),
        stderr_line: |_| None,
        spawn_error: "err.cursor.spawn",
        resumed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::lock::lock;
    use prometeu_protocols::{
        catalog::CatalogProcess,
        cursor::{Link, Start},
    };
    use serde_json::{json, Value};
    use std::{io::Write, sync::Mutex};

    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn exercise(start: Start, prompt: &str, interrupt: bool) -> Vec<Value> {
        let mut command = Command::new("cursor-agent");
        command.arg("acp").current_dir(&start.cwd);
        let mut process = CatalogProcess::spawn(
            &prometeu_process::query::UnixQueryLauncher,
            &mut command,
            Duration::from_secs(120),
        )
        .unwrap();
        let output = Output::default();
        let mut link = Link::new(Box::new(output.clone()), start);
        link.write(&json!({"v":1,"type":"message.send","text":prompt}))
            .unwrap();
        let mut events = vec![];
        let mut cancelled = false;
        loop {
            let bytes = std::mem::take(&mut *lock(&output.0));
            for line in String::from_utf8(bytes).unwrap().lines() {
                process.send(serde_json::from_str(line).unwrap()).unwrap();
            }
            let line = process
                .read_line()
                .unwrap()
                .expect("Cursor closed stdout before completing");
            let batch = link.on_line(&line);
            for event in &batch {
                if interrupt
                    && !cancelled
                    && matches!(
                        event["type"].as_str(),
                        Some("assistant.delta" | "request.opened")
                    )
                {
                    link.write(&json!({"v":1,"type":"turn.interrupt"})).unwrap();
                    cancelled = true;
                } else if event["type"] == "request.opened" && !cancelled {
                    assert_eq!(event["kind"], "approval");
                    link.write(&json!({"v":1,"type":"request.respond","requestId":event["requestId"],"response":{"outcome":"allow"}})).unwrap();
                }
            }
            let complete = batch.iter().any(|e| e["type"] == "turn.completed");
            events.extend(batch);
            if complete {
                return events;
            }
        }
    }
    #[test]
    #[ignore = "uses installed authenticated Cursor CLI and paid inference in a temporary directory"]
    fn live_cursor_round_trip() {
        assert!(installed());
        let cwd = std::env::temp_dir().join(format!("prometeu-cursor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&cwd).unwrap();
        let cwd = std::fs::canonicalize(cwd).unwrap();
        let mut command = Command::new("cursor-agent");
        command.arg("acp").current_dir(&cwd);
        let models = prometeu_protocols::cursor::query_catalog(
            &prometeu_process::query::UnixQueryLauncher,
            &mut command,
            Duration::from_secs(30),
            &cwd.to_string_lossy(),
        )
        .unwrap();
        let model = models
            .iter()
            .find(|m| m.id.starts_with("composer-"))
            .expect("Composer available for bounded smoke test")
            .id
            .clone();
        let settings = |resume, plan| Start {
            cwd: cwd.to_string_lossy().into_owned(),
            resume,
            model: model.clone(),
            plan,
            bypass: false,
            instructions: String::new(),
        };
        let plan = exercise(
            settings(None, true),
            "Reply PLAN_OK without using tools.",
            false,
        );
        assert_eq!(plan.last().unwrap()["outcome"], "ok");
        let first = exercise(settings(None, false), "Integration smoke test. Run pwd in shell and create probe.txt containing CURSOR_OK in this directory. Remember marker NATIVE_RIVER_5180. Reply CURSOR_OK. Do not delegate or access any other files.", false);
        assert_eq!(first.last().unwrap()["outcome"], "ok", "{first:?}");
        assert_eq!(
            std::fs::read_to_string(cwd.join("probe.txt"))
                .unwrap()
                .trim(),
            "CURSOR_OK"
        );
        assert!(first.iter().any(|e| e["type"] == "tool.completed"
            && e["output"]
                .as_str()
                .is_some_and(|s| s.contains(cwd.to_str().unwrap()))));
        assert!(first.iter().any(|e| e["type"] == "request.opened"));
        let session = first
            .iter()
            .find(|e| e["type"] == "session.identity")
            .unwrap()["providerSession"]
            .as_str()
            .unwrap()
            .to_owned();
        let resumed = exercise(
            settings(Some(session.clone()), true),
            "Reply only with the marker I asked you to remember. No tools.",
            false,
        );
        assert_eq!(resumed.last().unwrap()["outcome"], "ok");
        assert!(resumed.iter().any(|e| e["type"] == "assistant.block"
            && e["block"]["kind"] == "text"
            && e["block"]["text"]
                .as_str()
                .unwrap()
                .contains("NATIVE_RIVER_5180")));
        assert!(!resumed.iter().any(|e| e["type"] == "tool.completed"));
        let cancelled = exercise(
            settings(Some(session), false),
            "Run sleep 15 in the shell, then reply DONE.",
            true,
        );
        assert_eq!(cancelled.last().unwrap()["outcome"], "interrupted");
        std::fs::remove_dir_all(cwd).unwrap();
    }
}
