//! Adapt Codex app-server JSON-RPC over stdio into canonical ConversationEventV1 events and
//! translate commands in the opposite direction. Persist the returned thread ID in
//! Tab.agent_session for resume. The app stores its rendered transcript separately from native
//! Codex rollouts. Implement /compact through thread/compact/start and /context from tokenUsage;
//! reject unsupported slash commands. Default sessions use approvalPolicy never and an unrestricted
//! sandbox. Enable default_mode_request_user_input so ordinary turns can ask questions. Link
//! remains independent of threads and AppHandle for protocol tests.

use crate::lock::lock;
use crate::session::Launch;
use crate::{accounts, agents, chat, conversation, i18n, paths, plugins};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

mod account;
pub use account::{account_env, account_probe, login, prepare_profile, user_home};

/// Start codex app-server for a tab. Resume uses the previously returned thread ID; absence starts
/// a new conversation.
pub fn spawn(
    app: &AppHandle,
    id: &str,
    workspace: &str,
    worktree: &Path,
    resume: Option<String>,
    launch: &Launch,
) -> Result<chat::Chat, String> {
    let profile = accounts::active(crate::state::ProviderId::Codex)?;
    profile.prepare()?;
    // Standalone skills ride the plugin-package pipeline, so Codex materializes them together with
    // the selected plugins.
    let packages = launch.plugin_packages();
    let selected_plugins = plugins::codex_for(
        launch.config_scope.as_deref().unwrap_or(workspace),
        packages.as_ref(),
        &profile,
    )?;
    let mut cmd = Command::new("codex");
    profile.apply(&mut cmd);
    cmd.args(["app-server", "--enable", "default_mode_request_user_input"]);
    cmd.args(["-c", "suppress_unstable_features_warning=true"]);
    if let Some(home) = &selected_plugins.home {
        cmd.env("CODEX_HOME", home);
    }
    // Sessions without plugins must not require newer plugin features, preserving conversation and
    // MCP support on older installations.
    if !selected_plugins.ids.is_empty() {
        cmd.args(["--enable", "plugins", "--enable", "hooks"]);
    }
    cmd.current_dir(worktree);
    // Inject selected MCP configuration through -c and process environment secrets; see
    // mcp::codex_config. Materialization failure must stop startup rather than silently omit
    // selected tools.
    if let Some((servers, env)) = crate::mcp::codex_config(id, launch.mcp.as_ref())? {
        cmd.args(["-c", &format!("mcp_servers={servers}")]);
        for (key, value) in env {
            cmd.env(key, value);
        }
    }
    let log = paths::chat_log(id);
    let start = Start {
        cwd: worktree.display().to_string(),
        resume,
        model: launch.model.trim().to_string(),
        effort: agents::effort(&launch.effort).to_string(),
        plugin_ids: selected_plugins.ids,
        plugin_hook_ids: selected_plugins.hook_ids,
        permission: launch.permission,
        instructions: launch.instructions.clone(),
    };
    let io = chat::ProcessIo::new(
        process_stderr,
        move |stdin| {
            let link = Arc::new(Mutex::new(Link::new(Box::new(stdin), start)));
            let reader = link.clone();
            let translate = move |line: &str| {
                lock(&reader)
                    .on_line(line)
                    .iter()
                    .map(|event| event.to_string())
                    .collect()
            };
            (
                chat::Wire::Codex(link),
                Box::new(translate) as chat::Translate,
            )
        },
        profile,
    );
    chat::launch(app, id, cmd, &log, Some(log.clone()), "err.codex.spawn", io)
}

/// Thread startup settings.
pub struct Start {
    pub permission: Option<crate::actions::Permission>,
    pub instructions: String,
    pub cwd: String,
    pub resume: Option<String>,
    /// An empty model lets Codex choose its default.
    pub model: String,
    /// Use Codex's native effort name, ultra rather than ultracode. Omit empty values.
    pub effort: String,
    /// Explicitly selected plugin@marketplace IDs. Only their hooks may gain trust during the
    /// handshake.
    pub plugin_ids: Vec<String>,
    /// Selected plugins declaring hooks must have those hooks active before the thread opens.
    /// Missing hooks must not silently degrade to skills only.
    pub plugin_hook_ids: Vec<String>,
}

/// Track the purpose of each outstanding request to interpret its response.
enum Sent {
    Init,
    /// If thread/resume fails, start a new conversation and notify the person instead of leaving
    /// the tab unusable.
    Thread {
        resumed: bool,
    },
    Turn,
    Compact,
    Interrupt,
    /// Read account/rateLimits/read at startup because change notifications alone cannot populate
    /// usage before the first turn.
    Usage,
    /// Discover selected plugins' hooks before opening the thread.
    Hooks,
    /// Persist the current hashes authorized by explicit plugin selection.
    HookTrust,
}

/// A server request awaiting a UI response.
struct Ask {
    /// Preserve the server's JSON-RPC request ID for the reply.
    rpc: Value,
    kind: AskKind,
}

enum AskKind {
    Command,
    Patch,
    /// Question text for the UI and IDs expected by Codex.
    Input(Vec<(String, String)>),
}

/// A streaming text or reasoning block.
struct Open {
    item: String,
    index: usize,
    thinking: bool,
    text: String,
}

pub struct Link {
    out: Box<dyn Write + Send>,
    start: Start,
    next: u64,
    sent: HashMap<u64, Sent>,
    thread: Option<String>,
    /// Retain a thread startup failure and reject subsequent messages with its reason.
    failed: Option<String>,
    /// Queue messages received before the thread exists and deliver them in order after startup.
    queue: Vec<Value>,
    model: String,
    window: Option<u64>,
    /// Current conversation size from the latest tokenUsage update.
    ctx: Option<u64>,
    turn: Option<String>,
    asks: HashMap<String, Ask>,
    /// Keep known identities after completion so child restarts cannot change the primary turn.
    subagents: HashMap<String, (Value, bool)>,
    /// Retain files from open fileChange items because approval requests do not repeat them.
    patches: HashMap<String, Vec<String>>,
    /// Assign streaming drafts the same block indexes used by final assistant.block events.
    block: usize,
    message_open: bool,
    open: Option<Open>,
    /// Conversation size when compaction started.
    compact_pre: Option<u64>,
}

impl Link {
    pub fn new(out: Box<dyn Write + Send>, start: Start) -> Link {
        let mut link = Link {
            out,
            start,
            next: 0,
            sent: HashMap::new(),
            thread: None,
            failed: None,
            queue: vec![],
            model: String::new(),
            window: None,
            ctx: None,
            turn: None,
            asks: HashMap::new(),
            subagents: HashMap::new(),
            patches: HashMap::new(),
            block: 0,
            message_open: false,
            open: None,
            compact_pre: None,
        };
        let _ = link.call(
            "initialize",
            json!({
                "clientInfo": { "name": "prometeu", "title": "Prometeu", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true },
            }),
            Sent::Init,
        );
        link
    }

    /// Close stdin to signal process shutdown.
    pub fn close(&mut self) {
        self.out = Box::new(std::io::sink());
    }

    /* UI commands to Codex */

