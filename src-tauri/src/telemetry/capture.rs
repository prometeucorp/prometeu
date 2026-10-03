use super::*;
use crate::state::Board;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};
use tauri::Manager;

/// Resolve live Git branches without holding board, chat or capture locks. The board alone keeps
/// the names that supply opaque aliases; events contain only those aliases.
pub fn conversation_attribution(app: &tauri::AppHandle, conversation: &str) -> Origin {
    let state = app.state::<AppState>();
    let workspace = { lock(&state.board).workspace_of(conversation).cloned() };
    let Some(workspace) = workspace else {
        return Origin::default();
    };
    let branches: Vec<_> = workspace
        .repos
        .iter()
        .filter_map(|repo| {
            crate::github::head_branch(Path::new(&repo.worktree))
                .map(|branch| (repo.path.clone(), branch))
        })
        .collect();
    let (origin, changed) = {
        let mut board = lock(&state.board);
        let old = board.telemetry_ids.0.len();
        let action = board
            .tab_mut(conversation)
            .and_then(|t| t.task.as_ref())
            .map(|r| r.command.clone());
        let owner = board
            .delegations
            .iter()
            .find(|d| d.id == conversation)
            .map(|d| d.owner.clone());
        let origin = Origin {
            action_id: action.map(|action| board.telemetry_ids.ensure("action", &action)),
            delegated_by: owner.map(|owner| board.telemetry_ids.ensure("mcp-client", &owner)),
            repositories: branches
                .into_iter()
                .map(|(path, branch)| RepositoryBranch {
                    repository_id: board.telemetry_ids.ensure("repository", &path),
                    branch_id: board
                        .telemetry_ids
                        .ensure("branch", &format!("{path}\0{branch}")),
                })
                .collect(),
        };
        (origin, old != board.telemetry_ids.0.len())
    };
    if changed && crate::state::persist_now(app).is_err() {
        lock(&state.telemetry).failed();
        return Origin::default();
    }
    origin
}

