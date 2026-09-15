//! Adapt Claude stream-json commands and events at the process boundary. Everything delivered to
//! chat::Pump uses Prometeu's canonical conversation protocol.

use crate::conversation::{event, now};
use crate::{accounts, chat, i18n, paths};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;
use tauri::AppHandle;

pub fn user_home() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| paths::home())
                    .join(path)
            }
        })
        .unwrap_or_else(|| paths::home().join(".claude"))
}

const AUTH_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

pub fn account_env(command: &mut Command, profile: &accounts::Profile) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("CLAUDE") {
            command.env_remove(key);
        }
    }
    if profile.managed || std::env::var_os("CLAUDE_CONFIG_DIR").is_some() {
        command.env("CLAUDE_CONFIG_DIR", &profile.home);
    }
    if profile.managed {
        for key in AUTH_ENV {
            command.env_remove(key);
        }
    }
}

pub fn prepare_profile(profile: &accounts::Profile) -> Result<(), String> {
    prepare_profile_at(&user_home(), &profile.home)
}

fn prepare_profile_at(base: &Path, home: &Path) -> Result<(), String> {
    for name in [
        "projects",
        "plugins",
        "skills",
        "commands",
        "agents",
        "plans",
        "tasks",
        "file-history",
        "session-env",
    ] {
        accounts::share(base, home, name, true)?;
    }
    accounts::share(base, home, "CLAUDE.md", false)?;
    let body = match std::fs::read_to_string(base.join("settings.json")) {
        Ok(body) => Some(body),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(i18n::io(error)),
    };
    if let Some(body) = body {
        let mut settings: Value = serde_json::from_str(&body).map_err(i18n::io)?;
        settings
            .as_object_mut()
            .ok_or_else(|| i18n::t("err.account.profile"))?
            .remove("apiKeyHelper");
        if let Some(env) = settings["env"].as_object_mut() {
            env.retain(|key, _| !AUTH_ENV.contains(&key.as_str()) && key != "CLAUDE_CONFIG_DIR");
        }
        paths::write_private(&home.join("settings.json"), &settings.to_string())
            .map_err(i18n::io)?;
    }
    // MCP configuration and project trust live outside settings.json. Preserve the profile's login
    // identity without copying the global identity.
    let source = if base.join(".claude.json").exists() {
        base.join(".claude.json")
    } else {
        base.with_extension("json")
    };
    if source.exists() {
        let global: Value =
            serde_json::from_str(&std::fs::read_to_string(source).map_err(i18n::io)?)
                .map_err(i18n::io)?;
        let target = home.join(".claude.json");
        let mut local: Value = match std::fs::read_to_string(&target) {
            Ok(body) => serde_json::from_str(&body).map_err(i18n::io)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(error) => return Err(i18n::io(error)),
        };
        let local = local
            .as_object_mut()
            .ok_or_else(|| i18n::t("err.account.profile"))?;
        for name in ["mcpServers", "projects"] {
            if let Some(value) = global.get(name) {
                local.insert(name.into(), value.clone());
            }
        }
        paths::write_private(&target, &serde_json::to_string(local).map_err(i18n::io)?)
            .map_err(i18n::io)?;
    }
    Ok(())
}

pub fn login(
    profile: &accounts::Profile,
    cancel: Arc<AtomicBool>,
) -> Result<accounts::Identity, String> {
    let mut command = Command::new("claude");
    command
        .args(["auth", "login", "--claudeai"])
        .current_dir(paths::home());
    account_env(&mut command, profile);
    let mut process = accounts::AuthProcess::spawn(command, Duration::from_secs(600), cancel)?;
    while process.line()?.is_some() {}
    process.finish()?;
    let identity = account_status(profile)?;
    if !identity.connected {
        return Err(i18n::t("err.account.login"));
    }
    Ok(identity)
}

pub fn account_status(profile: &accounts::Profile) -> Result<accounts::Identity, String> {
    account_status_at(profile, &paths::home())
}

fn account_status_at(
    profile: &accounts::Profile,
    cwd: &Path,
) -> Result<accounts::Identity, String> {
    let mut command = Command::new("claude");
    command.args(["auth", "status", "--json"]).current_dir(cwd);
    account_env(&mut command, profile);
    let mut process = accounts::AuthProcess::spawn(
        command,
        Duration::from_secs(20),
        Arc::new(AtomicBool::new(false)),
    )?;
    let mut body = String::new();
    while let Some(line) = process.line()? {
        body.push_str(&line);
    }
    // auth status exits with code 1 when its JSON reports loggedIn=false.
    let value: Value = serde_json::from_str(&body).map_err(|_| i18n::t("err.account.status"))?;
    if profile.managed
        && value["loggedIn"] == true
        && (value["authMethod"] != "claude.ai" || value["apiProvider"] != "firstParty")
    {
        return Err(i18n::t("err.account.override"));
    }
    parse_account(&value)
}

