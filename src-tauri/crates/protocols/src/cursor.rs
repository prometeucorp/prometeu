//! Cursor ACP over stdio. Native messages stop here; presentation consumes only V1 events.
use crate::{conversation, i18n};
use prometeu_core::{
    agents::{CatalogError, Model},
    command::QueryLauncher,
    lock::lock,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::Write,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub struct Start {
    pub cwd: String,
    pub resume: Option<String>,
    pub model: String,
    pub plan: bool,
    pub bypass: bool,
    pub instructions: String,
    pub mcp_servers: Vec<Value>,
}

pub fn connect(
    out: Box<dyn Write + Send>,
    start: Start,
) -> prometeu_core::session::provider::AgentProtocol {
    let link = Arc::new(Mutex::new(Link::new(out, start)));
    let reader = link.clone();
    prometeu_core::session::provider::AgentProtocol {
        input: Box::new(Input(link)),
        translate: Box::new(move |line| {
            lock(&reader)
                .on_line(line)
                .iter()
                .map(Value::to_string)
                .collect()
        }),
    }
}
struct Input(Arc<Mutex<Link>>);
impl prometeu_core::conversation::stream::ConversationInput for Input {
    fn send(&mut self, command: &Value, _: &str) -> Result<Vec<Value>, String> {
        lock(&self.0).write(command)
    }
}
impl prometeu_core::session::provider::AgentInput for Input {
    fn is_connected(&self) -> bool {
        lock(&self.0).out.is_some()
    }
    fn close(&mut self) {
        lock(&self.0).out.take();
    }
    fn requires_idle(&self) -> bool {
        true
    }
}

fn event(kind: &str, fields: Value) -> Value {
    conversation::event(kind, conversation::now(), fields)
}
fn initialize() -> Value {
    json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"prometeu","version":env!("CARGO_PKG_VERSION")}})
}