/// A separate call lifecycle keeps app overhead visible without inventing conversation messages.
pub struct AppCapture {
    generation: u64,
    call_id: String,
    scope: Scope,
    source: AppSource,
    start: Instant,
    measurement: Measurement,
    outcome: Option<Outcome>,
}
impl AppCapture {
    pub fn start(
        service: &Mutex<Service>,
        generation: u64,
        scope: Scope,
        source: AppSource,
        selected: Option<String>,
    ) -> Self {
        let capture = Self {
            generation,
            call_id: id(),
            scope,
            source,
            start: Instant::now(),
            measurement: Measurement {
                selected_model: selected,
                ..Measurement::default()
            },
            outcome: None,
        };
        record(
            service,
            generation,
            capture.scope.clone(),
            Fact::AppCallStarted {
                call_id: capture.call_id.clone(),
                source: capture.source.clone(),
                measurement: capture.measurement.clone(),
            },
        );
        capture
    }
    pub fn observe(&mut self, frame: &Value) {
        let value = if frame["type"] == "telemetry.usage" {
            frame.get("measurement")
        } else {
            frame.get("telemetry").or_else(|| frame.get("usage"))
        };
        if let Some(mut measurement) =
            value.and_then(|value| serde_json::from_value::<Measurement>(value.clone()).ok())
        {
            measurement.selected_model = self.measurement.selected_model.clone();
            self.measurement = measurement;
        }
        if frame["type"] == "turn.completed" {
            self.outcome = match frame["outcome"].as_str() {
                Some("ok") => Some(Outcome::Ok),
                Some("error") => Some(Outcome::Error),
                Some("interrupted") => Some(Outcome::Interrupted),
                _ => None,
            };
        }
    }
    pub fn measurement(&mut self, mut measurement: Measurement) {
        measurement.selected_model = self.measurement.selected_model.clone();
        self.measurement = measurement;
    }
    pub fn finish(self, service: &Mutex<Service>, succeeded: bool) {
        let outcome = self.outcome.unwrap_or(if succeeded {
            Outcome::Ok
        } else {
            Outcome::Interrupted
        });
        record(
            service,
            self.generation,
            self.scope,
            Fact::AppCallCompleted {
                call_id: self.call_id,
                source: self.source,
                outcome,
                elapsed_ms: self.start.elapsed().as_millis() as u64,
                measurement: self.measurement,
            },
        );
    }
}
pub fn workspace_scope(board: &Board, workspace: &str) -> Option<Scope> {
    let w = board.workspace(workspace)?;
    Some(Scope {
        project_id: board.telemetry_ids.get("project", &w.project),
        workspace_id: board.telemetry_ids.get("workspace", &w.id),
        ..Scope::default()
    })
}
pub fn conversation_scope(board: &Board, conversation: &str) -> Option<(Scope, Option<String>)> {
    let w = board.workspace_of(conversation)?;
    let t = w.tabs.iter().find(|t| t.id == conversation)?;
    let choice = t.choice.as_ref();
    let provider = choice.map_or(w.agent, |c| c.agent);
    let model = choice.map_or(&w.model, |c| &c.model);
    let mut scope = workspace_scope(board, &w.id)?;
    scope.conversation_id = board.telemetry_ids.get("conversation", conversation);
    scope.provider = serde_json::to_value(provider)
        .ok()?
        .as_str()
        .map(str::to_string);
    Some((scope, (!model.is_empty()).then(|| model.clone())))
}
pub fn conversation_relations(board: &Board, conversation: &str) -> Vec<Event> {
    let Some(w) = board.workspace_of(conversation) else {
        return vec![];
    };
    let Some(scope) = workspace_scope(board, &w.id) else {
        return vec![];
    };
    w.repos
        .iter()
        .filter_map(|repo| {
            let pr = repo.pr.as_ref()?;
            Some(Event::new(
                scope.clone(),
                id(),
                Fact::PullRequestAssociated {
                    repository_id: board.telemetry_ids.get("repository", &repo.path)?,
                    branch_id: (!pr.head_ref_name.is_empty())
                        .then(|| {
                            board
                                .telemetry_ids
                                .get("branch", &format!("{}\0{}", repo.path, pr.head_ref_name))
                        })
                        .flatten(),
                    pull_request: pr.number,
                },
            ))
        })
        .collect()
}
pub fn journey(
    app: &tauri::AppHandle,
    generation: u64,
    workspace: &str,
    conversation: Option<&str>,
    fact: Fact,
) {
    let state = app.state::<AppState>();
    if lock(&state.telemetry).generation != generation {
        return;
    }
    if crate::state::persist_now(app).is_err() {
        let mut service = lock(&state.telemetry);
        if service.generation == generation {
            service.failed();
        }
        return;
    }
    let scope = {
        let board = lock(&state.board);
        match conversation {
            Some(c) => conversation_scope(&board, c).map(|v| v.0),
            None => workspace_scope(&board, workspace),
        }
    };
    if let Some(scope) = scope {
        record(&state.telemetry, generation, scope, fact);
    }
}
pub fn associate(
    app: &tauri::AppHandle,
    generation: u64,
    workspace: &str,
    repositories: &[(String, u64)],
) {
    let state = app.state::<AppState>();
    let events = {
        let board = lock(&state.board);
        let Some(w) = board.workspace(workspace) else {
            return;
        };
        let Some(scope) = workspace_scope(&board, workspace) else {
            return;
        };
        repositories
            .iter()
            .filter_map(|(path, number)| {
                Some((
                    scope.clone(),
                    Fact::PullRequestAssociated {
                        repository_id: board.telemetry_ids.get("repository", path)?,
                        branch_id: w
                            .repos
                            .iter()
                            .find(|r| r.path == *path)
                            .and_then(|r| r.pr.as_ref())
                            .filter(|pr| !pr.head_ref_name.is_empty())
                            .and_then(|pr| {
                                board
                                    .telemetry_ids
                                    .get("branch", &format!("{path}\0{}", pr.head_ref_name))
                            }),
                        pull_request: *number,
                    },
                ))
            })
            .collect::<Vec<_>>()
    };
    for (scope, fact) in events {
        record(&state.telemetry, generation, scope, fact);
    }
}