fn parse_account(value: &Value) -> Result<accounts::Identity, String> {
    Ok(accounts::Identity {
        connected: value["loggedIn"]
            .as_bool()
            .ok_or_else(|| i18n::t("err.account.status"))?,
        email: value["email"].as_str().map(str::to_string),
        plan: value["subscriptionType"].as_str().map(str::to_string),
    })
}

/// Start Claude with provider-specific configuration and protocol adapters. The chat module owns
/// process lifecycle and stream pumping.
pub fn spawn(
    app: &AppHandle,
    id: &str,
    worktree: &Path,
    resume: bool,
    launch: &crate::session::Launch,
) -> Result<chat::Chat, String> {
    let args = launch_args(id, resume, launch, worktree)?;
    let profile = accounts::active(crate::state::ProviderId::Claude)?;
    profile.prepare()?;
    if profile.managed && !account_status_at(&profile, worktree)?.connected {
        return Err(i18n::t("err.account.disconnected"));
    }
    let mut cmd = Command::new("claude");
    cmd.args(args).current_dir(worktree);
    profile.apply(&mut cmd);
    let seed = paths::transcript(id, worktree);
    let wire = |stdin| {
        let mut adapter = Adapter::default();
        (
            chat::Wire::Claude(Link::new(stdin)),
            Box::new(move |line: &str| adapter.translate_line(line)) as chat::Translate,
        )
    };
    chat::launch(
        app,
        id,
        cmd,
        &seed,
        None,
        "err.chat.spawn",
        chat::ProcessIo::new(passthrough_stderr, wire, profile),
    )
}

/// Build headless stream-json arguments for a new or resumed session using the same ID. Route
/// permission prompts through stdio so the process can receive replies. Plan mode permits a later
/// switch to bypass, but must not start with the bypass flag because it overrides plan mode.
fn launch_args(
    id: &str,
    resume: bool,
    launch: &crate::session::Launch,
    worktree: &Path,
) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--permission-prompt-tool",
        "stdio",
        if resume { "--resume" } else { "--session-id" },
        id,
    ]
    .map(String::from)
    .to_vec();
    if launch.plan {
        args.extend(
            [
                "--permission-mode",
                "plan",
                "--allow-dangerously-skip-permissions",
            ]
            .map(String::from),
        );
    } else if launch.permission != Some(crate::actions::Permission::Ask) {
        args.push("--dangerously-skip-permissions".into());
    }
    if !launch.instructions.is_empty() {
        args.extend(["--append-system-prompt".into(), launch.instructions.clone()]);
    }
    if !launch.model.trim().is_empty() {
        args.extend(["--model".into(), launch.model.trim().into()]);
    }
    if !launch.effort.trim().is_empty() {
        args.extend(["--effort".into(), launch.effort.trim().into()]);
    }
    // An explicit MCP selection requires strict configuration; without one, retain the CLI
    // defaults. The strict file carries the whole effective set, including the CLI-inherited
    // servers the layers kept (ADR 0044). Materialization failures must prevent startup rather
    // than silently discard selected tools.
    if let Some(path) = crate::mcp::config_for(id, launch.mcp.as_ref(), worktree)? {
        args.extend([
            "--mcp-config".into(),
            path.display().to_string(),
            "--strict-mcp-config".into(),
        ]);
    }
    // Inject selected plugins through session flags without modifying the CLI registry. Claude owns
    // name-based deduplication with globally enabled plugins. Without a selection, leave those
    // defaults intact; Codex materializes its own configuration in its adapter. Standalone skills
    // ride the same plugin-package pipeline, so they materialize together with the plugins.
    let packages = launch.plugin_packages();
    args.extend(crate::plugins::args_for(packages.as_ref()));
    Ok(args)
}

fn passthrough_stderr(line: &str) -> Option<String> {
    Some(line.to_string())
}

/// The Claude process's stream-json input transport.
pub struct Link {
    stdin: ChildStdin,
}

impl Link {
    fn new(stdin: ChildStdin) -> Self {
        Self { stdin }
    }

    pub fn write(&mut self, frame: &Value, buffer: &str) -> Result<Vec<Value>, String> {
        let provider = command(frame, buffer).ok_or_else(|| i18n::t("err.team.bad"))?;
        let mut line = provider.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).map_err(i18n::io)?;
        self.stdin.flush().map_err(i18n::io)?;
        Ok(vec![])
    }
}

#[derive(Default)]
pub struct Adapter {
    message: String,
    next_block: usize,
    tools: HashMap<String, String>,
    pending_skill: Option<String>,
    tasks: HashMap<String, Value>,
    commands: Vec<Value>,
    terminal: HashSet<String>,
}

