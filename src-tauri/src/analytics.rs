//! Anonymous product analytics sent to PostHog (ADR 0093). Only the closed `Event` vocabulary
//! crosses the network: fields are enums, booleans or explicit identifier tokens we author, never
//! free text, paths, names or content. Explicit opt-in is required. Builds without
//! `PROMETEU_POSTHOG_KEY` (development, tests, forks) send nothing.

use crate::{i18n, paths};
use prometeu_core::board::ProviderId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The project lives in PostHog US Cloud; keys only work against their own region.
const HOST: &str = "https://us.i.posthog.com";

/// Clicks arrive in bursts; one request per window instead of one per click.
const BATCH_WINDOW: Duration = Duration::from_secs(5);

/// Names are `object_verb` in the past tense. Add variants here and in `src/analytics.ts` and
/// document them in `docs/contracts/product-analytics.md`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "name", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    AppOpened,
    AnalyticsConsentGranted,
    ScreenViewed {
        screen: Screen,
    },
    WorkspaceCreated {
        provider: ProviderId,
        is_worktree: bool,
    },
    ConversationCreated {
        provider: ProviderId,
    },
    SettingsSectionViewed {
        section: SettingsSection,
    },
    SettingsSearchUsed,
    PreferenceChanged {
        setting: PreferenceSetting,
    },
    ResourceActionUsed {
        kind: ResourceKind,
        action: ResourceAction,
    },
    ResourceSelectionChanged {
        kind: ResourceKind,
        scope: ResourceScope,
    },
    IntegrationActionUsed {
        integration: IntegrationKind,
        action: IntegrationAction,
    },
    ProviderAccountActionUsed {
        provider: ProviderId,
        action: ProviderAccountAction,
    },
    ActionConfigurationChanged {
        kind: ActionConfigurationKind,
        action: ConfigurationAction,
    },
    IssueActionUsed {
        integration: IssueIntegration,
        action: IssueAction,
    },
    NotificationActionUsed {
        source: NotificationSource,
        action: NotificationAction,
    },
    CollaborationActionUsed {
        action: CollaborationAction,
    },
    UsageHistoryActionUsed {
        action: UsageHistoryAction,
    },
    UpdateActionUsed {
        action: UpdateAction,
    },
    ReleaseNotesViewed,
    FeedbackActionUsed {
        channel: FeedbackChannel,
        action: FeedbackAction,
        has_attachment: bool,
    },
    TrustDecisionMade {
        approved: bool,
    },
    DeskLayoutChanged {
        change: DeskLayoutChange,
    },
    ConversationOpened {
        surface: ConversationSurface,
    },
    ConversationResumed,
    AttachmentAdded,
    ContextAttached,
    OutputCopied {
        kind: OutputKind,
    },
    RequestAnswered,
    ApprovalSubmitted {
        approved: bool,
    },
    ModelSelectionChanged {
        provider: ProviderId,
    },
    ModelFavoriteChanged {
        is_favorite: bool,
    },
    EffortChanged {
        provider: ProviderId,
    },
    UsageSummaryViewed,
    FileOpened {
        kind: FileKind,
    },
    FileSearchUsed,
    DiffViewed,
    GitComparisonOpened,
    ReviewNoteCreated,
    ReviewNotesSent,
    PreviewOpened,
    BrowserContextAttached,
    PreviewCaptureCreated,
    DictationStarted,
    DictationTextInserted,
    ActionUsed {
        kind: ActionKind,
    },
    MachinePanelOpened,
    QuotaViewed,
    ProjectActionUsed {
        action: ProjectAction,
    },
    WorkspaceActionUsed {
        action: WorkspaceAction,
    },
    FileOperationCompleted {
        operation: FileOperation,
    },
    GitOperationCompleted {
        operation: GitOperation,
    },
    TerminalOpened,
    ProjectScriptStarted {
        kind: ScriptKind,
    },
    ProjectScriptStopped {
        kind: ScriptKind,
    },
    DelegationActionUsed {
        action: DelegationAction,
        client: DelegationClient,
    },
    UiClicked {
        control: Control,
        target: Token,
        area: Token,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SettingsSection {
    General,
    Agents,
    Resources,
    Actions,
    Work,
    Analytics,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceSetting {
    Language,
    DictationLanguage,
    DefaultModel,
    DefaultEffort,
    Notifications,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Mcp,
    Plugin,
    Skill,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceAction {
    Save,
    Remove,
    Install,
    Update,
    Copy,
    Share,
    Authenticate,
    Disconnect,
    Check,
    Discover,
    Generate,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceScope {
    Defaults,
    Global,
    Workspace,
    Project,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    Linear,
    Github,
    Typesafe,
    Cloud,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationAction {
    Connect,
    Disconnect,
    Configure,
    Test,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccountAction {
    LoginStart,
    LoginCancel,
    Select,
    Remove,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActionConfigurationKind {
    Command,
    Profile,
    Project,
    PrAction,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationAction {
    Save,
    Remove,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueIntegration {
    Linear,
    Github,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueAction {
    PrepareWorkspace,
    Claim,
    OpenSource,
    OpenWorkspace,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSource {
    Github,
    Team,
    Local,
    Island,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationAction {
    Open,
    Read,
    ReadAll,
    OpenSource,
    OpenWorkspace,
    Dismiss,
    Answer,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationAction {
    SelectOrganization,
    Share,
    Unshare,
    Grant,
    Revoke,
    CompanionEnable,
    CompanionDisable,
    Leave,
    Readmit,
    AllowInput,
    DismissInput,
    SendPending,
    EditPending,
    DiscardPending,
    Comment,
    Reply,
    Resolve,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UsageHistoryAction {
    View,
    Refresh,
    Filter,
    Export,
    Clear,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAction {
    Check,
    Install,
    Restart,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackChannel {
    Private,
    Public,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackAction {
    Open,
    Submit,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeskLayoutChange {
    Visibility,
    Order,
    Size,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationSurface {
    Desk,
    Workspace,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Message,
    Code,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Text,
    Markdown,
    Image,
    Csv,
    Pdf,
    Binary,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Prompt,
    Agent,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAction {
    Add,
    Create,
    Remove,
    Reorder,
    Group,
    Configure,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAction {
    Rename,
    Pin,
    Unpin,
    Stage,
    Finish,
    Archive,
    Restore,
    Remove,
    Cleanup,
    MarkRead,
    MarkUnread,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    Save,
    CreateFile,
    CreateFolder,
    Rename,
    Move,
    Trash,
    Restore,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    Stage,
    Unstage,
    Commit,
    Fetch,
    Pull,
    Push,
    Publish,
    Discard,
    Resolve,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScriptKind {
    Setup,
    Run,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationAction {
    Create,
    Send,
    Interrupt,
    Inspect,
    Files,
    Setup,
    Run,
    Preview,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationClient {
    Internal,
    External,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    Desk,
    Settings,
    Issues,
    Notifications,
    Archived,
    Workspace,
    Project,
    Terminal,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Button,
    Link,
    Menuitem,
    Tab,
    Option,
    Checkbox,
    Input,
}

/// An identifier shaped like an i18n key or element ID. The shape rejects sentences, paths and
/// URLs even if a caller passes visible text by mistake.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(try_from = "String")]
pub struct Token(String);

impl TryFrom<String> for Token {
    type Error = &'static str;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        let shaped = (1..=80).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-');
        if shaped {
            Ok(Token(text))
        } else {
            Err("not an identifier token")
        }
    }
}

static QUEUE: OnceLock<Option<Sender<(u64, Event, Instant)>>> = OnceLock::new();
static DELIVERY: OnceLock<Delivery> = OnceLock::new();

struct Delivery {
    consent: Mutex<Consent>,
    sending: Mutex<()>,
}

impl Delivery {
    fn new(path: &Path) -> Self {
        Self {
            consent: Mutex::new(Consent::load(path)),
            sending: Mutex::new(()),
        }
    }

    /// Returns the saved choice and whether this save turned sharing on.
    fn save(&self, path: &Path, enabled: bool) -> Result<(bool, bool), String> {
        let _sending = self.sending.lock().unwrap_or_else(|e| e.into_inner());
        let mut consent = self.consent.lock().unwrap_or_else(|e| e.into_inner());
        let was = consent.enabled;
        let saved = consent.save(path, enabled)?;
        Ok((saved, saved && !was))
    }

    fn send_batch(
        &self,
        events: Vec<(u64, Event, Instant)>,
        id: &str,
        send: impl FnOnce(Vec<Value>),
    ) {
        // Hold the delivery gate through HTTP completion, but never hold the capture lock
        // across network I/O. A successful disable cannot be followed by a stale send.
        let _sending = self.sending.lock().unwrap_or_else(|e| e.into_inner());
        let batch = self
            .consent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .batch(events.into_iter(), id, Instant::now());
        if !batch.is_empty() {
            send(batch);
        }
    }
}

struct Consent {
    enabled: bool,
    decided: bool,
    generation: u64,
}

impl Consent {
    fn load(path: &Path) -> Self {
        // Missing, unreadable and malformed preferences never imply consent, including upgrades.
        let enabled = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok());
        Self {
            enabled: enabled.unwrap_or(false),
            decided: enabled.is_some(),
            generation: 0,
        }
    }

    fn generation(&self) -> Option<u64> {
        self.enabled.then_some(self.generation)
    }

    fn save(&mut self, path: &Path, enabled: bool) -> Result<bool, String> {
        paths::write_private(path, if enabled { "true" } else { "false" })
            .map_err(|_| i18n::t("err.analytics.storage"))?;
        self.decided = true;
        if self.enabled != enabled {
            self.generation += 1;
            self.enabled = enabled;
        }
        Ok(self.enabled)
    }

    fn batch(
        &self,
        events: impl Iterator<Item = (u64, Event, Instant)>,
        id: &str,
        now: Instant,
    ) -> Vec<Value> {
        events
            .filter(|(generation, _, _)| self.generation() == Some(*generation))
            .map(|(_, event, occurred)| payload(event, id, now.saturating_duration_since(occurred)))
            .collect()
    }
}

fn delivery() -> &'static Delivery {
    DELIVERY.get_or_init(|| Delivery::new(&paths::root().join("analytics-enabled.json")))
}

#[tauri::command]
pub fn analytics_needs_consent() -> bool {
    !delivery()
        .consent
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .decided
}

#[tauri::command]
pub fn analytics_enabled() -> bool {
    delivery()
        .consent
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .enabled
}

#[tauri::command]
pub async fn analytics_set_enabled(enabled: bool) -> Result<bool, String> {
    // An in-flight request may take up to its timeout; keep that wait off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let (saved, granted) =
            delivery().save(&paths::root().join("analytics-enabled.json"), enabled)?;
        // Only opting in is observable; declining or disabling sends nothing.
        if granted {
            track(Event::AnalyticsConsentGranted);
        }
        Ok(saved)
    })
    .await
    .map_err(|_| i18n::t("err.analytics.storage"))?
}

/// Never waits for network delivery or fails the caller; delivery problems drop the event.
pub fn track(event: Event) {
    let occurred = Instant::now();
    let Some(generation) = delivery()
        .consent
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .generation()
    else {
        return;
    };
    // CI defines the variable as empty when the repository variable is missing.
    let queue = QUEUE.get_or_init(|| {
        option_env!("PROMETEU_POSTHOG_KEY")
            .filter(|key| !key.trim().is_empty())
            .map(start)
    });
    if let Some(queue) = queue {
        let _ = queue.send((generation, event, occurred));
    }
}

#[tauri::command]
pub fn analytics_track(event: Event) {
    track(event);
}

fn start(key: &'static str) -> Sender<(u64, Event, Instant)> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || deliver(key, receiver));
    sender
}

// ponytail: no retry or offline queue; a failed batch, or one still waiting when the app quits,
// is lost. Add a bounded on-disk queue if coverage gaps matter.
fn deliver(key: &str, events: Receiver<(u64, Event, Instant)>) {
    let id = install_id();
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return;
    };
    while let Ok(first) = events.recv() {
        std::thread::sleep(BATCH_WINDOW);
        let batch = std::iter::once(first).chain(events.try_iter()).collect();
        delivery().send_batch(batch, &id, |batch| {
            let _ = client
                .post(format!("{HOST}/batch/"))
                .json(&json!({ "api_key": key, "batch": batch }))
                .send();
        });
    }
}

/// A random identifier per installation, unrelated to accounts, machines or paths. It lets
/// PostHog group events for funnels and retention without identifying a person.
fn install_id() -> String {
    let path = paths::root().join("analytics-id");
    if let Some(id) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| uuid::Uuid::parse_str(text.trim()).ok())
    {
        return id.to_string();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ =
        paths::ensure_private_dir(&paths::root()).and_then(|_| paths::write_private(&path, &id));
    id
}

fn payload(event: Event, id: &str, delay: Duration) -> Value {
    let Ok(Value::Object(mut properties)) = serde_json::to_value(event) else {
        unreachable!("tagged enum serializes to an object")
    };
    let name = properties.remove("name");
    properties.extend([
        ("$process_person_profile".into(), json!(false)),
        ("$geoip_disable".into(), json!(true)),
        ("$lib".into(), json!("prometeu")),
        ("analytics_version".into(), json!(2)),
        ("app_version".into(), json!(env!("CARGO_PKG_VERSION"))),
        ("os".into(), json!(std::env::consts::OS)),
        ("arch".into(), json!(std::env::consts::ARCH)),
    ]);
    json!({ "event": name, "distinct_id": id, "offset": delay.as_millis(), "properties": properties })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_after_preparation_prevents_dispatch_even_after_reenabling() {
        let root =
            std::env::temp_dir().join(format!("prometeu-analytics-{}", uuid::Uuid::new_v4()));
        let path = root.join("analytics-enabled.json");
        let delivery = Delivery::new(&path);
        delivery.save(&path, true).unwrap();
        let generation = delivery.consent.lock().unwrap().generation().unwrap();
        let prepared = vec![(generation, Event::AppOpened, Instant::now())];

        delivery.save(&path, false).unwrap();
        delivery.send_batch(prepared.clone(), "id", |_| panic!("sent after disabling"));
        delivery.save(&path, true).unwrap();
        delivery.send_batch(prepared, "id", |_| panic!("revived a stale batch"));

        let generation = delivery.consent.lock().unwrap().generation().unwrap();
        let mut sent = false;
        delivery.send_batch(
            vec![(generation, Event::AppOpened, Instant::now())],
            "id",
            |_| sent = true,
        );
        assert!(sent);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn in_flight_delivery_serializes_disabling_without_blocking_capture() {
        use std::sync::Arc;
        let root =
            std::env::temp_dir().join(format!("prometeu-analytics-{}", uuid::Uuid::new_v4()));
        let path = root.join("analytics-enabled.json");
        let delivery = Arc::new(Delivery::new(&path));
        delivery.save(&path, true).unwrap();
        let generation = delivery.consent.lock().unwrap().generation().unwrap();
        let (started, sending) = channel();
        let (release, wait) = channel();
        let worker_delivery = delivery.clone();
        let worker = std::thread::spawn(move || {
            worker_delivery.send_batch(
                vec![(generation, Event::AppOpened, Instant::now())],
                "id",
                |_| {
                    assert!(worker_delivery.sending.try_lock().is_err());
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                },
            );
        });
        sending.recv_timeout(Duration::from_secs(2)).unwrap();
        // App events still acquire consent immediately while HTTP is in flight.
        assert_eq!(
            delivery.consent.try_lock().unwrap().generation(),
            Some(generation)
        );

        let (saving, began_save) = channel();
        let (saved, completed) = channel();
        let saver_delivery = delivery.clone();
        let saver = std::thread::spawn(move || {
            saving.send(()).unwrap();
            saved
                .send(saver_delivery.save(&path, false).unwrap())
                .unwrap();
        });
        began_save.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            completed.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        worker.join().unwrap();
        assert!(!completed.recv_timeout(Duration::from_secs(2)).unwrap().0);
        saver.join().unwrap();
        delivery.send_batch(
            vec![(generation, Event::AppOpened, Instant::now())],
            "id",
            |_| panic!("sent after disabling"),
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn consent_defaults_off_and_survives_restart_without_using_legacy_identity() {
        let root =
            std::env::temp_dir().join(format!("prometeu-analytics-{}", uuid::Uuid::new_v4()));
        let path = root.join("analytics-enabled.json");
        paths::write_private(
            &root.join("analytics-id"),
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        assert_eq!(Consent::load(&path).generation(), None);
        assert!(!Consent::load(&path).decided);
        for invalid in ["", "null", "{}", "\"true\"", "broken"] {
            paths::write_private(&path, invalid).unwrap();
            assert_eq!(Consent::load(&path).generation(), None);
            assert!(!Consent::load(&path).decided);
        }
        let mut consent = Consent::load(&path);
        assert!(consent
            .batch(
                std::iter::once((0, Event::AppOpened, Instant::now())),
                "id",
                Instant::now()
            )
            .is_empty());
        assert!(consent.save(&path, true).unwrap());
        assert!(Consent::load(&path).enabled);
        assert!(Consent::load(&path).decided);
        assert!(!consent.save(&path, false).unwrap());
        assert!(consent.decided);
        assert!(Consent::load(&path).decided);
        assert_eq!(Consent::load(&path).generation(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn first_decline_and_existing_boolean_choices_suppress_the_prompt() {
        let root =
            std::env::temp_dir().join(format!("prometeu-analytics-{}", uuid::Uuid::new_v4()));
        let path = root.join("analytics-enabled.json");
        let mut consent = Consent::load(&path);
        assert!(!consent.decided);
        consent.save(&path, false).unwrap();
        assert!(consent.decided);
        for stored in ["false", "true"] {
            paths::write_private(&path, stored).unwrap();
            let loaded = Consent::load(&path);
            assert!(loaded.decided);
            assert_eq!(loaded.enabled, stored == "true");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disabling_discards_pending_events_even_after_enabling_again() {
        let root =
            std::env::temp_dir().join(format!("prometeu-analytics-{}", uuid::Uuid::new_v4()));
        let path = root.join("analytics-enabled.json");
        let mut consent = Consent::load(&path);
        let (sender, receiver) = channel();
        consent.save(&path, true).unwrap();
        let generation = consent.generation().unwrap();
        sender
            .send((generation, Event::AppOpened, Instant::now()))
            .unwrap();
        assert_eq!(
            consent
                .batch(receiver.try_iter(), "id", Instant::now())
                .len(),
            1
        );
        sender
            .send((generation, Event::AppOpened, Instant::now()))
            .unwrap();
        consent.save(&path, false).unwrap();
        assert!(consent
            .batch(receiver.try_iter(), "id", Instant::now())
            .is_empty());
        // An event racing with revocation may reach the queue late; it still cannot be sent.
        sender
            .send((generation, Event::AppOpened, Instant::now()))
            .unwrap();
        consent.save(&path, true).unwrap();
        sender
            .send((
                consent.generation().unwrap(),
                Event::ScreenViewed {
                    screen: Screen::Settings,
                },
                Instant::now(),
            ))
            .unwrap();
        let batch = consent.batch(receiver.try_iter(), "id", Instant::now());
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0]["event"], "screen_viewed");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_turning_sharing_on_reports_a_consent_grant() {
        let root =
            std::env::temp_dir().join(format!("prometeu-analytics-{}", uuid::Uuid::new_v4()));
        let path = root.join("analytics-enabled.json");
        let delivery = Delivery::new(&path);
        assert_eq!(delivery.save(&path, false).unwrap(), (false, false));
        assert_eq!(delivery.save(&path, true).unwrap(), (true, true));
        assert_eq!(delivery.save(&path, true).unwrap(), (true, false));
        assert_eq!(delivery.save(&path, false).unwrap(), (false, false));
        // A failed save keeps sharing off, so the next successful opt-in still counts.
        assert!(delivery.save(&root, true).is_err());
        assert_eq!(delivery.save(&path, true).unwrap(), (true, true));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_saves_preserve_consent_and_unreadable_storage_defaults_off() {
        let root =
            std::env::temp_dir().join(format!("prometeu-analytics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let mut consent = Consent::load(&root);
        assert_eq!(consent.generation(), None);
        assert!(consent.save(&root, true).is_err());
        assert!(!consent.decided);
        assert_eq!(consent.generation(), None);
        consent
            .save(&root.join("analytics-enabled.json"), true)
            .unwrap();
        let generation = consent.generation();
        assert!(consent.save(&root, false).is_err());
        assert!(consent.decided);
        assert_eq!(consent.generation(), generation);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn consent_filters_stale_events_without_changing_capture_offsets() {
        let consent = Consent {
            enabled: true,
            decided: true,
            generation: 2,
        };
        let captured = Instant::now();
        let later = captured + Duration::from_millis(2_000);
        let events = [
            (1, Event::AppOpened, captured),
            (2, Event::AttachmentAdded, captured),
            (2, Event::PreviewOpened, later),
        ];
        let batch = consent.batch(
            events.into_iter(),
            "id",
            captured + Duration::from_millis(5_125),
        );
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0]["event"], "attachment_added");
        assert_eq!(batch[0]["offset"], 5_125);
        assert_eq!(batch[1]["event"], "preview_opened");
        assert_eq!(batch[1]["offset"], 3_125);
        for event in batch {
            assert_eq!(event["properties"]["analytics_version"], 2);
        }
    }

    fn parse(value: Value) -> Result<Event, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn payload_carries_only_the_event_and_fixed_metadata() {
        let event = parse(json!({ "name": "conversation_created", "provider": "codex" })).unwrap();
        let body = payload(
            event,
            "00000000-0000-4000-8000-000000000000",
            Duration::ZERO,
        );
        assert_eq!(body["event"], "conversation_created");
        let mut keys: Vec<&str> = body["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "$geoip_disable",
                "$lib",
                "$process_person_profile",
                "analytics_version",
                "app_version",
                "arch",
                "os",
                "provider"
            ]
        );
        assert_eq!(body["properties"]["provider"], "codex");
        assert_eq!(body["properties"]["$process_person_profile"], false);
        assert_eq!(body["properties"]["analytics_version"], 2);
    }

    #[test]
    fn rejects_events_outside_the_vocabulary() {
        assert!(parse(json!({ "name": "unregistered_event" })).is_err());
        // Variants without fields ignore extras; the payload is rebuilt from the enum.
        let opened = parse(json!({ "name": "app_opened", "path": "/Users/someone/repo" })).unwrap();
        assert!(payload(opened, "id", Duration::ZERO)["properties"]
            .get("path")
            .is_none());
        let workspace = json!({ "name": "workspace_created", "provider": "claude", "is_worktree": true, "title": "secret" });
        assert!(parse(workspace).is_err());
    }

    #[test]
    fn clicks_carry_only_identifier_tokens() {
        let click = |target: Value| {
            parse(
                json!({ "name": "ui_clicked", "control": "menuitem", "target": target, "area": "deskView" }),
            )
        };
        let body = payload(click(json!("top.settings")).unwrap(), "id", Duration::ZERO);
        assert_eq!(body["event"], "ui_clicked");
        assert_eq!(body["properties"]["target"], "top.settings");
        assert!(click(Value::Null).is_err());
        for text in [
            "Fix the login bug",
            "/Users/someone/repo",
            "https://example.com",
            "",
            &"a".repeat(81),
        ] {
            assert!(click(json!(text)).is_err(), "{text}");
        }
        assert!(parse(
            json!({ "name": "ui_clicked", "control": "div", "target": null, "area": null })
        )
        .is_err());
    }

    #[test]
    fn unknown_provider_text_never_reaches_the_payload() {
        let event =
            parse(json!({ "name": "conversation_created", "provider": "my-private-model" }))
                .unwrap();
        assert_eq!(
            payload(event, "id", Duration::ZERO)["properties"]["provider"],
            "claude"
        );
    }

    #[test]
    fn feature_vocabulary_accepts_only_authored_properties() {
        for section in [
            "general",
            "agents",
            "resources",
            "actions",
            "work",
            "analytics",
        ] {
            let event =
                parse(json!({"name": "settings_section_viewed", "section": section})).unwrap();
            assert_eq!(
                payload(event, "id", Duration::ZERO)["properties"]["section"],
                section
            );
        }
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/product-analytics.json")).unwrap();
        for value in fixtures {
            let event = parse(value.clone()).unwrap();
            let body = payload(event, "id", Duration::ZERO);
            assert_eq!(body["event"], value["name"]);
            for (key, expected) in value.as_object().unwrap() {
                if key != "name" {
                    assert_eq!(&body["properties"][key], expected);
                }
            }
            let mut extra = value.clone();
            extra["private_content"] = json!("private prompt");
            if let Ok(event) = parse(extra) {
                assert!(payload(event, "id", Duration::ZERO)["properties"]
                    .get("private_content")
                    .is_none());
            }
            for key in value
                .as_object()
                .unwrap()
                .keys()
                .filter(|key| key.as_str() != "name" && key.as_str() != "provider")
            {
                let mut invalid = value.clone();
                invalid[key] = json!("private dynamic value");
                assert!(parse(invalid).is_err(), "{}:{key}", value["name"]);
            }
        }
        for retired in ["message_accepted", "turn_finished"] {
            assert!(parse(json!({"name":retired})).is_err());
        }
    }

    #[test]
    fn payload_offsets_capture_time_before_batch_delivery() {
        let body = payload(Event::AppOpened, "id", Duration::from_millis(5_125));
        assert_eq!(body["offset"], 5_125);
        assert!(body.get("timestamp").is_none());
        assert!(body["properties"].get("offset").is_none());
    }
}