    /// Handle a canonical UI command and return any local events that do not require process input.
    pub fn write(&mut self, frame: &Value) -> Result<Vec<Value>, String> {
        if frame["v"] != 1 {
            return Err(i18n::t("err.team.bad"));
        }
        match frame["type"].as_str() {
            Some("message.send") => {
                if let Some(cause) = &self.failed {
                    return Err(i18n::ta("err.codex.thread", &[("cause", cause.clone())]));
                }
                let text = frame["text"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.team.bad"))?;
                if self.thread.is_none() {
                    self.queue.push(frame.clone());
                    return Ok(vec![]);
                }
                self.speak(text)
            }
            Some("request.respond") => {
                let id = frame["requestId"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.team.bad"))?
                    .to_string();
                let Some(ask) = self.asks.get(&id) else {
                    return Err(i18n::t("err.team.bad"));
                };
                let answer = &frame["response"];
                let outcome = answer["outcome"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.team.bad"))?;
                let allowed = outcome == "allow";
                let result = match &ask.kind {
                    AskKind::Command | AskKind::Patch => {
                        if !matches!(outcome, "allow" | "deny") {
                            return Err(i18n::t("err.team.bad"));
                        }
                        json!({ "decision": if allowed { "accept" } else { "decline" } })
                    }
                    AskKind::Input(questions) => {
                        if !matches!(outcome, "answer" | "deny") {
                            return Err(i18n::t("err.team.bad"));
                        }
                        let given = &answer["answers"];
                        let mut answers = serde_json::Map::new();
                        for (question, qid) in questions {
                            let text = given[&question].as_str().unwrap_or("").trim().to_string();
                            let list: Vec<String> =
                                if text.is_empty() { vec![] } else { vec![text] };
                            answers.insert(qid.clone(), json!({ "answers": list }));
                        }
                        json!({ "answers": answers })
                    }
                };
                let rpc = ask.rpc.clone();
                self.asks.remove(&id);
                self.reply(&rpc, result)?;
                Ok(vec![])
            }
            // Advertise locally supported slash commands; the app-server does not provide this
            // catalog.
            Some("commands.list") => {
                let commands: Vec<Value> = SLASH
                    .iter()
                    .map(|(name, pt, en)| json!({ "name": name, "description": i18n::pick(pt, en), "hint": "" }))
                    .collect();
                Ok(vec![canonical(
                    "commands.updated",
                    json!({ "commands": commands }),
                )])
            }
            Some("turn.interrupt") => {
                if let (Some(thread), Some(turn)) = (self.thread.clone(), self.turn.clone()) {
                    self.call(
                        "turn/interrupt",
                        json!({ "threadId": thread, "turnId": turn }),
                        Sent::Interrupt,
                    )?;
                }
                Ok(vec![])
            }
            // Codex already starts with approvalPolicy never, so this capability is not offered.
            Some("permission.mode.set") if frame["mode"] == "bypass" => Ok(vec![]),
            _ => Err(i18n::t("err.team.bad")),
        }
    }

    /// Recognize leading slash command names while preserving absolute paths as ordinary message
    /// text.
    fn speak(&mut self, text: &str) -> Result<Vec<Value>, String> {
        let text = text.trim();
        if let Some(cmd) = text.strip_prefix('/') {
            let cmd = cmd.split_whitespace().next().unwrap_or("");
            if !cmd.contains('/') {
                return self.slash(cmd);
            }
        }
        let mut params = json!({
            "threadId": self.thread.clone().unwrap_or_default(),
            "input": [{ "type": "text", "text": text, "text_elements": [] }],
            "summary": "auto",
        });
        if !self.start.effort.is_empty() {
            params["effort"] = Value::String(self.start.effort.clone());
        }
        self.call("turn/start", params, Sent::Turn)?;
        Ok(vec![])
    }

    fn slash(&mut self, cmd: &str) -> Result<Vec<Value>, String> {
        match cmd {
            "compact" => {
                let thread = self.thread.clone().unwrap_or_default();
                self.call(
                    "thread/compact/start",
                    json!({ "threadId": thread }),
                    Sent::Compact,
                )?;
                Ok(vec![canonical(
                    "context.compaction",
                    json!({ "state": "started", "detail": "" }),
                )])
            }
            "context" => Ok(vec![
                canonical(
                    "context.reported",
                    json!({ "markdown": self.context_report() }),
                ),
                turn_completed("ok", "", None),
            ]),
            other => Ok(vec![
                notice(
                    "error",
                    "command.unsupported",
                    &i18n::pick(
                        &format!("o Codex não tem o comando /{other}"),
                        &format!("Codex has no /{other} command"),
                    ),
                ),
                turn_completed("ok", "", None),
            ]),
        }
    }

    /// Render /context from tokenUsage using the markdown panel format shared with Claude.
    fn context_report(&self) -> String {
        let used = self.ctx.unwrap_or(0);
        let total = self.window.unwrap_or(0);
        let pct = if total > 0 {
            (used as f64 / total as f64 * 100.0).round() as u64
        } else {
            0
        };
        let free = total.saturating_sub(used);
        let conversation = i18n::pick("Conversa", "Conversation");
        format!(
            "## Context Usage\n**Model:** {}\n**Tokens:** {} / {} ({}%)\n\n### Estimated usage by category\n| Category | Tokens | Percentage |\n|---|---|---|\n| {} | {} | {}% |\n| Free space | {} | {}% |\n",
            if self.model.is_empty() { self.start.model.clone() } else { self.model.clone() },
            kilo(used),
            kilo(total),
            pct,
            conversation,
            kilo(used),
            pct,
            kilo(free),
            100 - pct.min(100),
        )
    }

    fn call(&mut self, method: &str, params: Value, sent: Sent) -> Result<(), String> {
        self.next += 1;
        let id = self.next;
        self.sent.insert(id, sent);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn reply(&mut self, rpc: &Value, result: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "id": rpc, "result": result }))
    }

    fn refuse(&mut self, rpc: &Value, message: &str) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "id": rpc, "error": { "code": -32601, "message": message } }))
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let mut line = msg.to_string();
        line.push('\n');
        self.out.write_all(line.as_bytes()).map_err(i18n::io)?;
        self.out.flush().map_err(i18n::io)
    }

    /* Codex events to UI */

    /// Translate one process line into canonical display events.
    pub fn on_line(&mut self, line: &str) -> Vec<Value> {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return vec![];
        };
        match (msg.get("method").and_then(Value::as_str), msg.get("id")) {
            (Some(method), Some(id)) => self.request(id.clone(), method, &msg["params"]),
            (Some(method), None) => self.notification(method, &msg["params"]),
            (None, Some(id)) => self.response(id.as_u64().unwrap_or(0), &msg),
            (None, None) => vec![],
        }
    }

    fn response(&mut self, id: u64, msg: &Value) -> Vec<Value> {
        let error = msg["error"]["message"].as_str().map(str::to_string);
        match self.sent.remove(&id) {
            Some(Sent::Init) => {
                let _ = self.notify("initialized", json!({}));
                let _ = self.call("account/rateLimits/read", json!({}), Sent::Usage);
                if self.start.plugin_hook_ids.is_empty() {
                    self.open_thread();
                } else {
                    let _ = self.call(
                        "hooks/list",
                        json!({ "cwds": [self.start.cwd.clone()] }),
                        Sent::Hooks,
                    );
                }
                vec![]
            }
            // An unauthenticated Codex account returns an error and leaves its usage section empty.
            Some(Sent::Usage) => match error {
                Some(_) => vec![],
                // Preserve rateLimitsByLimitId from newer responses so model-specific quotas reach
                // usage without collapsing into the legacy bucket.
                None => vec![rate_limits(&msg["result"])],
            },
            Some(Sent::Hooks) => {
                if let Some(cause) = error {
                    return self.fail_plugin_hooks(cause);
                }
                let selected = self
                    .start
                    .plugin_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>();
                let expected = self
                    .start
                    .plugin_hook_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>();
                let mut found = std::collections::HashSet::new();
                let mut invalid = std::collections::HashSet::new();
                let mut managed_disabled = std::collections::HashSet::new();
                let mut trusts = serde_json::Map::new();
                for hook in msg["result"]["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
                {
                    let Some(plugin) = hook["pluginId"].as_str() else {
                        continue;
                    };
                    if !selected.contains(plugin) {
                        continue;
                    }
                    found.insert(plugin);
                    let enabled = hook["enabled"].as_bool() == Some(true);
                    let status = hook["trustStatus"].as_str();
                    if status == Some("managed") {
                        if !enabled {
                            managed_disabled.insert(plugin);
                        }
                        continue;
                    }
                    if status == Some("trusted") && enabled {
                        continue;
                    }
                    let (Some(key), Some(hash)) =
                        (hook["key"].as_str(), hook["currentHash"].as_str())
                    else {
                        invalid.insert(plugin);
                        continue;
                    };
                    trusts.insert(
                        key.to_string(),
                        json!({ "trusted_hash": hash, "enabled": true }),
                    );
                }
                let mut missing = expected.difference(&found).copied().collect::<Vec<_>>();
                missing.sort_unstable();
                if !missing.is_empty() {
                    return self.fail_plugin_hooks(i18n::ta(
                        "err.plugin.codex.hooksMissing",
                        &[("plugins", missing.join(", "))],
                    ));
                }
                if !invalid.is_empty() {
                    let mut plugins = invalid.into_iter().collect::<Vec<_>>();
                    plugins.sort_unstable();
                    return self.fail_plugin_hooks(i18n::ta(
                        "err.plugin.codex.hooksInvalid",
                        &[("plugins", plugins.join(", "))],
                    ));
                }
                if !managed_disabled.is_empty() {
                    let mut plugins = managed_disabled.into_iter().collect::<Vec<_>>();
                    plugins.sort_unstable();
                    return self.fail_plugin_hooks(i18n::ta(
                        "err.plugin.codex.hooksManaged",
                        &[("plugins", plugins.join(", "))],
                    ));
                }
                if trusts.is_empty() {
                    self.open_thread();
                } else {
                    let _ = self.call(
                        "config/batchWrite",
                        json!({
                            "edits": [{
                                "keyPath": "hooks.state",
                                "value": trusts,
                                "mergeStrategy": "upsert",
                            }],
                            "reloadUserConfig": true,
                        }),
                        Sent::HookTrust,
                    );
                }
                vec![]
            }
            Some(Sent::HookTrust) => {
                if let Some(cause) = error {
                    self.fail_plugin_hooks(cause)
                } else {
                    self.open_thread();
                    vec![]
                }
            }
            Some(Sent::Thread { resumed }) => {
                if let Some(cause) = error {
                    // If resume fails, open a new conversation and notify the person.
                    if resumed {
                        self.start.resume = None;
                        self.open_thread();
                        return vec![notice("error", "provider.resume", &i18n::pick(
                            &format!("não deu para retomar a conversa no Codex ({cause}); esta é nova"),
                            &format!("could not resume the Codex conversation ({cause}); this one is new"),
                        ))];
                    }
                    self.failed = Some(cause.clone());
                    self.queue.clear();
                    return vec![notice("error", "provider.thread", &cause)];
                }
                let thread = msg["result"]["thread"]["id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                self.model = msg["result"]["model"].as_str().unwrap_or("").to_string();
                self.thread = Some(thread.clone());
                let mut out = vec![canonical(
                    "session.identity",
                    json!({ "providerSession": thread }),
                )];
                for frame in std::mem::take(&mut self.queue) {
                    if let Ok(more) = self.write(&frame) {
                        out.extend(more);
                    }
                }
                out
            }
            Some(Sent::Turn) => match error {
                Some(cause) => vec![
                    notice("error", "provider.turn", &cause),
                    turn_completed("error", &cause, None),
                ],
                None => {
                    if let Some(turn) = msg["result"]["turn"]["id"].as_str() {
                        self.turn = Some(turn.to_string());
                    }
                    vec![]
                }
            },
            Some(Sent::Compact) => match error {
                Some(cause) => vec![
                    canonical(
                        "context.compaction",
                        json!({ "state": "failed", "detail": cause }),
                    ),
                    turn_completed("error", &cause, None),
                ],
                None => vec![],
            },
            Some(Sent::Interrupt) | None => vec![],
        }
    }

    fn open_thread(&mut self) {
        let mut params = json!({
            "cwd": self.start.cwd,
            "approvalPolicy": if self.start.permission == Some(crate::actions::Permission::Ask) { "untrusted" } else { "never" },
            "sandbox": "danger-full-access",
        });
        if !self.start.instructions.is_empty() {
            params["developerInstructions"] = Value::String(self.start.instructions.clone());
        }
        if !self.start.model.is_empty() {
            params["model"] = Value::String(self.start.model.clone());
        }
        match self.start.resume.clone() {
            Some(thread) => {
                params["threadId"] = Value::String(thread);
                let _ = self.call("thread/resume", params, Sent::Thread { resumed: true });
            }
            None => {
                let _ = self.call("thread/start", params, Sent::Thread { resumed: false });
            }
        }
    }

    /// A selected plugin promises active behavior. Refuse thread startup when its hooks cannot be
    /// enabled.
    fn fail_plugin_hooks(&mut self, cause: String) -> Vec<Value> {
        let message = i18n::ta("err.plugin.codex.hooks", &[("cause", cause)]);
        self.failed = Some(message.clone());
        self.queue.clear();
        vec![notice("error", "plugin.hooks", &message)]
    }

    /// Present server requests as pending UI cards while retaining their JSON-RPC IDs.
    fn request(&mut self, rpc: Value, method: &str, params: &Value) -> Vec<Value> {
        let id = match &rpc {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let item = params["itemId"].as_str().unwrap_or("").to_string();
        let (kind, tool, input) = match method {
            "item/commandExecution/requestApproval" => {
                let command = params["command"].as_str().unwrap_or("").to_string();
                (
                    AskKind::Command,
                    "Bash",
                    json!({ "command": pretty(&command, &params["commandActions"]) }),
                )
            }
            "item/fileChange/requestApproval" => {
                let paths = self.patches.get(&item).cloned().unwrap_or_default();
                (
                    AskKind::Patch,
                    "Edit",
                    json!({ "file_path": paths.join(", ") }),
                )
            }
            "item/tool/requestUserInput" => {
                let mut ids = vec![];
                let questions: Vec<Value> = params["questions"]
                    .as_array()
                    .map(|qs| {
                        qs.iter()
                            .map(|q| {
                                let question = q["question"].as_str().unwrap_or("").to_string();
                                ids.push((question.clone(), q["id"].as_str().unwrap_or("").to_string()));
                                let options: Vec<Value> = q["options"]
                                    .as_array()
                                    .map(|os| {
                                        os.iter()
                                            .map(|o| json!({ "label": o["label"], "description": o["description"] }))
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                json!({ "question": question, "header": q["header"], "multiSelect": false, "options": options })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    AskKind::Input(ids),
                    "AskUserQuestion",
                    json!({ "questions": questions }),
                )
            }
            _ => {
                let _ = self.refuse(&rpc, "unsupported by prometeu");
                return vec![];
            }
        };
        let request_kind = if matches!(&kind, AskKind::Input(_)) {
            "question"
        } else {
            "approval"
        };
        self.asks.insert(id.clone(), Ask { rpc, kind });
        vec![canonical(
            "request.opened",
            json!({
                "requestId": id,
                "kind": request_kind,
                "toolId": if item.is_empty() { Value::Null } else { json!(item) },
                "tool": tool,
                "input": input,
            }),
        )]
    }

    fn notification(&mut self, method: &str, p: &Value) -> Vec<Value> {
        // Known child threads update background activity only, never the primary turn or content.
        if let (Some(mine), Some(thread)) = (self.thread.as_deref(), p["threadId"].as_str()) {
            if thread != mine {
                if self.subagents.contains_key(thread) {
                    let active = match method {
                        "turn/started" => Some(true),
                        "turn/completed" | "thread/closed" => Some(false),
                        "thread/status/changed" => match p["status"]["type"].as_str() {
                            Some("active") => Some(true),
                            Some("idle" | "notLoaded" | "systemError") => Some(false),
                            _ => None,
                        },
                        _ => None,
                    };
                    if active.is_some_and(|active| self.update_subagent(thread, active, "", None)) {
                        return vec![self.background()];
                    }
                }
                return vec![];
            }
        }
        match method {
            "turn/started" => {
                self.turn = p["turn"]["id"].as_str().map(str::to_string);
                self.block = 0;
                self.message_open = false;
                self.open = None;
                vec![]
            }
            "item/started" => self.started(&p["item"]),
            "item/completed" => self.completed(&p["item"]),
            "item/agentMessage/delta" | "item/plan/delta" => {
                self.delta(p["itemId"].as_str(), p["delta"].as_str())
            }
            "item/reasoning/summaryTextDelta" => {
                self.delta(p["itemId"].as_str(), p["delta"].as_str())
            }
            "item/reasoning/summaryPartAdded" => match p["summaryIndex"].as_u64() {
                Some(n) if n > 0 => self.delta(p["itemId"].as_str(), Some("\n\n")),
                _ => vec![],
            },
            // Usage belongs to the account, not the conversation. Route it through chat::react to
            // the usage store.
            "account/rateLimits/updated" => vec![rate_limits(&p["rateLimits"])],
            "thread/tokenUsage/updated" => {
                let usage = &p["tokenUsage"];
                self.window = usage["modelContextWindow"].as_u64().or(self.window);
                match usage["last"]["totalTokens"].as_u64() {
                    Some(n) if n > 0 => {
                        self.ctx = Some(n);
                        vec![canonical(
                            "context.updated",
                            json!({ "used": n, "window": self.window }),
                        )]
                    }
                    _ => vec![],
                }
            }
            "turn/completed" => {
                let mut out = self.seal(None);
                self.turn = None;
                self.block = 0;
                self.message_open = false;
                let turn = &p["turn"];
                let ms = turn["durationMs"].as_u64();
                out.push(match turn["status"].as_str() {
                    Some("failed") => {
                        let cause = turn["error"]["message"].as_str().unwrap_or("").to_string();
                        turn_completed("error", &cause, ms)
                    }
                    Some("interrupted") => turn_completed("interrupted", "", ms),
                    _ => turn_completed("ok", "", ms),
                });
                out
            }
            "error" => {
                let cause = p["error"]["message"].as_str().unwrap_or("").to_string();
                let retry = p["willRetry"].as_bool() == Some(true);
                let text = match retry {
                    true => i18n::pick(
                        &format!("{cause} (tentando de novo)"),
                        &format!("{cause} (retrying)"),
                    ),
                    false => cause,
                };
                vec![notice("error", "provider.error", &text)]
            }
            "warning" => p["message"]
                .as_str()
                .map(|message| notice("warning", "provider.warning", message))
                .into_iter()
                .collect(),
            _ => vec![],
        }
    }

    fn started(&mut self, item: &Value) -> Vec<Value> {
        let id = item["id"].as_str().unwrap_or("").to_string();
        match item["type"].as_str() {
            Some("agentMessage" | "plan") => self.open_text(&id, false),
            Some("reasoning") => self.open_text(&id, true),
            Some("commandExecution") => {
                let command = pretty(
                    item["command"].as_str().unwrap_or(""),
                    &item["commandActions"],
                );
                self.tool_use(&id, "Bash", json!({ "command": command }))
            }
            Some("fileChange") => {
                let paths: Vec<String> = item["changes"]
                    .as_array()
                    .map(|cs| {
                        cs.iter()
                            .filter_map(|c| c["path"].as_str())
                            .map(|p| self.relative(p))
                            .collect()
                    })
                    .unwrap_or_default();
                self.patches.insert(id.clone(), paths.clone());
                self.tool_use(&id, "Edit", json!({ "file_path": paths.join(", ") }))
            }
            Some("mcpToolCall") => {
                let name = format!(
                    "mcp__{}__{}",
                    item["server"].as_str().unwrap_or(""),
                    item["tool"].as_str().unwrap_or("")
                );
                self.tool_use(&id, &name, item["arguments"].clone())
            }
            Some("dynamicToolCall") => {
                let name = item["tool"].as_str().unwrap_or("tool").to_string();
                self.tool_use(&id, &name, item["arguments"].clone())
            }
            Some("webSearch") => self.tool_use(&id, "WebSearch", json!({ "query": item["query"] })),
            Some("collabAgentToolCall") => self.tool_use(
                &id,
                "Agent",
                json!({ "description": item["tool"], "prompt": item["prompt"] }),
            ),
            Some("imageView") => self.tool_use(&id, "Read", json!({ "file_path": item["path"] })),
            Some("contextCompaction") => {
                self.compact_pre = self.ctx;
                vec![canonical(
                    "context.compaction",
                    json!({ "state": "started", "detail": "" }),
                )]
            }
            _ => vec![],
        }
    }

    fn completed(&mut self, item: &Value) -> Vec<Value> {
        let id = item["id"].as_str().unwrap_or("").to_string();
        match item["type"].as_str() {
            Some("agentMessage" | "plan") => {
                let text = item["text"].as_str().unwrap_or("").to_string();
                self.close_text(&id, text, false)
            }
            Some("reasoning") => {
                let parts = |key: &str| -> Vec<String> {
                    item[key]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let mut summary = parts("summary");
                if summary.is_empty() {
                    summary = parts("content");
                }
                self.close_text(&id, summary.join("\n\n"), true)
            }
            Some("commandExecution") => {
                let ok = item["status"].as_str() == Some("completed");
                let mut text = item["aggregatedOutput"].as_str().unwrap_or("").to_string();
                if let (false, Some(code)) = (ok, item["exitCode"].as_i64()) {
                    text = format!("{text}\n(exit {code})").trim().to_string();
                }
                vec![tool_completed(&id, &text, !ok)]
            }
            Some("fileChange") => {
                self.patches.remove(&id);
                let ok = item["status"].as_str() == Some("completed");
                let diff: Vec<String> = item["changes"]
                    .as_array()
                    .map(|cs| cs.iter().map(|c| patch(c, &self.start.cwd)).collect())
                    .unwrap_or_default();
                vec![tool_completed(&id, &diff.join("\n"), !ok)]
            }
            Some("mcpToolCall") => {
                let failed = item["status"].as_str() == Some("failed") || !item["error"].is_null();
                let text = match item["error"]["message"].as_str() {
                    Some(m) => m.to_string(),
                    None => texts(&item["result"]["content"]),
                };
                vec![tool_completed(&id, &text, failed)]
            }
            Some("dynamicToolCall") => {
                let failed = item["success"].as_bool() == Some(false);
                vec![tool_completed(&id, &texts(&item["contentItems"]), failed)]
            }
            Some("collabAgentToolCall") => {
                let mut changed = false;
                for (thread, state) in item["agentsStates"].as_object().into_iter().flatten() {
                    let active = match state["status"].as_str() {
                        Some("pendingInit" | "running") => true,
                        Some("interrupted" | "completed" | "errored" | "shutdown" | "notFound") => {
                            false
                        }
                        _ => continue,
                    };
                    changed |= self.update_subagent(
                        thread,
                        active,
                        item["prompt"].as_str().unwrap_or(""),
                        Some(&id),
                    );
                }
                let mut out = vec![tool_completed(&id, "", false)];
                if changed {
                    out.push(self.background());
                }
                out
            }
            Some("subAgentActivity") => {
                // This item describes an activity; its item/started and item/completed envelopes
                // are not themselves child lifecycle transitions. Consume only the final item.
                let active = match item["kind"].as_str() {
                    Some("started") => true,
                    Some("completed" | "interrupted") => false,
                    _ => return vec![],
                };
                if self.update_subagent(
                    item["agentThreadId"].as_str().unwrap_or(""),
                    active,
                    item["agentPath"].as_str().unwrap_or(""),
                    None,
                ) {
                    vec![self.background()]
                } else {
                    vec![]
                }
            }
            Some("webSearch" | "imageView") => {
                vec![tool_completed(&id, "", false)]
            }
            Some("contextCompaction") => {
                let post = self.ctx;
                vec![
                    canonical(
                        "context.compaction",
                        json!({ "state": "stopped", "detail": "" }),
                    ),
                    canonical(
                        "context.compacted",
                        json!({ "before": self.compact_pre, "after": post }),
                    ),
                ]
            }
            _ => vec![],
        }
    }

    fn update_subagent(
        &mut self,
        thread: &str,
        active: bool,
        description: &str,
        tool_id: Option<&str>,
    ) -> bool {
        if thread.is_empty() || self.thread.as_deref() == Some(thread) {
            return false;
        }
        let (_, previous) = self.subagents.entry(thread.to_string()).or_insert_with(|| {
            (
                json!({ "id": thread, "description": if description.is_empty() { thread } else { description }, "toolId": tool_id }),
                false,
            )
        });
        let changed = *previous != active;
        *previous = active;
        changed
    }

    fn background(&self) -> Value {
        let tasks: Vec<&Value> = self
            .subagents
            .values()
            .filter_map(|(task, active)| active.then_some(task))
            .collect();
        canonical("background.changed", json!({ "tasks": tasks }))
    }

    /// Close an existing streaming block before opening another so block numbering remains stable
    /// even if event ordering changes.
    fn open_text(&mut self, id: &str, thinking: bool) -> Vec<Value> {
        let mut out = self.seal(None);
        if !self.message_open {
            self.message_open = true;
            out.push(canonical(
                "assistant.started",
                json!({ "messageId": self.msg() }),
            ));
        }
        let index = self.block;
        self.block += 1;
        out.push(canonical(
            "assistant.block.started",
            json!({
                "messageId": self.msg(),
                "index": index,
                "block": block_of("", thinking),
            }),
        ));
        self.open = Some(Open {
            item: id.to_string(),
            index,
            thinking,
            text: String::new(),
        });
        out
    }

    fn delta(&mut self, id: Option<&str>, text: Option<&str>) -> Vec<Value> {
        let (Some(id), Some(text)) = (id, text) else {
            return vec![];
        };
        let Some(open) = self.open.as_mut().filter(|o| o.item == id) else {
            return vec![];
        };
        open.text.push_str(text);
        let (index, thinking) = (open.index, open.thinking);
        vec![canonical(
            "assistant.delta",
            json!({
                "messageId": self.msg(),
                "index": index,
                "kind": if thinking { "thinking" } else { "text" },
                "delta": text,
            }),
        )]
    }

    /// Finish the item's block with final text, or emit the full block when no streaming draft
    /// arrived.
    fn close_text(&mut self, id: &str, text: String, thinking: bool) -> Vec<Value> {
        if self.open.as_ref().is_some_and(|o| o.item == id) {
            return self.seal(Some(text));
        }
        let index = self.block;
        self.block += 1;
        vec![self.assistant(index, block_of(&text, thinking))]
    }

    /// Emit the authoritative completed block, using final text when supplied or the accumulated
    /// draft otherwise.
    fn seal(&mut self, text: Option<String>) -> Vec<Value> {
        let Some(open) = self.open.take() else {
            return vec![];
        };
        let text = text.unwrap_or(open.text);
        vec![self.assistant(open.index, block_of(&text, open.thinking))]
    }

    /// Tool starts already contain the complete command, so their cards can render immediately.
    fn tool_use(&mut self, id: &str, name: &str, input: Value) -> Vec<Value> {
        let mut out = self.seal(None);
        self.message_open = true;
        let index = self.block;
        self.block += 1;
        out.push(self.assistant(
            index,
            json!({ "kind": "tool", "id": id, "name": name, "input": input }),
        ));
        out
    }

    fn assistant(&self, index: usize, block: Value) -> Value {
        canonical(
            "assistant.block",
            json!({ "messageId": self.msg(), "index": index, "block": block }),
        )
    }

    /// All blocks in a turn share one displayed message.
    fn msg(&self) -> String {
        self.turn.clone().unwrap_or_default()
    }

    /// Display paths relative to the worktree when possible.
    fn relative(&self, path: &str) -> String {
        relative(path, &self.start.cwd)
    }
}

fn relative(path: &str, cwd: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_string()
}

/* Canonical output helpers */

/// Route account usage through chat::react without persisting it as conversation text; see
/// chat::keep.
fn rate_limits(limits: &Value) -> Value {
    canonical(
        "usage.updated",
        json!({ "provider": "codex", "usage": limits }),
    )
}

/// Return localized supported slash commands for commands.list and composer suggestions.
const SLASH: [(&str, &str, &str); 2] = [
    (
        "compact",
        "Resume a conversa até aqui para liberar contexto",
        "Free up context by summarizing the conversation so far",
    ),
    (
        "context",
        "Quanto da janela de contexto está em uso",
        "How much of the context window is in use",
    ),
];

fn canonical(kind: &str, fields: Value) -> Value {
    conversation::event(kind, conversation::now(), fields)
}

fn notice(level: &str, code: &str, detail: &str) -> Value {
    canonical(
        "system.notice",
        json!({ "level": level, "code": code, "detail": detail }),
    )
}

fn turn_completed(outcome: &str, message: &str, duration_ms: Option<u64>) -> Value {
    canonical(
        "turn.completed",
        json!({
            "outcome": outcome,
            "message": message,
            "durationMs": duration_ms,
            "costUsd": Value::Null,
        }),
    )
}

fn tool_completed(id: &str, output: &str, error: bool) -> Value {
    canonical(
        "tool.completed",
        json!({ "toolId": id, "output": output, "error": error, "background": false }),
    )
}

fn block_of(text: &str, thinking: bool) -> Value {
    match thinking {
        true => json!({ "kind": "thinking", "text": text }),
        false => json!({ "kind": "text", "text": text }),
    }
}

/// Suppress timestamped tracing lines that duplicate tool errors already sent over JSON-RPC.
/// Preserve other stderr lines because they may explain failures before the protocol starts.
fn process_stderr(line: &str) -> Option<String> {
    let plain = strip_ansi(line);
    let mut fields = plain.split_whitespace();
    let timestamp = fields.next().unwrap_or("");
    let level = fields.next().unwrap_or("");
    let tracing = timestamp.contains('T')
        && timestamp.ends_with('Z')
        && matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR");
    (!tracing).then_some(plain)
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Prefer commandActions' inner command over Codex's /bin/zsh -lc wrapper.
fn pretty(command: &str, actions: &Value) -> String {
    actions
        .as_array()
        .and_then(|a| a.first())
        .and_then(|a| a["command"].as_str())
        .filter(|c| !c.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| command.to_string())
}

/// Add file headers and line markers to raw change hunks or added/deleted file contents so the UI
/// can render a complete diff.
fn patch(change: &Value, cwd: &str) -> String {
    let path = relative(change["path"].as_str().unwrap_or(""), cwd);
    let diff = change["diff"].as_str().unwrap_or("").trim_end_matches('\n');
    if diff.starts_with("diff --git") || diff.starts_with("--- ") {
        return diff.to_string();
    }
    let kind = change["kind"]["type"].as_str().unwrap_or("update");
    let signed = |sign: char| -> String {
        let n = diff.lines().count();
        let body: Vec<String> = diff.lines().map(|l| format!("{sign}{l}")).collect();
        let range = match sign {
            '+' => format!("@@ -0,0 +1,{n} @@"),
            _ => format!("@@ -1,{n} +0,0 @@"),
        };
        format!("{range}\n{}", body.join("\n"))
    };
    let (from, to, hunk) = match kind {
        "add" if !diff.starts_with("@@") => {
            ("/dev/null".to_string(), format!("b/{path}"), signed('+'))
        }
        "add" => (
            "/dev/null".to_string(),
            format!("b/{path}"),
            diff.to_string(),
        ),
        "delete" if !diff.starts_with("@@") => {
            (format!("a/{path}"), "/dev/null".to_string(), signed('-'))
        }
        "delete" => (
            format!("a/{path}"),
            "/dev/null".to_string(),
            diff.to_string(),
        ),
        _ => (format!("a/{path}"), format!("b/{path}"), diff.to_string()),
    };
    format!("diff --git a/{path} b/{path}\n--- {from}\n+++ {to}\n{hunk}")
}

/// Extract text from content blocks returned by MCP and dynamic tools.
fn texts(content: &Value) -> String {
    content
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Format token counts like the frontend's kilo helper: 24k, 3.1k, and 1.2m.
fn kilo(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1000..=9999 => format!("{:.1}k", n as f64 / 1000.0),
        10000..=999_999 => format!("{}k", n / 1000),
        _ => format!("{:.1}m", n as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Capture Link writes through a fake stdin for assertions.
    #[derive(Clone, Default)]
    struct Out(Arc<Mutex<Vec<Value>>>);

    impl Write for Out {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let text = String::from_utf8_lossy(buf);
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                self.0
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(line).unwrap());
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Out {
        fn take(&self) -> Vec<Value> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    fn link(resume: Option<&str>) -> (Link, Out) {
        let out = Out::default();
        let start = Start {
            cwd: "/wt".into(),
            resume: resume.map(str::to_string),
            model: "gpt-5.4".into(),
            effort: "high".into(),
            plugin_ids: vec![],
            plugin_hook_ids: vec![],
            permission: None,
            instructions: String::new(),
        };
        (Link::new(Box::new(out.clone()), start), out)
    }

    #[test]
    fn task_permissions_and_instructions_reach_codex_on_resume() {
        let (mut link, out) = link(Some("previous"));
        link.start.permission = Some(crate::actions::Permission::Ask);
        link.start.instructions = "Review independently".into();
        out.take();
        link.open_thread();
        let sent = out.take();
        let message = sent
            .iter()
            .find(|v| v["method"] == "thread/resume")
            .unwrap();
        assert_eq!(message["params"]["approvalPolicy"], "untrusted");
        assert_eq!(
            message["params"]["developerInstructions"],
            "Review independently"
        );
        assert_eq!(message["params"]["threadId"], "previous");
    }

    /// Complete initialize and thread/start before sending other conversation input.
    fn opened(link: &mut Link, out: &Out) -> Vec<Value> {
        let before = out.take();
        assert!(
            before.iter().all(|m| m["method"] == "initialize"),
            "{before:?}"
        );
        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        assert_eq!(sent[0]["method"], "initialized");
        let start = call_id(&sent, "thread/start");
        let thread = &sent[start.1];
        assert_eq!(thread["params"]["approvalPolicy"], "never");
        assert_eq!(thread["params"]["model"], "gpt-5.4");
        link.on_line(&format!(
            r#"{{"id":{},"result":{{"thread":{{"id":"t-1"}},"model":"gpt-5.4"}}}}"#,
            start.0
        ))
    }

    /// Find JSON-RPC requests by method rather than line position so adding startup requests does
    /// not invalidate unrelated tests.
    fn call_id(sent: &[Value], method: &str) -> (u64, usize) {
        let at = sent
            .iter()
            .position(|m| m["method"] == method)
            .unwrap_or_else(|| panic!("nenhum {method} em {sent:?}"));
        (sent[at]["id"].as_u64().unwrap(), at)
    }

    fn user(text: &str) -> Value {
        json!({ "v": 1, "type": "message.send", "text": text })
    }

    #[test]
    fn a_thread_abre_e_a_fala_que_esperava_vai() {
        let (mut link, out) = link(None);
        assert_eq!(out.take()[0]["method"], "initialize");
        assert!(link.write(&user("oi")).unwrap().is_empty());
        let frames = opened(&mut link, &out);
        assert_eq!(frames[0]["v"], 1);
        assert_eq!(frames[0]["type"], "session.identity");
        assert_eq!(frames[0]["providerSession"], "t-1");
        let sent = out.take();
        assert_eq!(sent[0]["method"], "turn/start");
        assert_eq!(sent[0]["params"]["threadId"], "t-1");
        assert_eq!(sent[0]["params"]["input"][0]["text"], "oi");
        assert_eq!(sent[0]["params"]["effort"], "high");
    }

    /// Explicit plugin selection approves current hashes for that plugin's hooks only, never
    /// unrelated user, project, or plugin hooks.
    #[test]
    fn plugins_escolhidos_aprovam_so_os_proprios_hooks_antes_da_thread() {
        let (mut link, out) = link(None);
        link.start.plugin_ids = vec!["ponytail@prometeu-dev".into()];
        link.start.plugin_hook_ids = vec!["ponytail@prometeu-dev".into()];
        out.take();

        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let hooks = call_id(&sent, "hooks/list");
        assert!(sent
            .iter()
            .all(|message| message["method"] != "thread/start"));
        assert_eq!(sent[hooks.1]["params"]["cwds"][0], "/wt");

        link.on_line(&format!(
            r#"{{"id":{},"result":{{"data":[{{"cwd":"/wt","hooks":[
              {{"pluginId":"ponytail@prometeu-dev","key":"plugin:ponytail:0","currentHash":"sha256:novo","trustStatus":"trusted","enabled":false}},
              {{"pluginId":"ponytail@prometeu-dev","key":"plugin:ponytail:1","currentHash":"sha256:novo-1","trustStatus":"untrusted","enabled":true}},
              {{"pluginId":"ponytail@prometeu-dev","key":"plugin:ponytail:2","currentHash":"sha256:pronto","trustStatus":"trusted","enabled":true}},
              {{"pluginId":"outro@prometeu-dev","key":"plugin:outro:0","currentHash":"sha256:outro","trustStatus":"untrusted","enabled":false}},
              {{"pluginId":null,"key":"/tmp/hooks.json:0","currentHash":"sha256:user","trustStatus":"untrusted","enabled":false}}
            ]}}]}}}}"#,
            hooks.0
        ));
        let sent = out.take();
        let trust = call_id(&sent, "config/batchWrite");
        let value = &sent[trust.1]["params"]["edits"][0]["value"];
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert_eq!(value["plugin:ponytail:0"]["trusted_hash"], "sha256:novo");
        assert_eq!(value["plugin:ponytail:0"]["enabled"], true);
        assert_eq!(value["plugin:ponytail:1"]["trusted_hash"], "sha256:novo-1");
        assert_eq!(value["plugin:ponytail:1"]["enabled"], true);
        assert!(value.get("plugin:ponytail:2").is_none());
        assert_eq!(sent[trust.1]["params"]["reloadUserConfig"], true);

        link.on_line(&format!(r#"{{"id":{},"result":{{}}}}"#, trust.0));
        assert_eq!(out.take()[0]["method"], "thread/start");
    }

    #[test]
    fn plugin_com_hook_declarado_nao_abre_sem_ser_descoberto() {
        let (mut link, out) = link(None);
        link.start.plugin_ids = vec!["caveman@prometeu-dev".into()];
        link.start.plugin_hook_ids = vec!["caveman@prometeu-dev".into()];
        out.take();
        assert!(link.write(&user("fala como caveman")).unwrap().is_empty());

        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let hooks = call_id(&sent, "hooks/list");
        let frames = link.on_line(&format!(
            r#"{{"id":{},"result":{{"data":[{{"cwd":"/wt","hooks":[
              {{"pluginId":"outro@prometeu-dev","key":"plugin:outro:0","currentHash":"sha256:outro","trustStatus":"untrusted","enabled":false}}
            ]}}]}}}}"#,
            hooks.0
        ));

        assert_eq!(frames[0]["type"], "system.notice");
        assert_eq!(frames[0]["level"], "error");
        assert_eq!(frames[0]["code"], "plugin.hooks");
        assert!(frames[0]["detail"]
            .as_str()
            .unwrap()
            .contains("caveman@prometeu-dev"));
        assert!(out
            .take()
            .iter()
            .all(|message| message["method"] != "thread/start"));
        assert!(link.write(&user("oi")).is_err());
    }

    #[test]
    fn falha_ao_ativar_hook_impede_a_thread() {
        let (mut link, out) = link(None);
        link.start.plugin_ids = vec!["caveman@prometeu-dev".into()];
        link.start.plugin_hook_ids = vec!["caveman@prometeu-dev".into()];
        out.take();

        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let hooks = call_id(&sent, "hooks/list");
        link.on_line(&format!(
            r#"{{"id":{},"result":{{"data":[{{"cwd":"/wt","hooks":[
              {{"pluginId":"caveman@prometeu-dev","key":"plugin:caveman:0","currentHash":"sha256:caveman","trustStatus":"untrusted","enabled":false}}
            ]}}]}}}}"#,
            hooks.0
        ));
        let sent = out.take();
        let trust = call_id(&sent, "config/batchWrite");
        let frames = link.on_line(&format!(
            r#"{{"id":{},"error":{{"code":-32603,"message":"config read-only"}}}}"#,
            trust.0
        ));

        assert_eq!(frames[0]["level"], "error");
        assert!(frames[0]["detail"]
            .as_str()
            .unwrap()
            .contains("config read-only"));
        assert!(out
            .take()
            .iter()
            .all(|message| message["method"] != "thread/start"));
    }

    #[test]
    fn initialize_responde_os_comandos_sem_ir_ao_processo() {
        let (mut link, out) = link(None);
        out.take();
        let req = json!({ "v": 1, "type": "commands.list" });
        let frames = link.write(&req).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["v"], 1);
        assert_eq!(frames[0]["type"], "commands.updated");
        let names: Vec<&str> = frames[0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["compact", "context"]);
        assert!(!frames[0]["commands"][0]["description"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(frames[0]["commands"][0]["hint"], "");
        assert!(out.take().is_empty());
    }

    #[test]
    fn leitura_de_cota_entrega_o_snapshot_multibucket_inteiro() {
        let (mut link, out) = link(None);
        out.take();
        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let (id, _) = call_id(&sent, "account/rateLimits/read");
        let frames = link.on_line(&format!(
            r#"{{"id":{id},"result":{{"rateLimits":{{"limitId":"codex"}},"rateLimitsByLimitId":{{"codex":{{"limitId":"codex"}},"spark":{{"limitId":"spark","limitName":"Spark"}}}}}}}}"#
        ));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["type"], "usage.updated");
        assert!(frames[0]["usage"]["rateLimitsByLimitId"]["spark"].is_object());
    }

    #[test]
    fn retomar_passa_o_id_e_cai_para_nova_se_falhar() {
        let (mut link, out) = link(Some("velha"));
        out.take();
        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let (id, at) = call_id(&sent, "thread/resume");
        assert_eq!(sent[at]["params"]["threadId"], "velha");
        let frames = link.on_line(&format!(
            r#"{{"id":{id},"error":{{"code":1,"message":"no such thread"}}}}"#
        ));
        assert_eq!(frames[0]["type"], "system.notice");
        assert_eq!(frames[0]["code"], "provider.resume");
        assert_eq!(out.take()[0]["method"], "thread/start");
    }

    #[test]
    fn um_turno_vira_rascunho_bloco_autoritativo_e_fim() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"t-1","turn":{"id":"turn-1"}}}"#,
        );
        let f = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"m1","text":""}}}"#);
        assert_eq!(f[0]["type"], "assistant.started");
        assert_eq!(f[0]["messageId"], "turn-1");
        assert_eq!(f[1]["type"], "assistant.block.started");
        assert_eq!(f[1]["index"], 0);
        let f = link.on_line(
            r#"{"method":"item/agentMessage/delta","params":{"itemId":"m1","delta":"Ol"}}"#,
        );
        assert_eq!(f[0]["type"], "assistant.delta");
        assert_eq!(f[0]["kind"], "text");
        assert_eq!(f[0]["delta"], "Ol");
        let f = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m1","text":"Olá"}}}"#);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["type"], "assistant.block");
        assert_eq!(f[0]["messageId"], "turn-1");
        assert_eq!(f[0]["block"]["kind"], "text");
        assert_eq!(f[0]["block"]["text"], "Olá");
        assert!(f[0]["at"].is_number());
        let f = link.on_line(r#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"completed","durationMs":900}}}"#);
        assert_eq!(f[0]["type"], "turn.completed");
        assert_eq!(f[0]["outcome"], "ok");
        assert_eq!(f[0]["durationMs"], 900);
    }

    #[test]
    fn turno_de_subagente_nao_encerra_a_conversa() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"t-1","turn":{"id":"turn-1"}}}"#,
        );
        // Subagents use different threads in the same process and must not alter the displayed
        // conversation.
        assert!(link
            .on_line(
                r#"{"method":"turn/started","params":{"threadId":"sub-1","turn":{"id":"turn-s"}}}"#
            )
            .is_empty());
        assert!(link.on_line(r#"{"method":"item/completed","params":{"threadId":"sub-1","turnId":"turn-s","item":{"type":"agentMessage","id":"s1","text":"achei"}}}"#).is_empty());
        assert!(link.on_line(r#"{"method":"turn/completed","params":{"threadId":"sub-1","turn":{"id":"turn-s","status":"completed"}}}"#).is_empty());
        // The primary conversation remains in its own active turn.
        let f = link.on_line(r#"{"method":"item/completed","params":{"threadId":"t-1","turnId":"turn-1","item":{"type":"agentMessage","id":"m1","text":"pronto"}}}"#);
        assert_eq!(f[0]["type"], "assistant.block");
        assert_eq!(f[0]["messageId"], "turn-1");
        let f = link.on_line(r#"{"method":"turn/completed","params":{"threadId":"t-1","turn":{"id":"turn-1","status":"completed"}}}"#);
        assert_eq!(f.last().unwrap()["type"], "turn.completed");
    }

    #[test]
    fn chamada_spawn_concluida_mantem_background_ate_o_filho_terminar() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"t-1","turn":{"id":"turn-1"}}}"#,
        );
        let spawn = json!({
            "method": "item/completed", "params": { "threadId": "t-1", "item": {
                "type": "collabAgentToolCall", "id": "spawn-1", "tool": "spawnAgent",
                "status": "completed", "senderThreadId": "t-1", "receiverThreadIds": ["sub-1"],
                "prompt": "mapear", "agentsStates": { "sub-1": { "status": "running", "message": null } },
            } },
        }).to_string();
        let events = link.on_line(&spawn);
        assert_eq!(events[0]["type"], "tool.completed");
        assert_eq!(events[1]["type"], "background.changed");
        assert_eq!(
            events[1]["tasks"],
            json!([{ "id": "sub-1", "description": "mapear", "toolId": "spawn-1" }])
        );
        assert_eq!(link.on_line(&spawn).len(), 1);

        let events = link.on_line(r#"{"method":"turn/completed","params":{"threadId":"t-1","turn":{"id":"turn-1","status":"completed"}}}"#);
        assert_eq!(events.last().unwrap()["type"], "turn.completed");
        assert_eq!(link.background()["tasks"].as_array().unwrap().len(), 1);
        assert!(link.on_line(r#"{"method":"turn/completed","params":{"threadId":"unknown","turn":{"id":"other","status":"completed"}}}"#).is_empty());

        link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"t-1","turn":{"id":"turn-2"}}}"#,
        );
        let events = link.on_line(r#"{"method":"turn/completed","params":{"threadId":"sub-1","turn":{"id":"child-turn","status":"completed"}}}"#);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "background.changed");
        assert_eq!(events[0]["tasks"], json!([]));
        assert_eq!(link.turn.as_deref(), Some("turn-2"));
        assert!(link.on_line(r#"{"method":"turn/completed","params":{"threadId":"sub-1","turn":{"id":"child-turn","status":"completed"}}}"#).is_empty());
    }

    #[test]
    fn atividade_de_subagente_nao_confunde_envelopes_com_inicio_e_fim() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let activity = |method: &str, kind: &str| {
            json!({
                "method": method, "params": { "threadId": "t-1", "item": {
                    "type": "subAgentActivity", "id": "activity-1", "kind": kind,
                    "agentThreadId": "sub-1", "agentPath": "/root/review",
                } },
            })
            .to_string()
        };
        assert!(link
            .on_line(&activity("item/started", "started"))
            .is_empty());
        let events = link.on_line(&activity("item/completed", "started"));
        assert_eq!(events[0]["type"], "background.changed");
        assert_eq!(events[0]["tasks"][0]["id"], "sub-1");
        assert!(link
            .on_line(&activity("item/completed", "started"))
            .is_empty());
        assert!(link
            .on_line(&activity("item/completed", "interacted"))
            .is_empty());
        assert!(link
            .on_line(&activity("item/started", "completed"))
            .is_empty());
        let events = link.on_line(&activity("item/completed", "completed"));
        assert_eq!(events[0]["tasks"], json!([]));
        assert!(link
            .on_line(&activity("item/started", "started"))
            .is_empty());

        // A known child can run again without changing the primary turn.
        let events = link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"sub-1","turn":{"id":"child-2"}}}"#,
        );
        assert_eq!(events[0]["type"], "background.changed");
        assert_eq!(events[0]["tasks"][0]["id"], "sub-1");
        assert_eq!(link.turn, None);
        let events = link.on_line(r#"{"method":"thread/status/changed","params":{"threadId":"sub-1","status":{"type":"idle"}}}"#);
        assert_eq!(events[0]["tasks"], json!([]));
        assert!(link.on_line(r#"{"method":"thread/status/changed","params":{"threadId":"unknown","status":{"type":"active","activeFlags":[]}}}"#).is_empty());
    }

    #[test]
    fn estados_parciais_de_subagentes_preservam_outros_filhos() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let collab = |states: Value| {
            json!({
            "method": "item/completed", "params": { "threadId": "t-1", "item": {
                "type": "collabAgentToolCall", "id": "call-1", "tool": "wait", "status": "completed",
                "agentsStates": states,
            } },
        }).to_string()
        };
        link.on_line(&collab(json!({
            "sub-1": { "status": "pendingInit" }, "sub-2": { "status": "running" },
            "t-1": { "status": "running" }, "": { "status": "running" },
        })));
        assert_eq!(link.background()["tasks"].as_array().unwrap().len(), 2);
        assert_eq!(
            link.on_line(&collab(json!({ "sub-1": { "status": "futureStatus" } })))
                .len(),
            1
        );
        assert_eq!(link.on_line(&collab(json!({}))).len(), 1);
        let events = link.on_line(&collab(json!({ "sub-1": { "status": "completed" } })));
        assert_eq!(events[1]["tasks"][0]["id"], "sub-2");
        for terminal in ["interrupted", "errored", "shutdown", "notFound"] {
            link.on_line(&collab(json!({ "sub-1": { "status": "running" } })));
            let events = link.on_line(&collab(json!({ "sub-1": { "status": terminal } })));
            assert_eq!(events[1]["tasks"].as_array().unwrap().len(), 1);
            assert_eq!(events[1]["tasks"][0]["id"], "sub-2");
        }
    }

    #[test]
    fn a_borda_do_codex_entrega_eventos_v1_sem_segunda_traducao() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"t-1","turn":{"id":"turn-1"}}}"#,
        );
        let started = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"ls","commandActions":[]}}}"#);
        assert_eq!(started[0]["v"], 1);
        assert_eq!(started[0]["type"], "assistant.block");
        assert_eq!(started[0]["block"]["kind"], "tool");

        let completed = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","status":"completed","aggregatedOutput":"ok","exitCode":0}}}"#);
        assert_eq!(completed[0]["type"], "tool.completed");
        assert_eq!(completed[0]["toolId"], "c1");

        let completed = link.on_line(
            r#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"completed"}}}"#,
        );
        assert_eq!(completed.last().unwrap()["type"], "turn.completed");
    }

    #[test]
    fn evento_externo_desconhecido_e_ignorado_na_borda() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        assert!(link
            .on_line(r#"{"method":"future/event","params":{"new":true}}"#)
            .is_empty());
    }

    #[test]
    fn comando_vira_bash_e_o_resultado_acha_o_bloco() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}"#);
        let f = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"/bin/zsh -lc \"ls -la\"","commandActions":[{"type":"unknown","command":"ls -la"}],"status":"inProgress"}}}"#);
        assert_eq!(f[0]["type"], "assistant.block");
        let block = &f[0]["block"];
        assert_eq!(block["kind"], "tool");
        assert_eq!(block["name"], "Bash");
        assert_eq!(block["id"], "c1");
        assert_eq!(block["input"]["command"], "ls -la");
        let f = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","status":"completed","aggregatedOutput":"a.txt\n","exitCode":0}}}"#);
        assert_eq!(f[0]["type"], "tool.completed");
        assert_eq!(f[0]["toolId"], "c1");
        assert_eq!(f[0]["output"], "a.txt\n");
        assert_eq!(f[0]["error"], false);
    }

    /// Close streamed text before adding a tool so block indexes match the UI's ordering.
    #[test]
    fn ferramenta_no_meio_do_texto_fecha_o_texto_antes() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}"#);
        link.on_line(
            r#"{"method":"item/started","params":{"item":{"type":"reasoning","id":"r1"}}}"#,
        );
        link.on_line(r#"{"method":"item/reasoning/summaryTextDelta","params":{"itemId":"r1","delta":"pensando"}}"#);
        let f = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"ls","commandActions":[]}}}"#);
        assert_eq!(f[0]["type"], "assistant.block");
        assert_eq!(f[0]["block"]["kind"], "thinking");
        assert_eq!(f[0]["block"]["text"], "pensando");
        assert_eq!(f[1]["block"]["kind"], "tool");
        // The next text uses index 2, after reasoning at 0 and the tool at 1.
        let f = link.on_line(
            r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"m1"}}}"#,
        );
        assert_eq!(f[0]["type"], "assistant.block.started");
        assert_eq!(f[0]["index"], 2);
    }

    #[test]
    fn a_pergunta_vira_card_e_a_resposta_volta_no_id_do_codex() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.on_line(r#"{"id":7,"method":"item/tool/requestUserInput","params":{"itemId":"q1","questions":[{"id":"cor","header":"Cor","question":"Qual cor?","options":[{"label":"azul","description":"frio"}]}]}}"#);
        assert_eq!(f[0]["type"], "request.opened");
        assert_eq!(f[0]["requestId"], "7");
        assert_eq!(f[0]["kind"], "question");
        assert_eq!(f[0]["tool"], "AskUserQuestion");
        assert_eq!(f[0]["input"]["questions"][0]["options"][0]["label"], "azul");
        link.write(&json!({
            "v": 1,
            "type": "request.respond",
            "requestId": "7",
            "response": { "outcome": "answer", "answers": { "Qual cor?": "azul" } },
        }))
        .unwrap();
        let sent = out.take();
        assert_eq!(sent[0]["id"], 7);
        assert_eq!(sent[0]["result"]["answers"]["cor"]["answers"][0], "azul");
    }

    #[test]
    fn aprovacao_de_comando_responde_accept_ou_decline() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.on_line(r#"{"id":"r-9","method":"item/commandExecution/requestApproval","params":{"itemId":"c1","command":"rm -rf x"}}"#);
        assert_eq!(f[0]["type"], "request.opened");
        assert_eq!(f[0]["kind"], "approval");
        assert_eq!(f[0]["tool"], "Bash");
        assert_eq!(f[0]["input"]["command"], "rm -rf x");
        link.write(&json!({
            "v": 1,
            "type": "request.respond",
            "requestId": "r-9",
            "response": { "outcome": "deny", "message": "não" },
        }))
        .unwrap();
        let sent = out.take();
        assert_eq!(sent[0]["id"], "r-9");
        assert_eq!(sent[0]["result"]["decision"], "decline");
    }

    #[test]
    fn compact_e_a_compactacao_contam_o_antes_e_o_depois() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":20000},"modelContextWindow":258400}}}"#);
        let f = link.write(&user("/compact")).unwrap();
        assert_eq!(f[0]["type"], "context.compaction");
        assert_eq!(f[0]["state"], "started");
        assert_eq!(out.take()[0]["method"], "thread/compact/start");
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-c"}}}"#);
        link.on_line(
            r#"{"method":"item/started","params":{"item":{"type":"contextCompaction","id":"k1"}}}"#,
        );
        link.on_line(r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":4000},"modelContextWindow":258400}}}"#);
        let f = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"contextCompaction","id":"k1"}}}"#);
        assert_eq!(f[0]["type"], "context.compaction");
        assert_eq!(f[0]["state"], "stopped");
        assert_eq!(f[1]["type"], "context.compacted");
        assert_eq!(f[1]["before"], 20000);
        assert_eq!(f[1]["after"], 4000);
    }

    #[test]
    fn context_vira_o_relatorio_que_a_tela_desenha() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.on_line(r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":12300},"modelContextWindow":258400}}}"#);
        assert_eq!(f[0]["type"], "context.updated");
        assert_eq!(f[0]["used"], 12300);
        assert_eq!(f[0]["window"], 258400);
        let f = link.write(&user("/context")).unwrap();
        assert_eq!(f[0]["type"], "context.reported");
        let text = f[0]["markdown"].as_str().unwrap();
        assert!(text.starts_with("## Context Usage"));
        assert!(text.contains("**Model:** gpt-5.4"));
        assert!(text.contains("**Tokens:** 12k / 258k (5%)"));
        assert_eq!(f[1]["type"], "turn.completed");
        assert!(out.take().is_empty(), "/context não vai ao processo");
    }

    #[test]
    fn comando_que_o_codex_nao_tem_e_recusado_na_tela() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.write(&user("/cost")).unwrap();
        assert_eq!(f[0]["type"], "system.notice");
        assert_eq!(f[0]["code"], "command.unsupported");
        assert!(f[0]["detail"].as_str().unwrap().contains("/cost"));
        assert_eq!(f[1]["type"], "turn.completed");
    }

    #[test]
    fn caminho_absoluto_nao_vira_comando_de_barra() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let path = r#"/var/folders/q7/TemporaryItems/Captura\ de\ Tela.png"#;
        assert!(link.write(&user(path)).unwrap().is_empty());
        let sent = out.take();
        assert_eq!(sent[0]["method"], "turn/start");
        assert_eq!(sent[0]["params"]["input"][0]["text"], path);
    }

    #[test]
    fn log_do_app_server_nao_duplica_erro_da_ferramenta() {
        let log = "\u{1b}[2m2026-08-28T16:54:16.210466Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m \u{1b}[2mcodex_core::tools::router\u{1b}[0m: error=apply_patch verification failed";
        assert_eq!(process_stderr(log), None);
        assert_eq!(
            process_stderr("codex: not logged in"),
            Some("codex: not logged in".into())
        );
    }

    #[test]
    fn interromper_precisa_do_turno() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let stop = json!({ "v": 1, "type": "turn.interrupt" });
        link.write(&stop).unwrap();
        assert!(out.take().is_empty());
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}"#);
        link.write(&stop).unwrap();
        let sent = out.take();
        assert_eq!(sent[0]["method"], "turn/interrupt");
        assert_eq!(sent[0]["params"]["turnId"], "turn-1");
        let f = link.on_line(r#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"interrupted"}}}"#);
        assert_eq!(f[0]["type"], "turn.completed");
        assert_eq!(f[0]["outcome"], "interrupted");
        assert_eq!(f[0]["message"], "");
    }

    /// Match Codex's absolute paths, raw modification hunks, and unmarked added-file contents.
    #[test]
    fn o_patch_vira_diff_com_cabecalho_e_sinal() {
        let change = json!({ "path": "/wt/src/a.rs", "kind": { "type": "update", "move_path": null }, "diff": "@@ -1 +1 @@\n-a\n+b\n" });
        let text = patch(&change, "/wt");
        assert_eq!(
            text,
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b"
        );
        let add =
            json!({ "path": "/wt/n.txt", "kind": { "type": "add" }, "diff": "novo\nlinha\n" });
        assert_eq!(patch(&add, "/wt"), "diff --git a/n.txt b/n.txt\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,2 @@\n+novo\n+linha");
        let del = json!({ "path": "/outro/x.txt", "kind": { "type": "delete" }, "diff": "fim\n" });
        assert!(patch(&del, "/wt").starts_with("diff --git a//outro/x.txt b//outro/x.txt\n--- a//outro/x.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-fim"));
    }

    #[test]
    fn kilo_como_na_tela() {
        assert_eq!(kilo(368), "368");
        assert_eq!(kilo(3140), "3.1k");
        assert_eq!(kilo(24000), "24k");
        assert_eq!(kilo(1_200_000), "1.2m");
    }
}