impl Adapter {
    pub fn translate_line(&mut self, line: &str) -> Vec<String> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return vec![];
        };
        self.translate(&value)
            .into_iter()
            .map(|event| event.to_string())
            .collect()
    }

    pub fn translate(&mut self, value: &Value) -> Vec<Value> {
        if value["v"] == 1 {
            return vec![value.clone()];
        }
        if value["prometheusV1Mirror"] == true
            || value["isSidechain"] == true
            || !value["parent_tool_use_id"].is_null()
        {
            return vec![];
        }
        let at = value["ts"].as_u64().unwrap_or_else(now);
        match value["type"].as_str() {
            Some("user") => self.user(value, at),
            Some("assistant") => self.assistant(value, at),
            Some("stream_event") => self.stream(value, at),
            Some("control_request") => self.request(value, at),
            Some("control_response") => self.command_list(value, at),
            Some("result") => vec![event(
                "turn.completed",
                at,
                json!({
                    "outcome": if value["is_error"] == true {
                        if turn_message(value).is_empty() { "interrupted" } else { "error" }
                    } else { "ok" },
                    "message": turn_message(value),
                    "durationMs": value["duration_ms"].as_u64(),
                    "costUsd": value["total_cost_usd"].as_f64(),
                }),
            )],
            Some("system") => self.system(value, at),
            // Read the historical discriminant for legacy imports only.
            Some("prometheus") => self.legacy_app(value, at),
            Some("rate_limit_event") => vec![event(
                "usage.updated",
                at,
                json!({ "provider": "claude", "usage": value["rate_limit_info"] }),
            )],
            _ => vec![],
        }
    }

    fn user(&mut self, value: &Value, at: u64) -> Vec<Value> {
        let content = &value["message"]["content"];
        if let Some(blocks) = content.as_array() {
            let mut out = vec![];
            let mut texts = vec![];
            for block in blocks {
                match block["type"].as_str() {
                    Some("tool_result") => {
                        let tool_id = block["tool_use_id"].as_str().unwrap_or("");
                        if tool_id.is_empty() {
                            continue;
                        }
                        out.push(event(
                            "tool.completed",
                            at,
                            json!({
                                "toolId": tool_id,
                                "output": result_text(&block["content"]),
                                "error": block["is_error"] == true,
                                "background": self.tasks.values().any(|task| task["toolId"] == tool_id),
                            }),
                        ));
                        if self.tools.get(tool_id).is_some_and(|name| name == "Skill")
                            && block["is_error"] != true
                        {
                            self.pending_skill = Some(tool_id.to_string());
                        }
                    }
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            texts.push(text.to_string());
                        }
                    }
                    Some("image") => texts.push("[imagem]".to_string()),
                    _ => {}
                }
            }
            if !texts.is_empty() {
                out.extend(self.spoken(&texts.join("\n\n"), value, at));
            }
            return out;
        }
        content
            .as_str()
            .map(|text| self.spoken(text, value, at))
            .unwrap_or_default()
    }

    fn spoken(&mut self, text: &str, value: &Value, at: u64) -> Vec<Value> {
        if let Some(tool_id) = self.pending_skill.clone() {
            let source = value["sourceToolUseID"].as_str();
            if (value["isSynthetic"] == true || value["isMeta"] == true)
                && source.is_none_or(|source| source == tool_id)
            {
                self.pending_skill = None;
                return vec![event(
                    "tool.completed",
                    at,
                    json!({ "toolId": tool_id, "output": text, "error": false, "background": false }),
                )];
            }
        }
        let trim = text.trim_start();
        if value["isMeta"] == true
            || trim.starts_with("<command-name>")
            || trim.starts_with("<local-command-stdout>")
            || trim.starts_with("<local-command-caveat>")
        {
            return vec![];
        }
        if value["isCompactSummary"] == true
            || text.starts_with("This session is being continued from a previous conversation")
        {
            return vec![event("system.summary", at, json!({ "text": text }))];
        }
        if trim.starts_with("<task-notification>") {
            if let Some(summary) = between(text, "<summary>", "</summary>") {
                return vec![event(
                    "system.notice",
                    at,
                    json!({ "level": "info", "code": "background.completed", "detail": summary.trim() }),
                )];
            }
        }
        (!text.trim().is_empty())
            .then(|| {
                event(
                    "user.message",
                    at,
                    json!({ "content": [{ "kind": "text", "text": text }] }),
                )
            })
            .into_iter()
            .collect()
    }

    fn assistant(&mut self, value: &Value, at: u64) -> Vec<Value> {
        self.pending_skill = None;
        let message = &value["message"];
        if message["model"] == "<synthetic>" {
            let markdown = message["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|block| block["type"] == "text"))
                .and_then(|block| block["text"].as_str());
            return markdown
                .map(|markdown| event("context.reported", at, json!({ "markdown": markdown })))
                .into_iter()
                .collect();
        }
        let message_id = message["id"]
            .as_str()
            .or_else(|| value["uuid"].as_str())
            .unwrap_or("");
        if message_id.is_empty() {
            return vec![];
        }
        self.select_message(message_id);
        let mut out = vec![];
        for raw in message["content"].as_array().into_iter().flatten() {
            let Some(block) = block(raw) else { continue };
            if block["kind"] == "tool" {
                if let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) {
                    self.tools.insert(id.to_string(), name.to_string());
                }
            }
            out.push(event(
                "assistant.block",
                at,
                json!({ "messageId": message_id, "index": self.next_block, "block": block }),
            ));
            self.next_block += 1;
        }
        out
    }

    fn stream(&mut self, value: &Value, at: u64) -> Vec<Value> {
        let raw = &value["event"];
        if raw["type"] == "message_start" {
            let message_id = raw["message"]["id"].as_str().unwrap_or("");
            if message_id.is_empty() {
                return vec![];
            }
            self.message = message_id.to_string();
            self.next_block = 0;
            return vec![event(
                "assistant.started",
                at,
                json!({ "messageId": message_id }),
            )];
        }
        let Some(index) = raw["index"].as_u64() else {
            return vec![];
        };
        if self.message.is_empty() {
            return vec![];
        }
        if raw["type"] == "content_block_start" {
            let Some(block) = block(&raw["content_block"]) else {
                return vec![];
            };
            if block["kind"] == "tool" {
                if let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) {
                    self.tools.insert(id.to_string(), name.to_string());
                }
            }
            return vec![event(
                "assistant.block.started",
                at,
                json!({ "messageId": self.message, "index": index, "block": block }),
            )];
        }
        if raw["type"] != "content_block_delta" {
            return vec![];
        }
        let delta = &raw["delta"];
        match delta["type"].as_str() {
            Some("text_delta") => vec![event(
                "assistant.delta",
                at,
                json!({ "messageId": self.message, "index": index, "kind": "text", "delta": delta["text"].as_str().unwrap_or("") }),
            )],
            Some("thinking_delta") => vec![event(
                "assistant.delta",
                at,
                json!({ "messageId": self.message, "index": index, "kind": "thinking", "delta": delta["thinking"].as_str().unwrap_or("") }),
            )],
            Some("input_json_delta") => vec![event(
                "tool.input.delta",
                at,
                json!({
                    "messageId": self.message,
                    "index": index,
                    "toolId": raw["tool_use_id"].as_str().unwrap_or(""),
                    "delta": delta["partial_json"].as_str().unwrap_or(""),
                }),
            )],
            _ => vec![],
        }
    }

    fn select_message(&mut self, message_id: &str) {
        if self.message != message_id {
            self.message = message_id.to_string();
            self.next_block = 0;
        }
    }

    fn request(&self, value: &Value, at: u64) -> Vec<Value> {
        let request = &value["request"];
        let request_id = value["request_id"].as_str().unwrap_or("");
        if request["subtype"] != "can_use_tool" || request_id.is_empty() {
            return vec![];
        }
        let tool = request["tool_name"].as_str();
        let kind = match tool {
            Some("AskUserQuestion") => "question",
            Some("ExitPlanMode") => "plan",
            _ => "approval",
        };
        vec![event(
            "request.opened",
            at,
            json!({
                "requestId": request_id,
                "kind": kind,
                "toolId": request["tool_use_id"].as_str(),
                "tool": tool,
                "input": request["input"].as_object().cloned().unwrap_or_default(),
            }),
        )]
    }

    fn command_list(&mut self, value: &Value, at: u64) -> Vec<Value> {
        let Some(commands) = value
            .pointer("/response/response/commands")
            .and_then(Value::as_array)
        else {
            return vec![];
        };
        self.commands = commands
            .iter()
            .filter_map(|command| {
                Some(json!({
                    "name": command["name"].as_str()?,
                    "description": command["description"].as_str().unwrap_or(""),
                    "hint": command["argumentHint"].as_str().unwrap_or(""),
                }))
            })
            .collect();
        vec![self.commands_event(at)]
    }

    fn system(&mut self, value: &Value, at: u64) -> Vec<Value> {
        match value["subtype"].as_str() {
            Some("init") => {
                self.terminal = value["terminal_slash_commands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                vec![self.commands_event(at)]
            }
            Some("status") => vec![event(
                "context.compaction",
                at,
                json!({
                    "state": if value["compact_result"] == "failed" { "failed" } else if value["status"] == "compacting" { "started" } else { "stopped" },
                    "detail": value["compact_error"].as_str().unwrap_or(""),
                }),
            )],
            Some("compact_boundary") => vec![event(
                "context.compacted",
                at,
                json!({
                    "before": value["compact_metadata"]["pre_tokens"].as_u64(),
                    "after": value["compact_metadata"]["post_tokens"].as_u64(),
                }),
            )],
            Some("task_started") => {
                let id = value["task_id"].as_str().unwrap_or("");
                if id.is_empty() {
                    return vec![];
                }
                self.tasks.insert(
                    id.to_string(),
                    json!({
                        "id": id,
                        "description": value["description"].as_str().unwrap_or(""),
                        "toolId": value["tool_use_id"].as_str(),
                    }),
                );
                vec![self.background(at)]
            }
            Some("background_tasks_changed") => {
                let mut next = HashMap::new();
                for raw in value["tasks"].as_array().into_iter().flatten() {
                    let id = raw["task_id"].as_str().unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    next.insert(
                        id.to_string(),
                        self.tasks.get(id).cloned().unwrap_or_else(|| {
                            json!({ "id": id, "description": raw["description"].as_str().unwrap_or(""), "toolId": null })
                        }),
                    );
                }
                self.tasks = next;
                vec![self.background(at)]
            }
            Some("task_notification") => {
                if let Some(id) = value["task_id"].as_str() {
                    self.tasks.remove(id);
                }
                let mut out = vec![self.background(at)];
                let detail = value["summary"].as_str().unwrap_or("").trim();
                if !detail.is_empty() {
                    out.push(event(
                        "system.notice",
                        at,
                        json!({
                            "level": if value["status"] == "completed" { "info" } else { "error" },
                            "code": "background.completed",
                            "detail": detail,
                        }),
                    ));
                }
                out
            }
            _ => vec![],
        }
    }

    fn legacy_app(&self, value: &Value, at: u64) -> Vec<Value> {
        let mapped = match value["subtype"].as_str() {
            Some("stderr") => event(
                "system.notice",
                at,
                json!({ "level": "error", "code": "provider.stderr", "detail": value["text"].as_str().unwrap_or("") }),
            ),
            Some("state") => event(
                "session.state",
                at,
                json!({ "state": if value["busy"] == true { "busy" } else { "ready" } }),
            ),
            Some("tokens") => event(
                "context.updated",
                at,
                json!({ "used": value["tokens"].as_u64().unwrap_or(0), "window": null }),
            ),
            Some("session") => event(
                "session.identity",
                at,
                json!({ "providerSession": value["session"].as_str().unwrap_or("") }),
            ),
            Some("usage") => event(
                "usage.updated",
                at,
                json!({ "provider": "codex", "usage": value["usage"] }),
            ),
            _ => return vec![],
        };
        vec![mapped]
    }

    fn commands_event(&self, at: u64) -> Value {
        let commands: Vec<&Value> = self
            .commands
            .iter()
            .filter(|command| {
                command["name"]
                    .as_str()
                    .is_some_and(|name| !self.terminal.contains(name))
            })
            .collect();
        event("commands.updated", at, json!({ "commands": commands }))
    }

    fn background(&self, at: u64) -> Value {
        event(
            "background.changed",
            at,
            json!({ "tasks": self.tasks.values().collect::<Vec<_>>() }),
        )
    }
}

