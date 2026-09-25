//! Antigravity's native NDJSON boundary; its CLI owns authentication and native history.
use crate::{accounts, chat, conversation, i18n, lock::lock, paths, session::Launch};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn discover(args: &[&str]) -> Option<String> {
    let mut command = Command::new("agy");
    command.args(args);
    run_bounded(command, Duration::from_secs(10))
}

fn run_bounded(mut command: Command, timeout: Duration) -> Option<String> {
    use std::os::unix::process::CommandExt;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let pid = child.id() as i32;
    let mut stdout = child.stdout.take()?;
    let (data_tx, data_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut data = String::new();
        let result = stdout
            .by_ref()
            .take(1_048_576)
            .read_to_string(&mut data)
            .map(|_| data);
        let _ = data_tx.send(result);
    });
    let (exit_tx, exit_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = exit_tx.send(child.wait());
    });
    if matches!(exit_rx.recv_timeout(timeout), Ok(Ok(status)) if status.success()) {
        if let Ok(Ok(data)) = data_rx.recv_timeout(Duration::from_millis(200)) {
            return Some(data);
        }
    }
    // The process owns its group. Killing it also releases a descendant that inherited stdout;
    // the wait thread reaps the direct child without a 20 ms completion poll.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = exit_rx.recv_timeout(Duration::from_secs(2));
    None
}
pub fn installed() -> bool {
    discover(&["--version"]).is_some_and(|version| supported_version(&version))
}
fn supported_version(raw: &str) -> bool {
    semver::Version::parse(raw.trim().trim_start_matches('v'))
        .is_ok_and(|version| version >= semver::Version::new(1, 2, 7))
}
fn failure_message(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("permission check failed") || lower.contains("user denied permission") {
        return i18n::t("err.antigravity.permission");
    }
    if [
        "authentication required",
        "not authenticated",
        "please log in",
        "please login",
        "unauthenticated",
    ]
    .iter()
    .any(|text| lower.contains(text))
    {
        i18n::t("err.antigravity.auth")
    } else {
        i18n::t("err.antigravity.result")
    }
}
fn stderr_line(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    if [
        "soft-denied",
        "soft denied",
        "permission denied",
        "requires approval",
        "requires permission",
        "denied by policy",
    ]
    .iter()
    .any(|text| lower.contains(text))
    {
        Some(i18n::t("err.antigravity.permission"))
    } else if [
        "authentication required",
        "not authenticated",
        "unauthenticated",
    ]
    .iter()
    .any(|text| lower.contains(text))
    {
        Some(i18n::t("err.antigravity.auth"))
    } else {
        None
    }
}
pub(crate) fn parse_catalog(
    raw: &str,
) -> Result<Vec<crate::agents::Model>, crate::agents::CatalogError> {
    let models = parse_models(raw);
    if raw
        .lines()
        .any(|line| !line.trim().is_empty() && parse_models(line).is_empty())
    {
        return Err(crate::agents::CatalogError::new("invalid"));
    }
    Ok(models)
}
/// Read the CLI's own quota report without creating an inference turn.
pub fn quota() -> Option<Vec<crate::usage::Window>> {
    parse_quota(&discover(&["-p", "/usage"])?)
}

fn parse_quota(raw: &str) -> Option<Vec<crate::usage::Window>> {
    let mut windows = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split('\t').map(str::trim).collect();
        if fields.len() != 4 || fields[0].is_empty() {
            return None;
        }
        let kind = match fields[1] {
            "Weekly Limit Remaining" => "weekly",
            "Five Hour Limit Remaining" => "session",
            _ => return None,
        };
        let remaining: f64 = fields[2].strip_suffix('%')?.parse().ok()?;
        if !remaining.is_finite() || !(0.0..=100.0).contains(&remaining) {
            return None;
        }
        windows.push(crate::usage::Window {
            kind: kind.into(),
            pct: 100.0 - remaining,
            resets: crate::usage::rfc3339(fields[3])?,
            scope: Some(fields[0].into()),
            label: Some(fields[0].into()),
        });
    }
    (!windows.is_empty()).then_some(windows)
}

