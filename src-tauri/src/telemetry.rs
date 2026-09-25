//! Local, content-free facts. Only this module owns the database; consumers use concrete queries.
mod capture;
mod query;
#[cfg(test)]
mod tests;
use crate::{conversation::now, lock::lock, paths, AppState};
pub use capture::{associate, conversation_relations, conversation_scope, journey, Capture};
pub use query::{Filter, Page, Summary};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};
use tauri::State;

pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
const FAILURE: &str = "err.telemetry.storage";
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub context_used: Option<u64>,
    pub context_window: Option<u64>,
    pub peak_context: Option<u64>,
    pub model_calls: Option<u64>,
    pub compactions: Option<u64>,
    pub cache_rebuilds: Option<u64>,
    pub cost_usd: Option<f64>,
}
impl Usage {
    fn valid(&self) -> bool {
        self.cost_usd.is_none_or(|n| n.is_finite() && n >= 0.)
            && [
                (self.cache_read_tokens, self.input_tokens),
                (self.cache_write_tokens, self.input_tokens),
                (self.reasoning_tokens, self.output_tokens),
            ]
            .iter()
            .all(|(part, total)| match (part, total) {
                (Some(p), Some(t)) => p <= t,
                _ => true,
            })
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Measurement {
    /// Usage covers the observed main agent, excluding independently running children.
    pub usage_scope: UsageScope,
    /// Both turn totals are complete; known partial sums remain useful when false.
    #[serde(default)]
    pub complete: bool,
    pub selected_model: Option<String>,
    pub observed_models: Option<Vec<String>>,
    pub usage: Usage,
    pub usage_by_model: Option<Vec<ModelUsage>>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum UsageScope {
    #[default]
    MainAgent,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsage {
    pub model: String,
    pub usage: Usage,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Error,
    Interrupted,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RequestKind {
    Approval,
    Question,
    Plan,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CreationMode {
    Worktree,
    Repository,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionScope {
    Workspace,
    Conversation,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", deny_unknown_fields)]
pub enum Fact {
    #[serde(rename = "workspace.created")]
    WorkspaceCreated { mode: CreationMode },
    #[serde(rename = "workspace.archived")]
    WorkspaceArchived {},
    #[serde(rename = "workspace.resumed")]
    WorkspaceResumed {},
    #[serde(rename = "conversation.created")]
    ConversationCreated {},
    #[serde(rename = "provider.selected")]
    ProviderSelected { scope: SelectionScope },
    #[serde(rename = "pull_request.associated", rename_all = "camelCase")]
    PullRequestAssociated {
        repository_id: String,
        branch_id: Option<String>,
        pull_request: u64,
    },
    #[serde(rename = "turn.started")]
    TurnStarted { measurement: Measurement },
    #[serde(rename = "turn.completed", rename_all = "camelCase")]
    TurnCompleted {
        outcome: Outcome,
        elapsed_ms: u64,
        provider_duration_ms: Option<u64>,
        measurement: Measurement,
    },
    #[serde(rename = "turn.usage.observed")]
    UsageObserved { measurement: Measurement },
    #[serde(rename = "agent.execution.started", rename_all = "camelCase")]
    ExecutionStarted { execution_id: String },
    #[serde(rename = "agent.execution.completed", rename_all = "camelCase")]
    ExecutionCompleted {
        execution_id: String,
        elapsed_ms: u64,
    },
    #[serde(rename = "human_input.requested", rename_all = "camelCase")]
    HumanRequested {
        request_id: String,
        kind: RequestKind,
    },
    #[serde(rename = "human_input.received", rename_all = "camelCase")]
    HumanReceived { request_id: String, elapsed_ms: u64 },
    #[serde(rename = "human_input.cancelled", rename_all = "camelCase")]
    HumanCancelled { request_id: String, elapsed_ms: u64 },
    #[serde(rename = "context.compacted")]
    ContextCompacted {
        before: Option<u64>,
        after: Option<u64>,
    },
}
impl Fact {
    fn category(&self) -> &'static str {
        match self {
            Self::WorkspaceCreated { .. }
            | Self::WorkspaceArchived { .. }
            | Self::WorkspaceResumed { .. }
            | Self::ConversationCreated { .. }
            | Self::ProviderSelected { .. } => "journey",
            _ => "work",
        }
    }
    fn measurement(&self) -> Option<&Measurement> {
        match self {
            Self::TurnStarted { measurement }
            | Self::TurnCompleted { measurement, .. }
            | Self::UsageObserved { measurement } => Some(measurement),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub project_id: Option<String>,
    pub workspace_id: Option<String>,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub provider: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub sequence: u64,
    pub id: String,
    pub occurrence_key: String,
    pub schema_version: u32,
    pub occurred_at: u64,
    pub recorded_at: u64,
    pub category: String,
    #[serde(flatten)]
    pub scope: Scope,
    #[serde(flatten)]
    pub fact: Fact,
}
impl Event {
    pub fn new(scope: Scope, occurrence: String, fact: Fact) -> Self {
        Self {
            sequence: 0,
            id: id(),
            occurrence_key: occurrence,
            schema_version: 1,
            occurred_at: now(),
            recorded_at: 0,
            category: fact.category().into(),
            scope,
            fact,
        }
    }
    fn validate(&self) -> Result<()> {
        let uuid = |s: &str| uuid::Uuid::parse_str(s).is_ok();
        let identifier = |s: &str| {
            !s.is_empty()
                && s.len() <= 200
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
        };
        let scopes = [
            &self.scope.project_id,
            &self.scope.workspace_id,
            &self.scope.conversation_id,
            &self.scope.turn_id,
        ];
        let model = |m: &Measurement| {
            m.usage.valid()
                && m.selected_model.as_deref().is_none_or(identifier)
                && m.observed_models
                    .as_ref()
                    .is_none_or(|v| v.iter().all(|s| identifier(s)))
                && m.usage_by_model
                    .as_ref()
                    .is_none_or(|v| v.iter().all(|r| identifier(&r.model) && r.usage.valid()))
        };
        let fact_valid = match &self.fact {
            Fact::PullRequestAssociated {
                repository_id,
                branch_id,
                pull_request,
            } => {
                uuid(repository_id)
                    && branch_id.as_deref().is_none_or(uuid)
                    && *pull_request > 0
                    && *pull_request <= i64::MAX as u64
            }
            Fact::ExecutionStarted { execution_id }
            | Fact::ExecutionCompleted { execution_id, .. } => uuid(execution_id),
            Fact::HumanRequested { request_id, .. }
            | Fact::HumanReceived { request_id, .. }
            | Fact::HumanCancelled { request_id, .. } => uuid(request_id),
            _ => true,
        };
        let turn_required = matches!(
            self.fact,
            Fact::TurnStarted { .. } | Fact::TurnCompleted { .. } | Fact::UsageObserved { .. }
        );
        if !uuid(&self.id)
            || self.schema_version != 1
            || self.category != self.fact.category()
            || !identifier(&self.occurrence_key)
            || self.occurred_at > i64::MAX as u64
            || !scopes.iter().all(|s| s.as_deref().is_none_or(uuid))
            || self.scope.workspace_id.is_none()
            || (turn_required
                && (self.scope.turn_id.is_none() || self.scope.conversation_id.is_none()))
            || !self
                .scope
                .provider
                .as_deref()
                .is_none_or(|p| matches!(p, "claude" | "codex" | "antigravity"))
            || !fact_valid
            || !self.fact.measurement().is_none_or(model)
        {
            return Err("err.telemetry.invalid".into());
        }
        Ok(())
    }
}

pub struct Store {
    db: Connection,
}
impl Store {
    fn open(path: &Path) -> Result<Self> {
        let work = || -> std::result::Result<Self, Box<dyn std::error::Error>> {
            paths::ensure_private_dir(path.parent().ok_or("parent")?)?;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .mode(0o600)
                .open(path)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            let mut db = Connection::open(path)?;
            db.busy_timeout(std::time::Duration::from_millis(250))?;
            let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if version > 1 {
                return Err("future_version".into());
            }
            // Snapshot readers use WAL so summaries and exports never hold up live commits.
            db.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;",
            )?;
            if version == 0 {
                let tx = db.transaction()?;
                tx.execute_batch("CREATE TABLE events (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
                    occurrence_key TEXT NOT NULL UNIQUE, schema_version INTEGER NOT NULL,
                    occurred_at INTEGER NOT NULL, recorded_at INTEGER NOT NULL, category TEXT NOT NULL,
                    type TEXT NOT NULL, project_id TEXT, workspace_id TEXT, conversation_id TEXT,
                    turn_id TEXT, provider TEXT, payload TEXT NOT NULL);
                    CREATE INDEX events_time ON events(occurred_at, sequence);
                    CREATE INDEX events_workspace_time ON events(workspace_id, occurred_at, sequence);
                    CREATE INDEX events_turn ON events(turn_id, sequence);
                    PRAGMA user_version=1;")?;
                tx.commit()?;
            }
            db.execute_batch("CREATE INDEX IF NOT EXISTS events_execution ON events(type, json_extract(payload,'$.executionId'), sequence);
                CREATE INDEX IF NOT EXISTS events_request ON events(type, json_extract(payload,'$.requestId'), sequence);")?;
            Ok(Self { db })
        };
        work().map_err(|_| FAILURE.into())
    }
    fn append(&mut self, event: &Event) -> Result<()> {
        event.validate()?;
        let mut event = event.clone();
        event.recorded_at = now();
        let tagged = serde_json::to_value(&event.fact).map_err(|_| FAILURE)?;
        let payload = serde_json::to_string(&tagged["payload"]).map_err(|_| FAILURE)?;
        let tx = self.db.transaction().map_err(|_| FAILURE)?;
        let existing: Option<Event> = tx
            .query_row(
                "SELECT * FROM events WHERE occurrence_key=?1 OR id=?2",
                params![event.occurrence_key, event.id],
                query::row,
            )
            .optional()
            .map_err(|_| FAILURE)?;
        if let Some(mut old) = existing {
            old.sequence = 0;
            old.recorded_at = event.recorded_at;
            if old == event {
                return Ok(());
            }
            return Err("err.telemetry.conflict".into());
        }
        if let Fact::PullRequestAssociated {
            repository_id,
            pull_request,
            ..
        } = &event.fact
        {
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE type='pull_request.associated' AND workspace_id=?1 AND json_extract(payload,'$.repositoryId')=?2 AND json_extract(payload,'$.pullRequest')=?3)",
                params![event.scope.workspace_id,repository_id,*pull_request as i64], |r| r.get(0)).map_err(|_| FAILURE)?;
            if exists {
                return Ok(());
            }
        }
        tx.execute("INSERT INTO events(id,occurrence_key,schema_version,occurred_at,recorded_at,category,type,project_id,workspace_id,conversation_id,turn_id,provider,payload) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", params![event.id,event.occurrence_key,event.schema_version,event.occurred_at as i64,event.recorded_at as i64,event.category,tagged["type"].as_str(),event.scope.project_id,event.scope.workspace_id,event.scope.conversation_id,event.scope.turn_id,event.scope.provider,payload]).map_err(|_| FAILURE)?;
        tx.commit().map_err(|_| FAILURE.into())
    }
    fn clear(&mut self) -> Result<()> {
        let tx = self.db.transaction().map_err(|_| FAILURE)?;
        tx.execute_batch("DELETE FROM events; DELETE FROM sqlite_sequence WHERE name='events';")
            .map_err(|_| FAILURE)?;
        tx.commit().map_err(|_| FAILURE)?;
        self.db.execute_batch("VACUUM;").map_err(|_| FAILURE)?;
        let busy: u32 = self
            .db
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .map_err(|_| FAILURE)?;
        if busy != 0 {
            return Err(FAILURE.into());
        }
        Ok(())
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub failures: u64,
    pub last_failure_at: Option<u64>,
    pub unavailable: bool,
}

pub struct Service {
    store: Option<Store>,
    root: PathBuf,
    pub generation: u64,
    health: Health,
    readers: Arc<RwLock<()>>,
}
impl Service {
    pub fn new(root: PathBuf) -> Self {
        let health = std::fs::read(root.join("telemetry-health.json"))
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        let mut s = Self {
            store: None,
            root,
            generation: 0,
            health,
            readers: Arc::new(RwLock::new(())),
        };
        match Store::open(&s.root.join("telemetry.sqlite3")) {
            Ok(store) => {
                s.store = Some(store);
                s.health.unavailable = false
            }
            Err(_) => s.failed(),
        }
        s
    }
    pub(crate) fn failed(&mut self) {
        self.health.failures = self.health.failures.saturating_add(1);
        self.health.last_failure_at = Some(now());
        self.health.unavailable = self.store.is_none();
        eprintln!("telemetry: capture unavailable or incomplete");
        if let Ok(text) = serde_json::to_string(&self.health) {
            let _ = paths::write_private(&self.root.join("telemetry-health.json"), &text);
        }
    }
    pub fn capture(&mut self, generation: u64, event: &Event) {
        if generation != self.generation {
            return;
        }
        if self
            .store
            .as_mut()
            .ok_or_else(|| FAILURE.into())
            .and_then(|s| s.append(event))
            .is_err()
        {
            self.failed();
        }
    }
    fn clear(&mut self) -> Result<()> {
        let readers = self.readers.clone();
        let _exclusive = readers.write().unwrap_or_else(|e| e.into_inner());
        // Invalidate old in-flight capture even when erasure fails; never restore deleted rows.
        self.generation = self.generation.wrapping_add(1);
        let result = self
            .store
            .as_mut()
            .ok_or_else(|| FAILURE.to_string())
            .and_then(Store::clear);
        if result.is_err() {
            self.failed();
            return result;
        }
        let path = self.root.join("telemetry-health.json");
        if path.exists() {
            std::fs::remove_file(path).map_err(|_| FAILURE)?;
        }
        self.health = Health::default();
        Ok(())
    }
}

#[tauri::command(async)]
pub fn telemetry_summary(state: State<AppState>, filter: Filter) -> Result<Summary> {
    let queries = lock(&state.telemetry).queries();
    queries.summary(&filter)
}
#[tauri::command(async)]
pub fn telemetry_events(
    state: State<AppState>,
    filter: Filter,
    cursor: Option<query::Cursor>,
) -> Result<Page> {
    let queries = lock(&state.telemetry).queries();
    queries.page(&filter, cursor)
}
#[tauri::command(async)]
pub fn telemetry_export(state: State<AppState>, filter: Filter, path: String) -> Result<()> {
    let queries = lock(&state.telemetry).queries();
    queries.export_to(&filter, Path::new(&path))
}
fn write_export(
    path: &Path,
    write: impl FnOnce(&mut dyn std::io::Write) -> Result<()>,
) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    const EXPORT_FAILURE: &str = "err.telemetry.export";
    if path.extension().and_then(|v| v.to_str()) != Some("jsonl") {
        return Err(EXPORT_FAILURE.into());
    }
    let validate_destination = || match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(EXPORT_FAILURE.to_string()),
    };
    validate_destination()?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory = std::fs::File::open(parent).map_err(|_| EXPORT_FAILURE)?;
    // Keep streamed data beside the destination so publication is one atomic rename. Unlike
    // private app state, an export must never change its parent directory's permissions.
    // ponytail: crashes can leave private siblings; tracked recovery is needed for automatic cleanup.
    let temporary = path.with_file_name(format!(".prometeu-telemetry-{}.tmp", id()));
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| EXPORT_FAILURE)?;
    let result = (|| {
        let mut writer = std::io::BufWriter::new(file);
        write(&mut writer)?;
        writer.flush().map_err(|_| EXPORT_FAILURE)?;
        writer.get_ref().sync_all().map_err(|_| EXPORT_FAILURE)?;
        drop(writer);
        validate_destination()?;
        // Rename replaces a directory entry, never follows a symlink swapped in after validation.
        std::fs::rename(&temporary, path).map_err(|_| EXPORT_FAILURE)?;
        // File sync persists contents; directory sync persists the replacement itself.
        directory.sync_all().map_err(|_| EXPORT_FAILURE.into())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[tauri::command(async)]
pub fn telemetry_clear(state: State<AppState>) -> Result<()> {
    lock(&state.telemetry).clear()
}

/// Map legacy path-based domain identities in board state, never in the telemetry database.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Identities(pub std::collections::BTreeMap<String, String>);
impl Identities {
    pub fn ensure(&mut self, kind: &str, source: &str) -> String {
        self.0
            .entry(format!("{kind}:{source}"))
            .or_insert_with(|| {
                if matches!(kind, "workspace" | "conversation")
                    && uuid::Uuid::parse_str(source).is_ok()
                {
                    source.into()
                } else {
                    id()
                }
            })
            .clone()
    }
    pub fn get(&self, kind: &str, source: &str) -> Option<String> {
        self.0.get(&format!("{kind}:{source}")).cloned()
    }
}
/// Locks only telemetry state. Call after releasing board and publication locks.
pub fn record(service: &Mutex<Service>, generation: u64, scope: Scope, fact: Fact) {
    let event = Event::new(scope, id(), fact);
    let mut service = lock(service);
    service.capture(generation, &event);
}