/// Translate canonical commands into Claude stream-json input. Legacy commands remain unchanged for
/// rollback compatibility.
pub fn command(frame: &Value, buffer: &str) -> Option<Value> {
    if frame["v"] != 1 {
        return Some(frame.clone());
    }
    match frame["type"].as_str()? {
        "message.send" => Some(json!({
            "type": "user",
            "message": { "role": "user", "content": frame["text"].as_str()? },
        })),
        "turn.interrupt" => Some(json!({
            "type": "control_request",
            "request_id": format!("interrupt-{}", now()),
            "request": { "subtype": "interrupt" },
        })),
        "permission.mode.set" if frame["mode"] == "bypass" => Some(json!({
            "type": "control_request",
            "request_id": format!("permission-{}", now()),
            "request": { "subtype": "set_permission_mode", "mode": "bypassPermissions" },
        })),
        "commands.list" => Some(json!({
            "type": "control_request",
            "request_id": "initialize",
            "request": { "subtype": "initialize" },
        })),
        "request.respond" => {
            let id = frame["requestId"].as_str()?;
            let request = request_in(buffer, id)?;
            let response = &frame["response"];
            let answer = match response["outcome"].as_str()? {
                "allow" => json!({ "behavior": "allow", "updatedInput": request["input"] }),
                "answer" => {
                    let mut input = request["input"].clone();
                    input
                        .as_object_mut()?
                        .insert("answers".into(), response["answers"].clone());
                    json!({ "behavior": "allow", "updatedInput": input })
                }
                "deny" => json!({
                    "behavior": "deny",
                    "message": response["message"].as_str().unwrap_or("Denied"),
                }),
                _ => return None,
            };
            Some(json!({
                "type": "control_response",
                "response": { "subtype": "success", "request_id": id, "response": answer },
            }))
        }
        _ => None,
    }
}