fn parse_models(raw: &str) -> Vec<crate::agents::Model> {
    let mut seen = std::collections::HashSet::new();
    raw.lines()
        .filter_map(|line| {
            let (id, label) = line.split_once('\t')?;
            let (id, label) = (id.trim(), label.trim());
            if id.is_empty()
                || label.is_empty()
                || !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
                || !seen.insert(id.to_owned())
            {
                return None;
            }
            Some(crate::agents::Model {
                id: id.into(),
                label: label.into(),
                efforts: vec![],
                additional: false,
            })
        })
        .collect()
}
fn launch_args(
    resume: Option<&str>,
    worktree: &Path,
    launch: &Launch,
) -> Result<Vec<String>, String> {
    if launch.plan
        || [&launch.mcp, &launch.plugins, &launch.skills]
            .iter()
            .any(|v| v.as_ref().is_some_and(|ids| !ids.is_empty()))
    {
        return Err(i18n::t("err.antigravity.unsupported"));
    }
    let mut args: Vec<String> = [
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--print-timeout",
        "0",
        "--disable-slash-commands",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    // agy resolves its workspace from native session state, not from the process cwd.
    let worktree = std::path::absolute(worktree).map_err(i18n::io)?;
    args.extend(["--add-dir".into(), worktree.display().to_string()]);
    for (flag, value) in [
        ("--conversation", resume.unwrap_or("")),
        ("--model", launch.model.as_str()),
        ("--effort", launch.effort.as_str()),
    ] {
        if !value.is_empty() {
            args.extend([flag.into(), value.into()]);
        }
    }
    // Ordinary conversations use automatic execution; explicit Ask task profiles retain their policy.
    if launch.permission != Some(crate::actions::Permission::Ask) {
        args.push("--dangerously-skip-permissions".into());
    }
    Ok(args)
}
pub fn spawn(
    app: &tauri::AppHandle,
    id: &str,
    _workspace: &str,
    cwd: &Path,
    resume: Option<String>,
    launch: &Launch,
) -> Result<chat::Chat, String> {
    let args = launch_args(resume.as_deref(), cwd, launch)?;
    if !installed() {
        return Err(i18n::t("err.antigravity.version"));
    }
    let profile = accounts::active(crate::state::ProviderId::Antigravity)?;
    let mut cmd = Command::new("agy");
    cmd.args(args).current_dir(cwd);
    let instructions = launch.instructions.clone();
    let io = chat::ProcessIo::new(
        stderr_line,
        move |stdin| {
            let link = Arc::new(Mutex::new(Link::new(Box::new(stdin), resume, instructions)));
            let reader = link.clone();
            (
                chat::Wire::Antigravity(link),
                Box::new(move |line: &str| {
                    lock(&reader)
                        .on_line(line)
                        .into_iter()
                        .map(|v| v.to_string())
                        .collect()
                }) as chat::Translate,
            )
        },
        profile,
    );
    let log = paths::chat_log(id);
    chat::launch(
        app,
        id,
        cmd,
        &log,
        Some(log.clone()),
        "err.antigravity.spawn",
        io,
    )
}
fn event(kind: &str, fields: Value) -> Value {
    conversation::event(kind, conversation::now(), fields)
}
#[derive(Default)]
struct Step {
    text: String,
    done: bool,
    tool: bool,
    block: Option<Value>,
}
pub struct Link {
    out: Option<Box<dyn Write + Send>>,
    session: Option<String>,
    identity_sent: bool,
    instructions: String,
    busy: bool,
    failed: bool,
    user_interrupted: bool,
    steps: BTreeMap<u64, Step>,
    started: Instant,
    turn_id: String,
}
impl Link {
    fn new(out: Box<dyn Write + Send>, session: Option<String>, instructions: String) -> Self {
        Self {
            out: Some(out),
            session,
            identity_sent: false,
            instructions,
            busy: false,
            failed: false,
            user_interrupted: false,
            steps: BTreeMap::new(),
            started: Instant::now(),
            turn_id: String::new(),
        }
    }
    pub fn close(&mut self) {
        self.out.take();
    }
    pub fn interrupted(&mut self) -> Vec<Value> {
        self.user_interrupted = true;
        vec![]
    }
    pub fn write(&mut self, v: &Value) -> Result<Vec<Value>, String> {
        if v["v"] != 1 {
            return Err(i18n::t("err.team.bad"));
        }
        if self.failed || self.out.is_none() {
            return Err(i18n::t("err.antigravity.transport"));
        }
        match v["type"].as_str() {
            Some("commands.list") => Ok(vec![event("commands.updated", json!({"commands":[]}))]),
            Some("message.send") => {
                let text = v["text"].as_str().ok_or_else(|| i18n::t("err.team.bad"))?;
                self.send_prompt(text.to_owned())?;
                Ok(vec![])
            }
            _ => Err(i18n::t("err.antigravity.unsupported")),
        }
    }
    fn send_prompt(&mut self, mut text: String) -> Result<(), String> {
        // The core persists pending input and releases it after publishing turn.completed.
        if self.busy {
            return Err("conversation_busy".into());
        }
        if !self.instructions.is_empty() {
            text = format!("{}\n\n{text}", self.instructions);
        }
        let frame = json!({"event":"user","message":{"content":text}});
        let result = self
            .out
            .as_mut()
            .ok_or_else(|| i18n::t("err.antigravity.transport"))
            .and_then(|out| {
                writeln!(out, "{frame}")
                    .and_then(|_| out.flush())
                    .map_err(|_| i18n::t("err.antigravity.transport"))
            });
        if result.is_err() {
            self.failed = true;
            return result;
        }
        self.instructions.clear();
        self.busy = true;
        self.user_interrupted = false;
        self.steps.clear();
        self.started = Instant::now();
        self.turn_id = uuid::Uuid::new_v4().to_string();
        Ok(())
    }
    fn identity(&mut self, v: &Value) -> Vec<Value> {
        if let Some(id) = v["conversation_id"].as_str().filter(|s| !s.is_empty()) {
            if self.session.as_deref() != Some(id) || !self.identity_sent {
                self.session = Some(id.into());
                self.identity_sent = true;
                return vec![event("session.identity", json!({"providerSession":id}))];
            }
        }
        vec![]
    }
    fn message_id(&self, index: u64) -> String {
        format!(
            "antigravity:{}:{index}",
            self.session.as_deref().unwrap_or(&self.turn_id)
        )
    }
    pub fn on_line(&mut self, line: &str) -> Vec<Value> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return vec![];
        };
        match v["event"].as_str() {
            Some("init") => {
                let mut out = self.identity(&v);
                out.push(event("commands.updated", json!({"commands":[]})));
                out
            }
            Some("step_update") if self.busy => self.step(&v["step_update"]),
            Some("result") if !self.failed => self.result(&v["result"]),
            _ => vec![],
        }
    }
    fn step(&mut self, v: &Value) -> Vec<Value> {
        let Some(index) = v["step_index"].as_u64() else {
            return vec![];
        };
        if !matches!(v["state"].as_str(), Some("ACTIVE" | "DONE" | "ERROR")) {
            return vec![];
        }
        if !matches!(v["step_type"].as_str(), Some("agent_response" | "tool")) {
            return vec![];
        }
        let mut out = self.identity(v);
        if v["step_type"] == "agent_response"
            && v["text_delta"].as_str().is_none_or(str::is_empty)
            && !self.steps.contains_key(&index)
        {
            return out;
        }
        let id = self.message_id(index);
        let step = self.steps.entry(index).or_default();
        if step.done {
            return out;
        }
        if v["step_type"] == "agent_response" {
            if step.block.is_none() {
                out.push(event("assistant.started", json!({"messageId":id})));
                out.push(event(
                    "assistant.block.started",
                    json!({"messageId":id,"index":0,"block":{"kind":"text","text":""}}),
                ));
                step.block = Some(json!({"kind":"text","text":""}));
            }
            if let Some(delta) = v["text_delta"].as_str().filter(|s| !s.is_empty()) {
                step.text.push_str(delta);
                out.push(event(
                    "assistant.delta",
                    json!({"messageId":id,"index":0,"kind":"text","delta":delta}),
                ));
            }
            if matches!(v["state"].as_str(), Some("DONE" | "ERROR")) {
                out.push(event(
                    "assistant.block",
                    json!({"messageId":id,"index":0,"block":{"kind":"text","text":step.text}}),
                ));
            }
        } else {
            step.tool = true;
            let info = &v["tool_info"];
            let name = info["name"]
                .as_str()
                .or_else(|| v["tool_name"].as_str())
                .unwrap_or("tool");
            let input = info["parameters"].as_object().cloned().unwrap_or_default();
            let block = json!({"kind":"tool","id":id,"name":name,"input":input});
            if step.block.as_ref() != Some(&block) {
                out.push(event(
                    "assistant.block",
                    json!({"messageId":id,"index":0,"block":block}),
                ));
                step.block = Some(block);
            }
            if matches!(v["state"].as_str(), Some("DONE" | "ERROR")) {
                let error = v["state"] == "ERROR" || !info["error"].is_null();
                let output = info["output"]
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        if error {
                            failure_message(info["error"]["message"].as_str().unwrap_or(""))
                        } else {
                            String::new()
                        }
                    });
                out.push(event(
                    "tool.completed",
                    json!({"toolId":id,"output":output,"error":error,"background":false}),
                ));
            }
        }
        step.done = matches!(v["state"].as_str(), Some("DONE" | "ERROR"));
        out
    }
    fn result(&mut self, v: &Value) -> Vec<Value> {
        let mut out = self.identity(v);
        let status = v["status"].as_str().unwrap_or("INVALID");
        // A failed startup can produce a result before any prompt is accepted.
        if !self.busy && status == "SUCCESS" {
            return out;
        }
        for (index, step) in &self.steps {
            if !step.done && step.tool {
                out.push(event("tool.completed", json!({"toolId":self.message_id(*index),"output":"","error":true,"background":false})));
            }
            if !step.done && !step.tool {
                out.push(event("assistant.block", json!({"messageId":self.message_id(*index),"index":0,"block":{"kind":"text","text":step.text}})));
            }
        }
        if self.steps.values().all(|step| step.text.is_empty()) {
            if let Some(text) = v["response"].as_str().filter(|text| !text.is_empty()) {
                out.push(event("assistant.block", json!({"messageId":format!("antigravity:result:{}",self.turn_id),"index":0,"block":{"kind":"text","text":text}})));
            }
        }
        let denied = v["denied_actions"]
            .as_array()
            .is_some_and(|actions| !actions.is_empty());
        let outcome = if self.user_interrupted
            || matches!(status, "CANCELED" | "INTERRUPTED")
            || (status == "ERROR" && v["error"].as_str() == Some("interrupted"))
        {
            "interrupted"
        } else if status == "SUCCESS" && !denied {
            "ok"
        } else {
            "error"
        };
        // Native duration and token totals are cumulative, so measure the accepted turn locally.
        out.push(event("turn.completed", json!({"outcome":outcome,"message":if outcome == "error" {if denied {i18n::t("err.antigravity.permission")} else {failure_message(v["error"].as_str().unwrap_or(""))}} else {String::new()},"durationMs":self.started.elapsed().as_millis() as u64,"costUsd":null})));
        self.busy = false;
        if outcome == "error" {
            self.failed = true;
            self.close();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_discovery_reaps_success_and_times_out_a_hung_child() {
        let mut success = Command::new("/bin/sh");
        success.args(["-c", "printf 'ready'"]);
        assert_eq!(
            run_bounded(success, Duration::from_secs(1)).as_deref(),
            Some("ready")
        );

        let mut hung = Command::new("/bin/sh");
        hung.args(["-c", "sleep 5"]);
        let started = Instant::now();
        assert!(run_bounded(hung, Duration::from_millis(100)).is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
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
    impl Output {
        fn take(&self) -> Vec<Value> {
            String::from_utf8(std::mem::take(&mut *lock(&self.0)))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }
    fn link() -> (Link, Output) {
        let out = Output::default();
        (
            Link::new(
                Box::new(out.clone()),
                Some("native".into()),
                "instructions".into(),
            ),
            out,
        )
    }
    fn send(link: &mut Link, text: &str) {
        link.write(&json!({"v":1,"type":"message.send","text":text}))
            .unwrap();
    }
    fn frame(link: &mut Link, payload: Value) -> Vec<Value> {
        link.on_line(&payload.to_string())
    }
    fn step(link: &mut Link, index: u64, state: &str, text: &str) -> Vec<Value> {
        frame(
            link,
            json!({"event":"step_update","step_update":{"conversation_id":"native","step_index":index,"state":state,"step_type":"agent_response","text_delta":text}}),
        )
    }
    fn result(link: &mut Link, status: &str, text: &str) -> Vec<Value> {
        frame(
            link,
            json!({"event":"result","result":{"conversation_id":"native","status":status,"response":text,"duration_seconds":900,"usage":{"input_tokens":90000},"error":"secret-token"}}),
        )
    }
    #[test]
    fn model_catalog_distinguishes_empty_and_invalid_output() {
        assert!(parse_catalog("").unwrap().is_empty());
        assert_eq!(
            parse_catalog("unexpected output").unwrap_err().code,
            "err.modelsCatalog.invalid"
        );
        assert!(parse_catalog("custom-model\tCustom Model\ninvalid line\n").is_err());
        let models = parse_catalog("custom-model\tCustom Model\n").unwrap();
        assert_eq!(models[0].id, "custom-model");
        assert!(models[0].efforts.is_empty());
        assert!(!models[0].additional);
    }

    #[test]
    fn prompts_before_init_reject_busy_until_completion_and_instructions_only_once() {
        let (mut link, output) = link();
        send(&mut link, "first");
        assert_eq!(
            link.write(&json!({"v":1,"type":"message.send","text":"next"}))
                .unwrap_err(),
            "conversation_busy"
        );
        assert_eq!(
            output.take(),
            vec![json!({"event":"user","message":{"content":"instructions\n\nfirst"}})]
        );
        let init = frame(
            &mut link,
            json!({"event":"init","conversation_id":"native","init":{"tools":["private"]}}),
        );
        assert_eq!(init[0]["providerSession"], "native");
        assert_eq!(init[1]["type"], "commands.updated");
        result(&mut link, "SUCCESS", "one");
        assert!(output.take().is_empty());
        assert!(!link.busy);
        send(&mut link, "next");
        assert_eq!(
            output.take(),
            vec![json!({"event":"user","message":{"content":"next"}})]
        );
    }
    #[test]
    fn deltas_include_done_once_without_final_response_duplication() {
        let (mut link, _) = link();
        send(&mut link, "hi");
        step(&mut link, 2, "ACTIVE", "hello");
        let done = step(&mut link, 2, "DONE", " world");
        assert_eq!(done.last().unwrap()["block"]["text"], "hello world");
        assert_eq!(done.last().unwrap()["messageId"], "antigravity:native:2");
        assert!(step(&mut link, 2, "DONE", " world").is_empty());
        let done = result(&mut link, "SUCCESS", "hello world\n");
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["outcome"], "ok");
        assert!(done[0]["durationMs"].as_u64().unwrap() < 900000);
    }
    #[test]
    fn tool_details_update_same_block_then_complete_once() {
        let (mut link, _) = link();
        send(&mut link, "tool");
        let active = json!({"event":"step_update","step_update":{"conversation_id":"native","step_index":4,"state":"ACTIVE","step_type":"tool","tool_name":"run_command"}});
        frame(&mut link, active);
        let done = json!({"event":"step_update","step_update":{"conversation_id":"native","step_index":4,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hello"},"output":"hello\r\n"}}});
        let events = frame(&mut link, done.clone());
        assert_eq!(events[0]["block"]["input"]["CommandLine"], "echo hello");
        assert_eq!(events[1]["toolId"], events[0]["block"]["id"]);
        assert_eq!(events[1]["output"], "hello\r\n");
        assert!(frame(&mut link, done).is_empty());
        let serialized = serde_json::to_string(&events).unwrap();
        for forbidden in ["tool_info", "step_index", "step_update", "conversation_id"] {
            assert!(!serialized.contains(forbidden));
        }
    }
    #[test]
    fn terminal_statuses_clear_busy_and_never_expose_error_payload() {
        for (status, outcome) in [
            ("ERROR", "error"),
            ("INVALID", "error"),
            ("WAITING", "error"),
            ("RUNNING", "error"),
            ("CANCELED", "interrupted"),
            ("INTERRUPTED", "interrupted"),
        ] {
            let (mut link, _) = link();
            send(&mut link, "test");
            let events = result(&mut link, status, "");
            assert_eq!(events.last().unwrap()["outcome"], outcome);
            assert!(!link.busy);
            assert!(!serde_json::to_string(&events)
                .unwrap()
                .contains("secret-token"));
        }
    }
    #[test]
    fn result_fallback_and_unfinished_stream_are_sealed() {
        let (mut link, _) = link();
        send(&mut link, "one");
        let events = result(&mut link, "SUCCESS", "fallback");
        assert!(events.iter().any(|e| e["block"]["text"] == "fallback"));
        send(&mut link, "two");
        step(&mut link, 5, "ACTIVE", "partial");
        let events = result(&mut link, "INTERRUPTED", "fallback ignored");
        assert_eq!(events[0]["block"]["text"], "partial");
    }
    #[test]
    fn local_interrupt_without_native_control_messages() {
        let (mut link, output) = link();
        send(&mut link, "one");
        output.take();
        assert!(link.interrupted().is_empty());
        assert!(output.take().is_empty());
        assert_eq!(
            result(&mut link, "SUCCESS", "").last().unwrap()["outcome"],
            "interrupted"
        );
        assert!(output.take().is_empty());
        send(&mut link, "three");
        assert_eq!(output.take().len(), 1);
    }
    #[test]
    fn discovery_keeps_native_labels_without_inventing_efforts() {
        let models = parse_models("gemini-3.1-pro-high\tGemini 3.1 Pro (High)\ninvalid line\nclaude-sonnet-4-6\tClaude Sonnet 4.6\ngemini-3.1-pro-high\tDuplicate\n");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].label, "Gemini 3.1 Pro (High)");
        assert!(models[0].efforts.is_empty());
    }
    #[test]
    fn launch_preserves_native_resume_and_permission_defaults() {
        let worktree = Path::new("/tmp/prometeu-worktree");
        let mut launch = Launch {
            model: "native-model".into(),
            effort: "high".into(),
            ..Launch::default()
        };
        let args = launch_args(Some("native-session"), worktree, &launch).unwrap();
        assert!(args
            .windows(2)
            .any(|a| a == ["--add-dir", "/tmp/prometeu-worktree"]));
        assert!(args
            .windows(2)
            .any(|a| a == ["--conversation", "native-session"]));
        assert!(args.windows(2).any(|a| a == ["--model", "native-model"]));
        assert!(args.windows(2).any(|a| a == ["--effort", "high"]));
        assert!(args.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(launch_args(None, worktree, &launch)
            .unwrap()
            .windows(2)
            .any(|a| a == ["--add-dir", "/tmp/prometeu-worktree"]));
        let relative = launch_args(None, Path::new("relative-worktree"), &launch).unwrap();
        let relative = relative
            .windows(2)
            .find(|a| a[0] == "--add-dir")
            .map(|a| Path::new(&a[1]))
            .unwrap();
        assert!(relative.is_absolute());
        assert!(relative.ends_with("relative-worktree"));
        launch.permission = Some(crate::actions::Permission::Ask);
        assert!(!launch_args(None, worktree, &launch)
            .unwrap()
            .iter()
            .any(|a| a == "--dangerously-skip-permissions"));
        launch.permission = Some(crate::actions::Permission::Auto);
        assert!(launch_args(None, worktree, &launch)
            .unwrap()
            .iter()
            .any(|a| a == "--dangerously-skip-permissions"));
        launch.plan = true;
        assert!(launch_args(None, worktree, &launch).is_err());
        launch.plan = false;
        launch.mcp = Some(vec!["selected".into()]);
        assert!(launch_args(None, worktree, &launch).is_err());
    }
    #[test]
    fn canonical_events_match_cross_language_fixture() {
        let mut events = vec![];
        for (recording, session) in [
            (
                include_str!("antigravity/fixtures/tool.ndjson"),
                "fixture-session",
            ),
            (
                include_str!("antigravity/fixtures/resume.ndjson"),
                "fixture-session",
            ),
            (
                include_str!("antigravity/fixtures/interrupted.ndjson"),
                "fixture-interrupted",
            ),
            (
                include_str!("antigravity/fixtures/permission-denied.ndjson"),
                "fixture-denied",
            ),
        ] {
            // Each recording starts a separate process; resume preserves the original step IDs.
            let mut link = Link::new(
                Box::new(Output::default()),
                Some(session.into()),
                String::new(),
            );
            send(&mut link, "fixture prompt");
            for line in recording.lines() {
                let mut native: Value = serde_json::from_str(line).unwrap();
                if native.get("conversation_id").is_some() {
                    native["conversation_id"] = json!(session);
                }
                for payload in ["step_update", "result"] {
                    if native[payload].get("conversation_id").is_some() {
                        native[payload]["conversation_id"] = json!(session);
                    }
                }
                events.extend(link.on_line(&native.to_string()));
            }
        }
        for event in &mut events {
            event["at"] = json!(0);
            if event["type"] == "turn.completed" {
                event["durationMs"] = json!(0);
            }
        }
        let fixture = json!({"provenance":"Canonical V1 conversion of the recorded Antigravity CLI 1.2.7 tool, native resume, and SIGINT NDJSON fixtures. Timestamps and turn durations are zeroed; the independent interrupted conversation has a distinct fixture identity.","events":events});
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/antigravity/fixtures/canonical-events.json");
        if std::env::var_os("PROMETEU_UPDATE_ANTIGRAVITY_FIXTURE").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, serde_json::to_string_pretty(&fixture).unwrap() + "\n").unwrap();
        } else {
            let expected: Value =
                serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(fixture, expected);
        }
    }
    #[test]
    fn version_and_stderr_classification_never_forward_raw_secrets() {
        assert!(supported_version("1.2.7\n"));
        assert!(!supported_version("1.2.6"));
        assert!(!supported_version("random output"));
        assert_eq!(
            failure_message("authentication required secret-token"),
            i18n::t("err.antigravity.auth")
        );
        assert_eq!(
            stderr_line("tool soft-denied secret-token"),
            Some(i18n::t("err.antigravity.permission"))
        );
        assert!(stderr_line("debug secret-token").is_none());
    }
    #[test]
    fn recorded_native_tool_resume_and_interrupt_translate_to_v1() {
        for (recording, expected) in [
            (include_str!("antigravity/fixtures/tool.ndjson"), "ok"),
            (include_str!("antigravity/fixtures/resume.ndjson"), "ok"),
            (
                include_str!("antigravity/fixtures/interrupted.ndjson"),
                "interrupted",
            ),
        ] {
            let (mut link, _) = link();
            send(&mut link, "fixture prompt");
            let events: Vec<Value> = recording
                .lines()
                .flat_map(|line| link.on_line(line))
                .collect();
            assert_eq!(events.last().unwrap()["outcome"], expected);
            assert!(events
                .iter()
                .any(|e| e["providerSession"] == "fixture-session"));
            assert!(events
                .iter()
                .all(|e| e["v"] == 1 && e.get("event").is_none()));
            assert!(!events.iter().any(|e| e["type"] == "assistant.block"
                && e["block"]["kind"] == "text"
                && e["block"]["text"] == ""));
            if expected == "ok" {
                assert!(events.iter().any(|e| e["block"]["text"] == "AGY_TOOL_OK\n"));
            }
        }
    }
    #[test]
    fn native_permission_denial_is_not_an_empty_success() {
        let (mut link, _) = link();
        send(&mut link, "fixture prompt");
        let events: Vec<Value> = include_str!("antigravity/fixtures/permission-denied.ndjson")
            .lines()
            .flat_map(|line| link.on_line(line))
            .collect();
        let completed: Vec<_> = events
            .iter()
            .filter(|e| e["type"] == "tool.completed")
            .collect();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0]["error"], true);
        assert_eq!(
            completed[0]["output"],
            i18n::t("err.antigravity.permission")
        );
        assert_eq!(events.last().unwrap()["outcome"], "error");
        assert_eq!(
            events.last().unwrap()["message"],
            i18n::t("err.antigravity.permission")
        );
    }

    #[test]
    fn native_quota_converts_remaining_to_used_and_preserves_groups() {
        let windows = parse_quota(include_str!("antigravity/fixtures/usage.tsv")).unwrap();
        assert_eq!(windows.len(), 4);
        assert_eq!(
            windows.iter().map(|w| w.pct).collect::<Vec<_>>(),
            [41.0, 3.0, 73.0, 100.0]
        );
        assert_eq!(windows[0].kind, "weekly");
        assert_eq!(windows[1].kind, "session");
        assert_eq!(windows[0].scope.as_deref(), Some("Gemini Models"));
        assert_eq!(windows[2].scope.as_deref(), Some("Claude and GPT models"));
        assert_eq!(
            windows[0].resets,
            crate::usage::rfc3339("2026-09-23T02:43:13Z").unwrap()
        );
        for raw in [
            "",
            "authentication required",
            "Models\tWeekly Limit Remaining\t101%\t2026-09-23T02:43:13Z",
            "Models\tWeekly Limit Remaining\tNaN%\t2026-09-23T02:43:13Z",
            "Models\tWeekly Limit Remaining\t2%\tinvalid",
        ] {
            assert!(parse_quota(raw).is_none());
        }
    }

    #[test]
    fn unsupported_controls_do_not_reach_native_stdin() {
        let (mut link, output) = link();
        for kind in [
            "request.respond",
            "permission.mode.set",
            "context.compact",
            "turn.interrupt",
        ] {
            assert!(link.write(&json!({"v":1,"type":kind})).is_err());
        }
        assert!(output.take().is_empty());
        assert!(link.on_line("unstructured secret-token").is_empty());
    }
}