pub fn associate_history(
    app: &tauri::AppHandle,
    generation: u64,
    workspace: &str,
    path: &str,
    branch: &str,
    observations: &[crate::github::PrObservation],
    complete: bool,
) {
    let state = app.state::<AppState>();
    if lock(&state.telemetry).generation != generation {
        return;
    }
    let (scope, repository_id, branch_id, changed) = {
        let mut board = lock(&state.board);
        let Some(scope) = workspace_scope(&board, workspace) else {
            return;
        };
        let old = board.telemetry_ids.0.len();
        let repository_id = board.telemetry_ids.ensure("repository", path);
        let branch_id = board
            .telemetry_ids
            .ensure("branch", &format!("{path}\0{branch}"));
        (
            scope,
            repository_id,
            branch_id,
            old != board.telemetry_ids.0.len(),
        )
    };
    if changed && crate::state::persist_now(app).is_err() {
        let mut service = lock(&state.telemetry);
        if service.generation == generation {
            service.failed();
        }
        return;
    }
    let snapshot_id = id();
    let at = now();
    let timestamp = |value: &Option<String>| crate::github::lifecycle_timestamp(value.as_deref());
    notify_changed(app, || {
        let mut service = lock(&state.telemetry);
        let candidates: Vec<_> = observations
            .iter()
            .filter(|observation| observation.pr.head_ref_name == branch)
            .collect();
        for observation in &candidates {
            let pr = &observation.pr;
            let state = match pr.state.as_str() {
                "OPEN" => PullRequestState::Open,
                "CLOSED" => PullRequestState::Closed,
                "MERGED" => PullRequestState::Merged,
                _ => continue,
            };
            let mut event = Event::new(
                scope.clone(),
                id(),
                Fact::PullRequestObserved {
                    repository_id: repository_id.clone(),
                    branch_id: branch_id.clone(),
                    pull_request: pr.number,
                    state,
                    created_at: timestamp(&observation.created_at),
                    closed_at: timestamp(&observation.closed_at),
                    merged_at: timestamp(&observation.merged_at),
                    history_complete: complete
                        && [
                            &observation.created_at,
                            &observation.closed_at,
                            &observation.merged_at,
                        ]
                        .iter()
                        .all(|value| value.is_none() || timestamp(value).is_some()),
                    snapshot_id: snapshot_id.clone(),
                    history_size: candidates.len() as u64,
                    observed_after: observation.observed_after,
                },
            );
            event.occurred_at = at;
            service.capture(generation, &event);
            service.capture(
                generation,
                &Event::new(
                    scope.clone(),
                    id(),
                    Fact::PullRequestAssociated {
                        repository_id: repository_id.clone(),
                        branch_id: Some(branch_id.clone()),
                        pull_request: pr.number,
                    },
                ),
            );
        }
    });
}