fn request_in(buffer: &str, id: &str) -> Option<Value> {
    for line in buffer.lines().rev() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value["v"] == 1 && value["type"] == "request.closed" && value["requestId"] == id {
            return None;
        }
        if value["v"] == 1 && value["type"] == "request.opened" && value["requestId"] == id {
            return Some(value);
        }
        if value["type"] == "control_request"
            && value["request_id"] == id
            && value["request"]["subtype"] == "can_use_tool"
        {
            return Some(json!({
                "requestId": id,
                "tool": value["request"]["tool_name"],
                "toolId": value["request"]["tool_use_id"],
                "input": value["request"]["input"],
            }));
        }
    }
    None
}

fn block(value: &Value) -> Option<Value> {
    match value["type"].as_str()? {
        "text" => Some(json!({ "kind": "text", "text": value["text"].as_str().unwrap_or("") })),
        "thinking" => {
            Some(json!({ "kind": "thinking", "text": value["thinking"].as_str().unwrap_or("") }))
        }
        "tool_use" => Some(json!({
            "kind": "tool",
            "id": value["id"].as_str().unwrap_or(""),
            "name": value["name"].as_str().unwrap_or(""),
            "input": value["input"],
        })),
        _ => None,
    }
}

fn result_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| match part["type"].as_str() {
            Some("text") => part["text"].as_str().map(str::to_string),
            Some("image") => Some("[imagem]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn turn_message(value: &Value) -> String {
    let errors = value["errors"]
        .as_array()
        .map(|errors| {
            errors
                .iter()
                .filter_map(|error| error.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !errors.is_empty() {
        errors.join("\n")
    } else if value["is_error"] == true {
        value["result"].as_str().unwrap_or("").to_string()
    } else {
        String::new()
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = text.split_once(start)?.1;
    Some(rest.split_once(end)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_stream_e_bloco_final() {
        let mut adapter = Adapter::default();
        let started = adapter.translate(&json!({
            "type": "stream_event",
            "ts": 10,
            "event": { "type": "message_start", "message": { "id": "m1" } },
        }));
        assert_eq!(started[0]["type"], "assistant.started");
        let final_block = adapter.translate(&json!({
            "type": "assistant",
            "ts": 11,
            "message": { "id": "m1", "content": [{ "type": "text", "text": "oi" }] },
        }));
        assert_eq!(final_block[0]["type"], "assistant.block");
        assert_eq!(final_block[0]["index"], 0);
        assert_eq!(final_block[0]["block"]["text"], "oi");
    }

    #[test]
    fn comando_de_resposta_reusa_o_input_do_pedido() {
        let buffer = event(
            "request.opened",
            1,
            json!({
                "requestId": "r1",
                "kind": "approval",
                "toolId": "t1",
                "tool": "Bash",
                "input": { "command": "rm arquivo" },
            }),
        )
        .to_string();
        let response = json!({
            "v": 1,
            "type": "request.respond",
            "requestId": "r1",
            "response": { "outcome": "allow" },
        });
        let provider = command(&response, &buffer).unwrap();
        assert_eq!(
            provider.pointer("/response/response/updatedInput/command"),
            Some(&json!("rm arquivo"))
        );

        let closed = format!(
            "{}\n{}",
            buffer,
            event(
                "request.closed",
                2,
                json!({ "requestId": "r1", "outcome": "allowed" })
            )
        );
        assert!(command(&response, &closed).is_none());
    }

    #[test]
    fn evento_desconhecido_nao_vira_conversa() {
        assert!(Adapter::default()
            .translate(&json!({ "type": "provider/new-event" }))
            .is_empty());
    }

    #[test]
    fn normaliza_pedido_background_e_compactacao() {
        let mut adapter = Adapter::default();
        let request = adapter.translate(&json!({
            "type": "control_request",
            "ts": 1,
            "request_id": "r1",
            "request": { "subtype": "can_use_tool", "tool_name": "Bash", "tool_use_id": "t1", "input": { "command": "ls" } },
        }));
        assert_eq!(request[0]["type"], "request.opened");
        assert_eq!(request[0]["kind"], "approval");

        let background = adapter.translate(&json!({
            "type": "system",
            "subtype": "task_started",
            "ts": 2,
            "task_id": "bg1",
            "tool_use_id": "t1",
            "description": "mapear",
        }));
        assert_eq!(background[0]["type"], "background.changed");
        assert_eq!(background[0]["tasks"][0]["toolId"], "t1");

        let compact = adapter.translate(&json!({
            "type": "system",
            "subtype": "compact_boundary",
            "ts": 3,
            "compact_metadata": { "pre_tokens": 100, "post_tokens": 20 },
        }));
        assert_eq!(compact[0]["type"], "context.compacted");
        assert_eq!(compact[0]["before"], 100);
        assert_eq!(compact[0]["after"], 20);
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    #[ignore = "precisa do Claude Code instalado; não faz login nem envia prompts"]
    fn perfil_vazio_nao_herda_login_do_terminal() {
        let id = uuid::Uuid::new_v4().to_string();
        let home = std::env::temp_dir().join(format!("prometeu-claude-auth-{id}"));
        paths::ensure_private_dir(&home).unwrap();
        let profile = accounts::Profile {
            id,
            provider: crate::state::ProviderId::Claude,
            home: home.clone(),
            managed: true,
            revision: 0,
        };
        let result = account_status(&profile);
        std::fs::remove_dir_all(home).unwrap();
        assert!(!result.unwrap().connected);
    }

    #[test]
    fn contas_compartilham_transcript_e_plugins_sem_copiar_login() {
        let root =
            std::env::temp_dir().join(format!("prometeu-claude-accounts-{}", uuid::Uuid::new_v4()));
        let base = root.join("base");
        let first = root.join("first");
        let second = root.join("second");
        for home in [&base, &first, &second] {
            paths::ensure_private_dir(home).unwrap();
        }
        std::fs::write(base.join(".credentials.json"), "login original").unwrap();
        std::fs::write(base.join(".claude.json"), r#"{"oauthAccount":{"email":"original@example.com"},"mcpServers":{"local":{"command":"echo"}},"projects":{"/repo":{"hasTrustDialogAccepted":true}}}"#).unwrap();
        std::fs::write(base.join("settings.json"), r#"{"env":{"ANTHROPIC_API_KEY":"segredo","CLAUDE_CONFIG_DIR":"/outra-conta","KEEP":"sim"},"apiKeyHelper":"outra-chave","enabledPlugins":{"teste":true}}"#).unwrap();
        prepare_profile_at(&base, &first).unwrap();
        prepare_profile_at(&base, &second).unwrap();
        std::fs::write(first.join(".credentials.json"), "primeira conta").unwrap();
        std::fs::write(first.join("projects/conversa.jsonl"), "transcript completo").unwrap();
        prepare_profile_at(&base, &first).unwrap();
        assert_eq!(
            std::fs::read_to_string(second.join("projects/conversa.jsonl")).unwrap(),
            "transcript completo"
        );
        assert_eq!(
            std::fs::read_link(second.join("plugins")).unwrap(),
            base.join("plugins")
        );
        assert_eq!(
            std::fs::read_to_string(first.join(".credentials.json")).unwrap(),
            "primeira conta"
        );
        assert!(!second.join(".credentials.json").exists());
        assert_eq!(
            std::fs::read_to_string(base.join(".credentials.json")).unwrap(),
            "login original"
        );
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(first.join("settings.json")).unwrap())
                .unwrap();
        assert!(settings["apiKeyHelper"].is_null());
        assert!(settings["env"]["ANTHROPIC_API_KEY"].is_null());
        assert_eq!(settings["env"]["KEEP"], "sim");
        let config: Value =
            serde_json::from_str(&std::fs::read_to_string(first.join(".claude.json")).unwrap())
                .unwrap();
        assert!(config["oauthAccount"].is_null());
        assert_eq!(config["mcpServers"]["local"]["command"], "echo");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_261_expoe_somente_identidade_da_conta() {
        // Sanitized shape captured from claude auth status --json 2.1.261.
        let value = json!({"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","analyticsDisabled":false,"projectsDirectory":"/privado/projects","email":"pessoa@example.com","orgId":"org-teste","orgName":"Time","subscriptionType":"max"});
        let identity = parse_account(&value).unwrap();
        assert!(identity.connected);
        assert_eq!(identity.email.as_deref(), Some("pessoa@example.com"));
        assert!(!serde_json::to_string(&identity)
            .unwrap()
            .contains("/privado"));
        assert!(!parse_account(&json!({"loggedIn":false})).unwrap().connected);
        assert!(parse_account(&json!({"error":"indisponível"})).is_err());
    }
}

#[cfg(test)]
mod launch_tests {
    use super::launch_args;
    use crate::paths;
    use crate::session::Launch;
    use crate::state::ProviderId;
    use std::path::Path;
    use std::process::Command;

    /// A directory without `.mcp.json`; the strict-config test also redirects HOME in its child
    /// process, so CLI-inherited discovery (ADR 0044) finds nothing in these tests.
    fn work() -> &'static Path {
        Path::new("/prometeu-launch-test")
    }

    fn launch(model: &str, effort: &str, plan: bool) -> Launch {
        Launch {
            mcp: None,
            plugins: None,
            agent: ProviderId::Claude,
            model: model.into(),
            effort: effort.into(),
            plan,
            ..Default::default()
        }
    }

    #[test]
    fn task_permissions_and_instructions_reach_claude() {
        let launch = Launch {
            permission: Some(crate::actions::Permission::Ask),
            instructions: "Review independently".into(),
            ..Default::default()
        };
        let args = launch_args("id", true, &launch, work()).unwrap();
        assert!(!args.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--append-system-prompt", "Review independently"]));
        let automatic = Launch {
            permission: Some(crate::actions::Permission::Auto),
            ..launch
        };
        assert!(launch_args("id", true, &automatic, work())
            .unwrap()
            .contains(&"--dangerously-skip-permissions".to_string()));
    }

    /// Without an explicit MCP selection, preserve CLI defaults. A selection supplies both the
    /// generated file and strict configuration.
    #[test]
    fn mcp_so_entra_quando_alguem_escolheu() {
        // Use the existing subprocess pattern instead of changing the shared test environment.
        if std::env::var("PROMETEU_CLAUDE_MCP_TEST_CHILD").as_deref() != Ok("1") {
            let root = std::env::temp_dir().join(format!("prometeu-mcp-{}", uuid::Uuid::new_v4()));
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "claude::launch_tests::mcp_so_entra_quando_alguem_escolheu",
                    "--nocapture",
                ])
                .env("PROMETEU_CLAUDE_MCP_TEST_CHILD", "1")
                .env("PROMETEU_ROOT", &root)
                .env("HOME", &root)
                .output()
                .unwrap();
            std::fs::remove_dir_all(&root).ok();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let sem = launch_args("id", false, &launch("", "", false), work()).unwrap();
        assert!(!sem.contains(&"--mcp-config".to_string()));
        assert!(!sem.contains(&"--strict-mcp-config".to_string()));

        // A chosen id must exist in the universe: materialization fails instead of silently
        // starting without the requested server.
        crate::mcp::store(&[crate::mcp::Server {
            id: "notion".into(),
            config: serde_json::json!({ "command": "npx" }),
            note: String::new(),
        }])
        .expect("hub");
        let escolheu = Launch {
            mcp: Some(vec!["notion".into()]),
            ..launch("", "", false)
        };
        let args = launch_args("id", false, &escolheu, work()).unwrap();
        let at = args
            .iter()
            .position(|a| a == "--mcp-config")
            .expect("o arquivo");
        assert!(std::path::Path::new(&args[at + 1]).exists());
        assert!(std::path::Path::new(&args[at + 1]).starts_with(paths::root()));
        assert!(args.contains(&"--strict-mcp-config".to_string()));
    }

    /// Without a plugin selection, preserve CLI defaults. Selected-plugin flag coverage belongs to
    /// plugins.rs.
    #[test]
    fn sem_escolha_nao_ha_flag_de_plugin() {
        let args = launch_args("id", false, &launch("", "", false), work()).unwrap();
        assert!(!args.contains(&"--plugin-dir".to_string()));
        assert!(!args.contains(&"--plugin-url".to_string()));
    }

    /// Bypass overrides plan mode, so plan mode must use the allow flag without enabling bypass
    /// immediately.
    #[test]
    fn plan_mode_nao_leva_o_bypass_junto() {
        let solto = launch_args("id", false, &launch("", "", false), work()).unwrap();
        assert!(solto.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(!solto.contains(&"--permission-mode".to_string()));

        let plano = launch_args("id", false, &launch("", "", true), work()).unwrap();
        assert!(!plano.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(plano.contains(&"--allow-dangerously-skip-permissions".to_string()));
        let at = plano.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(plano[at + 1], "plan");
    }

    /// Omit empty model and effort flags so Claude can choose its defaults.
    #[test]
    fn modelo_e_esforco_so_quando_escolhidos() {
        let padrao = launch_args("id", true, &launch("", " ", false), work()).unwrap();
        assert!(!padrao.contains(&"--model".to_string()));
        assert!(!padrao.contains(&"--effort".to_string()));
        let at = padrao.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(padrao[at + 1], "id");

        let escolhido =
            launch_args("id", false, &launch("opus[1m]", "max", false), work()).unwrap();
        assert_eq!(
            escolhido[escolhido.len() - 4..],
            ["--model", "opus[1m]", "--effort", "max"]
        );
        let at = escolhido.iter().position(|a| a == "--session-id").unwrap();
        assert_eq!(escolhido[at + 1], "id");
    }

    /// Keep conversation input, output, and permission requests on the same stream-json transport.
    #[test]
    fn a_conversa_e_stream_json_com_permissao_por_stdio() {
        let args = launch_args("id", false, &launch("", "", false), work()).unwrap();
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert_eq!(args[0], "-p");
        assert!(has(["--input-format", "stream-json"]));
        assert!(has(["--output-format", "stream-json"]));
        assert!(has(["--permission-prompt-tool", "stdio"]));
        assert!(args.contains(&"--include-partial-messages".to_string()));
    }
}