/// Empty ACP sessions are ephemeral until prompted. Discovery performs no inference or file work.
/// CLI `models` aliases differ from ACP model IDs; use the IDs this transport accepts.
pub fn query_catalog(
    launcher: &dyn QueryLauncher<Command>,
    command: &mut Command,
    timeout: Duration,
    cwd: &str,
) -> Result<Vec<Model>, CatalogError> {
    let mut process = crate::catalog::CatalogProcess::spawn(launcher, command, timeout)?;
    for (id, method, params) in [
        (1, "initialize", initialize()),
        (2, "authenticate", json!({"methodId":"cursor_login"})),
    ] {
        process.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let result = process.response(id)?;
        if id == 1 && result["protocolVersion"] != 1 {
            return Err(CatalogError::new("invalid"));
        }
    }
    process.send(
        json!({"jsonrpc":"2.0","id":3,"method":"session/new","params":{"cwd":cwd,"mcpServers":[]}}),
    )?;
    parse_catalog(&process.response(3)?)
}
pub fn parse_catalog(value: &Value) -> Result<Vec<Model>, CatalogError> {
    let items = value["models"]["availableModels"]
        .as_array()
        .ok_or_else(|| CatalogError::new("invalid"))?;
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
        .map(|item| {
            let id = item["modelId"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| CatalogError::new("invalid"))?;
            let label = item["name"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| CatalogError::new("invalid"))?;
            if !seen.insert(id) {
                return Err(CatalogError::new("invalid"));
            }
            Ok(Model {
                id: id.into(),
                label: label.into(),
                efforts: vec![],
                additional: false,
            })
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Sent {
    Init,
    Auth,
    Session,
    Model,
    Mode,
    Turn,
}
struct Ask {
    rpc: Value,
    method: String,
    params: Value,
}
struct Tool {
    index: usize,
    name: String,
    input: Value,
    done: bool,
}
struct Open {
    index: usize,
    kind: &'static str,
    text: String,
}

pub struct Link {
    out: Option<Box<dyn Write + Send>>,
    start: Start,
    mode: String,
    next: u64,
    sent: HashMap<u64, Sent>,
    session: Option<String>,
    ready: bool,
    failed: Option<String>,
    pending: Option<String>,
    busy: bool,
    interrupted: bool,
    asks: HashMap<String, Ask>,
    tools: HashMap<String, Tool>,
    open: Option<Open>,
    index: usize,
    message: String,
    started: Instant,
}
impl Link {
    pub fn new(out: Box<dyn Write + Send>, start: Start) -> Self {
        let mut link = Self {
            out: Some(out),
            mode: if start.plan {
                "plan".into()
            } else {
                "agent".into()
            },
            start,
            next: 0,
            sent: HashMap::new(),
            session: None,
            ready: false,
            failed: None,
            pending: None,
            busy: false,
            interrupted: false,
            asks: HashMap::new(),
            tools: HashMap::new(),
            open: None,
            index: 0,
            message: String::new(),
            started: Instant::now(),
        };
        if let Err(error) = link.call("initialize", initialize(), Sent::Init) {
            link.failed = Some(error);
            link.out.take();
        }
        link
    }
    fn send(&mut self, value: Value) -> Result<(), String> {
        let out = self
            .out
            .as_mut()
            .ok_or_else(|| i18n::t("err.cursor.transport"))?;
        writeln!(out, "{value}")
            .and_then(|_| out.flush())
            .map_err(|_| i18n::t("err.cursor.transport"))
    }
    fn call(&mut self, method: &str, params: Value, sent: Sent) -> Result<(), String> {
        self.next += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params}))?;
        self.sent.insert(self.next, sent);
        Ok(())
    }
    fn reply(&mut self, id: &Value, result: Value) -> Result<(), String> {
        self.send(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
    pub fn write(&mut self, command: &Value) -> Result<Vec<Value>, String> {
        if command["v"] != 1 {
            return Err(i18n::t("err.team.bad"));
        }
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        match command["type"].as_str() {
            Some("commands.list") => Ok(vec![event("commands.updated", json!({"commands":[]}))]),
            Some("message.send") => {
                let text = command["text"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.team.bad"))?;
                if self.busy {
                    return Err("conversation_busy".into());
                }
                self.busy = true;
                self.interrupted = false;
                if !self.ready {
                    self.pending = Some(text.into());
                    return Ok(vec![]);
                }
                if let Err(error) = self.prompt(text.into()) {
                    self.busy = false;
                    return Err(error);
                }
                Ok(vec![])
            }
            Some("request.respond") => self.respond(command),
            Some("permission.mode.set") if command["mode"] == "bypass" => {
                self.start.bypass = true;
                Ok(vec![])
            }
            Some("turn.interrupt") => {
                if !self.busy {
                    return Ok(vec![]);
                }
                self.interrupted = true;
                if self.pending.take().is_some() {
                    return Ok(self.finish("interrupted", ""));
                }
                self.send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":self.session}}))?;
                let mut events = vec![];
                // Resolve blocked client requests before waiting for ACP's cancelled terminal.
                for (id, ask) in std::mem::take(&mut self.asks) {
                    self.reply(&ask.rpc, json!({"outcome":{"outcome":"cancelled"}}))?;
                    events.push(event(
                        "request.closed",
                        json!({"requestId":id,"outcome":"cancelled"}),
                    ));
                }
                Ok(events)
            }
            _ => Err(i18n::t("err.cursor.unsupported")),
        }
    }
    fn prompt(&mut self, text: String) -> Result<(), String> {
        let text = if self.start.instructions.is_empty() {
            text
        } else {
            format!("{}\n\n{text}", self.start.instructions)
        };
        self.call(
            "session/prompt",
            json!({"sessionId":self.session,"prompt":[{"type":"text","text":text}]}),
            Sent::Turn,
        )?;
        self.start.instructions.clear();
        self.started = Instant::now();
        self.index = 0;
        self.tools.clear();
        self.open = None;
        // A process restart must not reuse persisted assistant IDs.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.message = format!(
            "cursor:{}:{nonce}:{}",
            self.session.as_deref().unwrap_or(""),
            self.next
        );
        Ok(())
    }
    pub fn on_line(&mut self, line: &str) -> Vec<Value> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return vec![];
        };
        if value["jsonrpc"] != "2.0" || self.failed.is_some() {
            return vec![];
        }
        let result = if let Some(method) = value["method"].as_str() {
            if value.get("id").is_some() {
                self.request(value["id"].clone(), method, &value["params"])
            } else if method == "session/update"
                && self.ready
                && self.busy
                && value["params"]["sessionId"].as_str() == self.session.as_deref()
            {
                Ok(self.update(&value["params"]["update"]))
            } else {
                Ok(vec![])
            }
        } else if let Some(sent) = value["id"].as_u64().and_then(|id| self.sent.remove(&id)) {
            if value.get("error").is_some() {
                let code = match sent {
                    Sent::Auth => "err.cursor.auth",
                    Sent::Session if self.start.resume.is_some() => "err.cursor.resume",
                    _ => "err.cursor.failed",
                };
                if matches!(sent, Sent::Turn) {
                    return self.finish(
                        if self.interrupted {
                            "interrupted"
                        } else {
                            "error"
                        },
                        &i18n::t(code),
                    );
                }
                Err(i18n::t(code))
            } else if value["result"].is_object() {
                self.response(sent, &value["result"])
            } else {
                Err(i18n::t("err.cursor.transport"))
            }
        } else {
            Ok(vec![])
        };
        result.unwrap_or_else(|error| {
            self.failed = Some(error.clone());
            self.out.take();
            self.pending = None;
            let mut events = self.finish("error", &error);
            events.push(event(
                "system.notice",
                json!({"level":"error","code":"cursor.connection","detail":error}),
            ));
            events
        })
    }
    fn response(&mut self, sent: Sent, value: &Value) -> Result<Vec<Value>, String> {
        match sent {
            Sent::Init => {
                if value["protocolVersion"] != 1
                    || (self.start.resume.is_some()
                        && value["agentCapabilities"]["loadSession"] != true)
                {
                    return Err(i18n::t("err.cursor.unsupported"));
                }
                self.call(
                    "authenticate",
                    json!({"methodId":"cursor_login"}),
                    Sent::Auth,
                )?;
            }
            Sent::Auth => {
                let mut params = json!({"cwd":self.start.cwd,"mcpServers":self.start.mcp_servers});
                let method = if let Some(session) = &self.start.resume {
                    params["sessionId"] = json!(session);
                    "session/load"
                } else {
                    "session/new"
                };
                self.call(method, params, Sent::Session)?;
            }
            Sent::Session => {
                // Native mode can change after plan approval; do not restore the initial mode on resume.
                if self.start.resume.is_some() {
                    if let Some(mode) = value["modes"]["currentModeId"].as_str() {
                        self.mode = mode.into();
                    }
                }
                let session = self
                    .start
                    .resume
                    .as_deref()
                    .or_else(|| value["sessionId"].as_str())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| i18n::t("err.cursor.transport"))?
                    .to_owned();
                self.session = Some(session.clone());
                if self.start.model.is_empty() {
                    self.set_mode()?;
                } else {
                    self.call(
                        "session/set_model",
                        json!({"sessionId":session,"modelId":self.start.model}),
                        Sent::Model,
                    )?;
                }
                return Ok(vec![event(
                    "session.identity",
                    json!({"providerSession":session}),
                )]);
            }
            Sent::Model => self.set_mode()?,
            Sent::Mode => {
                self.ready = true;
                if let Some(text) = self.pending.take() {
                    self.prompt(text)?;
                }
            }
            Sent::Turn => {
                let outcome = if self.interrupted || value["stopReason"] == "cancelled" {
                    "interrupted"
                } else if matches!(
                    value["stopReason"].as_str(),
                    Some("end_turn" | "max_tokens" | "max_turn_requests" | "refusal")
                ) {
                    "ok"
                } else {
                    "error"
                };
                let message = if outcome == "error" {
                    i18n::t("err.cursor.failed")
                } else {
                    String::new()
                };
                return Ok(self.finish(outcome, &message));
            }
        }
        Ok(vec![])
    }
    fn set_mode(&mut self) -> Result<(), String> {
        self.call(
            "session/set_mode",
            json!({"sessionId":self.session,"modeId":self.mode}),
            Sent::Mode,
        )
    }
    fn flush(&mut self) -> Vec<Value> {
        self.open.take().map(|open| event("assistant.block", json!({"messageId":self.message,"index":open.index,"block":{"kind":open.kind,"text":open.text}}))).into_iter().collect()
    }
    fn text(&mut self, kind: &'static str, text: &str) -> Vec<Value> {
        if text.is_empty() {
            return vec![];
        }
        let mut events = vec![];
        if self.open.as_ref().is_some_and(|open| open.kind != kind) {
            events.extend(self.flush());
        }
        if self.open.is_none() {
            if self.index == 0 {
                events.push(event(
                    "assistant.started",
                    json!({"messageId":self.message}),
                ));
            }
            let index = self.index;
            self.index += 1;
            self.open = Some(Open {
                index,
                kind,
                text: String::new(),
            });
            events.push(event(
                "assistant.block.started",
                json!({"messageId":self.message,"index":index,"block":{"kind":kind,"text":""}}),
            ));
        }
        let open = self.open.as_mut().unwrap();
        open.text.push_str(text);
        events.push(event(
            "assistant.delta",
            json!({"messageId":self.message,"index":open.index,"kind":kind,"delta":text}),
        ));
        events
    }
    fn update(&mut self, value: &Value) -> Vec<Value> {
        match value["sessionUpdate"].as_str() {
            Some("agent_message_chunk" | "agent_thought_chunk")
                if value["content"]["type"] == "text" =>
            {
                self.text(
                    if value["sessionUpdate"] == "agent_thought_chunk" {
                        "thinking"
                    } else {
                        "text"
                    },
                    value["content"]["text"].as_str().unwrap_or(""),
                )
            }
            Some("tool_call" | "tool_call_update") => self.tool(value),
            _ => vec![],
        }
    }
    fn tool(&mut self, value: &Value) -> Vec<Value> {
        let Some(id) = value["toolCallId"].as_str().filter(|s| !s.is_empty()) else {
            return vec![];
        };
        if self.tools.get(id).is_some_and(|tool| tool.done) {
            return vec![];
        }
        let mut events = self.flush();
        if !self.tools.contains_key(id) {
            let index = self.index;
            self.index += 1;
            let (name, input) = tool_input(value);
            self.tools.insert(
                id.into(),
                Tool {
                    index,
                    name,
                    input,
                    done: false,
                },
            );
        }
        let tool = self.tools.get_mut(id).unwrap();
        if value["rawInput"].is_object() {
            tool.input = tool_input(value).1;
        }
        events.push(event("assistant.block", json!({"messageId":self.message,"index":tool.index,"block":{"kind":"tool","id":id,"name":tool.name,"input":tool.input}})));
        if matches!(value["status"].as_str(), Some("completed" | "failed")) {
            tool.done = true;
            let mut output = value["content"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            if item["type"] == "content" {
                                item["content"]["text"].as_str()
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            if output.is_empty() {
                let raw = &value["rawOutput"];
                output = if raw.is_object() {
                    [
                        raw["stdout"].as_str().unwrap_or(""),
                        raw["stderr"].as_str().unwrap_or(""),
                    ]
                    .concat()
                } else {
                    raw.as_str().unwrap_or("").into()
                };
                if output.is_empty() && !raw.is_null() {
                    output = raw.to_string();
                }
            }
            events.push(event("tool.completed", json!({"toolId":id,"output":output,"error":value["status"] == "failed","background":false})));
        }
        events
    }
    fn finish(&mut self, outcome: &str, message: &str) -> Vec<Value> {
        if !self.busy {
            return vec![];
        }
        self.busy = false;
        let mut events = self.flush();
        for (id, _) in self.asks.drain() {
            events.push(event(
                "request.closed",
                json!({"requestId":id,"outcome":"cancelled"}),
            ));
        }
        events.push(event("turn.completed", json!({"outcome":outcome,"message":message,"durationMs":self.started.elapsed().as_millis() as u64,"costUsd":null})));
        events
    }
    fn request(&mut self, rpc: Value, method: &str, params: &Value) -> Result<Vec<Value>, String> {
        if !self.ready
            || !self.busy
            || params
                .get("sessionId")
                .is_some_and(|id| id.as_str() != self.session.as_deref())
        {
            self.send(json!({"jsonrpc":"2.0","id":rpc,"error":{"code":-32602,"message":"No active session"}}))?;
            return Ok(vec![]);
        }
        if self.interrupted {
            self.reply(&rpc, json!({"outcome":{"outcome":"cancelled"}}))?;
            return Ok(vec![]);
        }
        // Server request counters restart with the process; transcript identities must not.
        let id = format!("{}:request:{rpc}", self.message);
        let mut events = self.flush();
        let (kind, tool, input) = match method {
            "session/request_permission" => {
                if self.start.bypass {
                    if let Some(option) = permission_option(params, true) {
                        self.reply(
                            &rpc,
                            json!({"outcome":{"outcome":"selected","optionId":option}}),
                        )?;
                        return Ok(events);
                    }
                }
                let (tool, input) = tool_input(&params["toolCall"]);
                ("approval", tool, input)
            }
            "cursor/ask_question" => {
                let Some(questions) = params["questions"].as_array().filter(|questions| {
                    let mut prompts = std::collections::HashSet::new();
                    !questions.is_empty()
                        && questions.iter().all(|q| {
                            q["id"].as_str().is_some_and(|s| !s.is_empty())
                                && q["prompt"]
                                    .as_str()
                                    .is_some_and(|s| !s.is_empty() && prompts.insert(s))
                                && q["options"].as_array().is_some_and(|options| {
                                    options.iter().all(|o| {
                                        o["id"].as_str().is_some_and(|s| !s.is_empty())
                                            && o["label"].as_str().is_some_and(|s| !s.is_empty())
                                    })
                                })
                        })
                }) else {
                    self.reply(
                        &rpc,
                        json!({"outcome":{"outcome":"skipped","reason":"Invalid questions"}}),
                    )?;
                    return Ok(events);
                };
                let questions: Vec<_> = questions.iter().map(|q| json!({"question":q["prompt"],"multiSelect":q["allowMultiple"] == true,"options":q["options"].as_array().map(|options| options.iter().map(|o| json!({"label":o["label"]})).collect::<Vec<_>>()).unwrap_or_default()})).collect();
                (
                    "question",
                    "AskUserQuestion".into(),
                    json!({"questions":questions}),
                )
            }
            "cursor/create_plan" => {
                let Some(plan) = params["plan"]
                    .as_str()
                    .filter(|plan| !plan.trim().is_empty())
                else {
                    self.reply(
                        &rpc,
                        json!({"outcome":{"outcome":"rejected","reason":"Invalid plan"}}),
                    )?;
                    return Ok(events);
                };
                events.extend(self.text("text", plan));
                events.extend(self.flush());
                ("plan", "ExitPlanMode".into(), json!({"plan":plan}))
            }
            _ => {
                self.send(json!({"jsonrpc":"2.0","id":rpc,"error":{"code":-32601,"message":"Unsupported client method"}}))?;
                return Ok(events);
            }
        };
        let tool_id = params["toolCallId"]
            .as_str()
            .or_else(|| params["toolCall"]["toolCallId"].as_str());
        events.push(event(
            "request.opened",
            json!({"requestId":id,"kind":kind,"toolId":tool_id,"tool":tool,"input":input}),
        ));
        self.asks.insert(
            id,
            Ask {
                rpc,
                method: method.into(),
                params: params.clone(),
            },
        );
        Ok(events)
    }
    fn respond(&mut self, command: &Value) -> Result<Vec<Value>, String> {
        let id = command["requestId"]
            .as_str()
            .ok_or_else(|| i18n::t("err.team.bad"))?;
        let ask = self.asks.get(id).ok_or_else(|| i18n::t("err.team.bad"))?;
        let response = &command["response"];
        let outcome = response["outcome"].as_str().unwrap_or("");
        let allow = outcome == "allow";
        let result = match ask.method.as_str() {
            "session/request_permission" if matches!(outcome, "allow" | "deny") => {
                match permission_option(&ask.params, allow) {
                    Some(option) => json!({"outcome":{"outcome":"selected","optionId":option}}),
                    None if !allow => json!({"outcome":{"outcome":"cancelled"}}),
                    None => return Err(i18n::t("err.cursor.unsupported")),
                }
            }
            "cursor/create_plan" if matches!(outcome, "allow" | "deny") => {
                if allow {
                    json!({"outcome":{"outcome":"accepted"}})
                } else {
                    json!({"outcome":{"outcome":"rejected","reason":response["message"]}})
                }
            }
            "cursor/ask_question" if matches!(outcome, "answer" | "deny") => {
                question_answer(&ask.params, response)?
            }
            _ => return Err(i18n::t("err.team.bad")),
        };
        let rpc = ask.rpc.clone();
        self.reply(&rpc, result)?;
        self.asks.remove(id);
        Ok(vec![event(
            "request.closed",
            json!({"requestId":id,"outcome":match outcome { "allow" => "allowed", "answer" => "answered", _ => "denied" }}),
        )])
    }
}
fn permission_option(params: &Value, allow: bool) -> Option<&str> {
    params["options"]
        .as_array()?
        .iter()
        .find(|option| option["kind"] == if allow { "allow_once" } else { "reject_once" })?
        ["optionId"]
        .as_str()
}
fn question_answer(params: &Value, response: &Value) -> Result<Value, String> {
    if response["outcome"] == "deny" {
        return Ok(json!({"outcome":{"outcome":"cancelled"}}));
    }
    let mut answers = vec![];
    let mut feedback = vec![];
    let mut free_text = false;
    for question in params["questions"]
        .as_array()
        .ok_or_else(|| i18n::t("err.team.bad"))?
    {
        let prompt = question["prompt"]
            .as_str()
            .ok_or_else(|| i18n::t("err.team.bad"))?;
        let answer = response["answers"][prompt]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| i18n::t("err.team.bad"))?;
        let options = question["options"]
            .as_array()
            .ok_or_else(|| i18n::t("err.team.bad"))?;
        let selected: Vec<_> = options
            .iter()
            .filter(|option| {
                option["label"].as_str().is_some_and(|label| {
                    label == answer
                        || (question["allowMultiple"] == true
                            && answer.split(", ").any(|part| part == label))
                })
            })
            .map(|o| o["id"].clone())
            .collect();
        let mut selected_labels: Vec<_> = options
            .iter()
            .filter(|o| selected.contains(&o["id"]))
            .filter_map(|o| o["label"].as_str())
            .collect();
        if selected_labels.join(", ") != answer {
            let mut supplied: Vec<_> = answer.split(", ").collect();
            supplied.sort_unstable();
            selected_labels.sort_unstable();
            free_text |= question["allowMultiple"] != true || supplied != selected_labels;
        }
        feedback.push(format!("{prompt}: {answer}"));
        answers.push(json!({"questionId":question["id"],"selectedOptionIds":selected}));
    }
    // ACP's Cursor extension accepts option IDs only. Preserve free text in its supported reason
    // field instead of inventing option IDs or silently dropping the person's answer.
    if free_text
        || answers
            .iter()
            .any(|a| a["selectedOptionIds"].as_array().is_none_or(Vec::is_empty))
    {
        Ok(json!({"outcome":{"outcome":"skipped","reason":feedback.join("\n")}}))
    } else {
        Ok(json!({"outcome":{"outcome":"answered","answers":answers}}))
    }
}
fn tool_input(value: &Value) -> (String, Value) {
    let name = match value["kind"].as_str() {
        Some("execute") => "Bash",
        Some("read") => "Read",
        Some("edit") => "Edit",
        Some("search") => "Grep",
        _ => value["title"].as_str().unwrap_or("Tool"),
    };
    let mut input = value["rawInput"].as_object().cloned().unwrap_or_default();
    if !input.contains_key("command") && name == "Bash" {
        input.insert("command".into(), value["title"].clone());
    }
    if !input.contains_key("file_path") {
        if let Some(path) = input
            .get("path")
            .cloned()
            .or_else(|| value["locations"][0]["path"].as_str().map(|s| json!(s)))
        {
            input.insert("file_path".into(), path);
        }
    }
    (name.into(), Value::Object(input))
}

#[cfg(test)]
mod tests;