struct Span {
    id: String,
    start: Instant,
    scope: Scope,
}
impl Span {
    fn new(scope: Scope) -> Self {
        Self {
            id: id(),
            start: Instant::now(),
            scope,
        }
    }
    fn elapsed(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}
#[derive(Default)]
pub struct Capture {
    generation: u64,
    scope: Option<Scope>,
    selected: Option<String>,
    turn: Option<String>,
    main: Option<Span>,
    /// New input cannot claim the terminal of a run that survived history deletion.
    erased_main: bool,
    children: HashMap<String, Span>,
    requests: HashMap<String, Span>,
    retired_children: HashSet<String>,
    retired_requests: HashSet<String>,
    measurement: Measurement,
}
impl Capture {
    pub fn initialized(&self) -> bool {
        self.scope.is_some()
    }
    #[cfg(test)]
    pub fn accepted(&mut self, service: &mut Service, scope: Scope, selected: Option<String>) {
        self.accepted_with_origin(service, scope, selected, Origin::default());
    }
    pub fn accepted_with_origin(
        &mut self,
        service: &mut Service,
        scope: Scope,
        selected: Option<String>,
        origin: Origin,
    ) {
        if self.generation != service.generation {
            self.erased_main |= self.main.take().is_some();
            self.retired_children.extend(self.children.keys().cloned());
            self.children.clear();
            self.retired_requests.extend(self.requests.keys().cloned());
            self.requests.clear();
        }
        let overlapping = self.main.is_some() || self.erased_main;
        self.generation = service.generation;
        self.selected = selected;
        let mut scope = scope;
        let turn = id();
        scope.turn_id = Some(turn.clone());
        self.turn = Some(turn);
        self.scope = Some(scope.clone());
        self.measurement = Measurement {
            selected_model: self.selected.clone(),
            ..Measurement::default()
        };
        self.emit(
            service,
            scope.clone(),
            Fact::TurnStarted {
                measurement: self.measurement.clone(),
                origin,
            },
            "start",
        );
        if overlapping {
            // Streaming input may queue or steer inside the provider. Without an acknowledgement
            // identifying that message, a later terminal cannot safely be assigned to either input.
            self.turn = None;
            if let Some(scope) = &mut self.scope {
                scope.turn_id = None;
            }
            service.failed();
            return;
        }
        self.main = Some(Span::new(scope));
        self.start_main(service);
    }
    fn emit(&self, service: &mut Service, scope: Scope, fact: Fact, suffix: &str) {
        let key = format!(
            "{}:{suffix}",
            scope.turn_id.as_deref().unwrap_or("activity")
        );
        service.capture(self.generation, &Event::new(scope, key, fact));
    }
    fn start_main(&self, service: &mut Service) {
        if let Some(span) = &self.main {
            self.emit(
                service,
                span.scope.clone(),
                Fact::ExecutionStarted {
                    execution_id: span.id.clone(),
                },
                &format!("{}:start", span.id),
            );
        }
    }
    fn end_main(&mut self, service: &mut Service) {
        if let Some(span) = self.main.take() {
            self.emit(
                service,
                span.scope.clone(),
                Fact::ExecutionCompleted {
                    execution_id: span.id.clone(),
                    elapsed_ms: span.elapsed(),
                },
                &format!("{}:end", span.id),
            );
        }
    }
    pub fn observe(&mut self, service: &mut Service, frame: &Value) {
        if self.generation != service.generation || self.erased_main {
            if frame["type"] == "turn.completed" {
                self.main = None;
                self.erased_main = false;
            }
            // Remember only native identities in memory so an old task snapshot cannot resurrect
            // erased executions after the next accepted message.
            if frame["type"] == "background.changed" {
                self.retired_children.extend(
                    frame["tasks"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|t| t["id"].as_str().map(str::to_string)),
                );
            }
            if frame["type"] == "request.opened" {
                if let Some(native) = frame["requestId"].as_str() {
                    self.retired_requests.insert(native.into());
                }
            }
            return;
        }
        let Some(scope) = self.scope.clone() else {
            return;
        };
        let kind = frame["type"].as_str().unwrap_or("");
        match kind {
            "turn.completed" => {
                if let Some(turn) = self.turn.take() {
                    if let Some(m) = frame
                        .get("telemetry")
                        .or_else(|| frame.get("usage"))
                        .and_then(|v| serde_json::from_value::<Measurement>(v.clone()).ok())
                    {
                        self.merge(m);
                    }
                    let outcome = match frame["outcome"].as_str() {
                        Some("ok") => Outcome::Ok,
                        Some("error") => Outcome::Error,
                        Some("interrupted") => Outcome::Interrupted,
                        _ => return,
                    };
                    let elapsed = self.main.as_ref().map_or(0, Span::elapsed);
                    let mut scope = scope;
                    scope.turn_id = Some(turn);
                    self.emit(
                        service,
                        scope.clone(),
                        Fact::TurnCompleted {
                            outcome,
                            elapsed_ms: elapsed,
                            provider_duration_ms: frame["providerDurationMs"].as_u64(),
                            measurement: self.measurement.clone(),
                            message_key: scope
                                .conversation_id
                                .as_deref()
                                .zip(frame["messageId"].as_str())
                                .map(|(conversation, message)| message_key(conversation, message)),
                        },
                        "completed",
                    );
                }
                self.end_main(service);
                if frame["outcome"] == "interrupted" {
                    self.drain_children(service, &HashSet::new());
                }
            }
            "telemetry.usage" if self.turn.is_some() => {
                if let Ok(m) = serde_json::from_value::<Measurement>(frame["measurement"].clone()) {
                    self.merge(m);
                    self.snapshot(service, &scope);
                }
            }
            "context.updated" if self.turn.is_some() => {
                if let Some(used) = frame["used"].as_u64() {
                    self.measurement.usage.context_used = Some(used);
                    self.measurement.usage.peak_context =
                        Some(self.measurement.usage.peak_context.unwrap_or(0).max(used));
                }
                if let Some(window) = frame["window"].as_u64() {
                    self.measurement.usage.context_window = Some(window);
                }
                self.snapshot(service, &scope);
            }
            "context.compacted" => {
                self.measurement.usage.compactions =
                    Some(self.measurement.usage.compactions.unwrap_or(0) + 1);
                self.emit(
                    service,
                    scope,
                    Fact::ContextCompacted {
                        before: frame["before"].as_u64(),
                        after: frame["after"].as_u64(),
                    },
                    &id(),
                );
            }
            "request.opened" => {
                let Some(native) = frame["requestId"].as_str() else {
                    return;
                };
                if self.requests.contains_key(native) || self.retired_requests.contains(native) {
                    return;
                }
                let kind = match frame["kind"].as_str() {
                    Some("approval") => RequestKind::Approval,
                    Some("question") => RequestKind::Question,
                    Some("plan") => RequestKind::Plan,
                    _ => return,
                };
                let span = Span::new(scope.clone());
                self.emit(
                    service,
                    scope,
                    Fact::HumanRequested {
                        request_id: span.id.clone(),
                        kind,
                    },
                    &format!("{}:request", span.id),
                );
                self.requests.insert(native.into(), span);
            }
            "request.closed" => {
                let Some(native) = frame["requestId"].as_str() else {
                    return;
                };
                if !matches!(
                    frame["outcome"].as_str(),
                    Some("allowed" | "denied" | "answered" | "cancelled")
                ) {
                    return;
                }
                if let Some(span) = self.requests.remove(native) {
                    let fact = if frame["outcome"] == "cancelled" {
                        Fact::HumanCancelled {
                            request_id: span.id.clone(),
                            elapsed_ms: span.elapsed(),
                        }
                    } else {
                        Fact::HumanReceived {
                            request_id: span.id.clone(),
                            elapsed_ms: span.elapsed(),
                        }
                    };
                    self.emit(service, span.scope, fact, &format!("{}:closed", span.id));
                    self.retired_requests.insert(native.into());
                }
            }
            "background.changed" => {
                let Some(tasks) = frame["tasks"].as_array() else {
                    return;
                };
                let present: HashSet<String> = tasks
                    .iter()
                    .filter_map(|t| t["id"].as_str().map(str::to_string))
                    .collect();
                self.drain_children(service, &present);
                for native in present {
                    if self.children.contains_key(&native)
                        || self.retired_children.contains(&native)
                    {
                        continue;
                    }
                    let span = Span::new(scope.clone());
                    self.emit(
                        service,
                        scope.clone(),
                        Fact::ExecutionStarted {
                            execution_id: span.id.clone(),
                        },
                        &format!("{}:start", span.id),
                    );
                    self.children.insert(native, span);
                }
            }
            _ if crate::conversation::agent_activity(frame) && self.main.is_none() => {
                let mut scope = scope;
                scope.turn_id = None;
                self.main = Some(Span::new(scope));
                self.start_main(service);
            }
            _ => {}
        }
    }
    fn snapshot(&self, service: &mut Service, scope: &Scope) {
        self.emit(
            service,
            scope.clone(),
            Fact::UsageObserved {
                measurement: self.measurement.clone(),
            },
            &id(),
        );
    }
    fn merge(&mut self, m: Measurement) {
        // Measurements are snapshots; context and compaction observations are captured separately.
        let old = &self.measurement.usage;
        let mut usage = m.usage;
        usage.context_used = usage.context_used.or(old.context_used);
        usage.context_window = usage.context_window.or(old.context_window);
        usage.peak_context = usage.peak_context.or(old.peak_context);
        usage.compactions = usage.compactions.or(old.compactions);
        self.measurement = Measurement {
            selected_model: self.selected.clone(),
            usage,
            ..m
        };
    }
    fn drain_children(&mut self, service: &mut Service, present: &HashSet<String>) {
        let gone: Vec<_> = self
            .children
            .keys()
            .filter(|k| !present.contains(*k))
            .cloned()
            .collect();
        for key in gone {
            if let Some(span) = self.children.remove(&key) {
                self.emit(
                    service,
                    span.scope.clone(),
                    Fact::ExecutionCompleted {
                        execution_id: span.id.clone(),
                        elapsed_ms: span.elapsed(),
                    },
                    &format!("{}:end", span.id),
                );
            }
        }
    }
}
