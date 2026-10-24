//! Anonymous product analytics sent to PostHog (ADR 0093). Only the closed `Event` vocabulary
//! crosses the network: fields are enums, booleans or identifier tokens we author (i18n keys and
//! static element IDs), never free text, paths, names or content. Builds without
//! `PROMETEU_POSTHOG_KEY` (development, tests, forks) send nothing.

use crate::paths;
use prometeu_core::board::ProviderId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
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
    MessageAccepted {
        provider: ProviderId,
        is_action: bool,
        is_delegated: bool,
    },
    TurnFinished {
        provider: ProviderId,
        outcome: TurnOutcome,
        is_action: bool,
        is_delegated: bool,
        has_agent_activity: bool,
    },
    UiClicked {
        control: Control,
        target: Option<Token>,
        area: Option<Token>,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
}

/// Conversation context, not the identity of whoever submitted its latest message.
#[derive(Clone, Copy, Default)]
pub(crate) struct MessageContext {
    pub is_action: bool,
    pub is_delegated: bool,
}

/// Observe only live commands and canonical output. Provider turns and accepted messages are
/// independent counts. The adapters own terminal deduplication: overlapping input can finish
/// without intervening output, so activity cannot identify duplicate terminals here.
pub(crate) struct ConversationCapture {
    provider: ProviderId,
    context: Option<MessageContext>,
    has_agent_activity: bool,
}

impl ConversationCapture {
    pub fn new(provider: ProviderId) -> Self {
        Self {
            provider,
            context: None,
            has_agent_activity: false,
        }
    }

    pub fn accepted(&mut self, context: MessageContext) -> Event {
        self.context = Some(context);
        Event::MessageAccepted {
            provider: self.provider,
            is_action: context.is_action,
            is_delegated: context.is_delegated,
        }
    }

    pub fn observe(&mut self, event: &Value) -> Option<Event> {
        let context = self.context?;
        match event["type"].as_str() {
            Some("assistant.started" | "assistant.block") => {
                self.has_agent_activity = true;
                None
            }
            Some("turn.completed") => {
                let outcome = match event["outcome"].as_str()? {
                    "ok" => TurnOutcome::Completed,
                    "error" => TurnOutcome::Failed,
                    "interrupted" => TurnOutcome::Interrupted,
                    _ => return None,
                };
                Some(Event::TurnFinished {
                    provider: self.provider,
                    outcome,
                    is_action: context.is_action,
                    is_delegated: context.is_delegated,
                    has_agent_activity: std::mem::take(&mut self.has_agent_activity),
                })
            }
            _ => None,
        }
    }
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

static QUEUE: OnceLock<Option<Sender<(Event, Instant)>>> = OnceLock::new();

/// Never blocks or fails the caller; delivery problems drop the event.
pub fn track(event: Event) {
    let occurred = Instant::now();
    // CI defines the variable as empty when the repository variable is missing.
    let queue = QUEUE.get_or_init(|| {
        option_env!("PROMETEU_POSTHOG_KEY")
            .filter(|key| !key.trim().is_empty())
            .map(start)
    });
    if let Some(queue) = queue {
        let _ = queue.send((event, occurred));
    }
}

#[tauri::command]
pub fn analytics_track(event: Event) {
    track(event);
}

fn start(key: &'static str) -> Sender<(Event, Instant)> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || deliver(key, receiver));
    sender
}

// ponytail: no retry or offline queue; a failed batch, or one still waiting when the app quits,
// is lost. Add a bounded on-disk queue if coverage gaps matter.
fn deliver(key: &str, events: Receiver<(Event, Instant)>) {
    let id = install_id();
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return;
    };
    while let Ok(first) = events.recv() {
        std::thread::sleep(BATCH_WINDOW);
        let batch: Vec<Value> = std::iter::once(first)
            .chain(events.try_iter())
            .map(|(event, occurred)| payload(event, &id, occurred.elapsed()))
            .collect();
        let _ = client
            .post(format!("{HOST}/batch/"))
            .json(&json!({ "api_key": key, "batch": batch }))
            .send();
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
        assert!(parse(json!({ "name": "file_opened" })).is_err());
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
        assert!(click(Value::Null).is_ok());
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
    fn capture_counts_writes_and_observed_turns_without_inspecting_content() {
        let mut capture = ConversationCapture::new(ProviderId::Codex);
        let terminal = json!({"type":"turn.completed","outcome":"ok","message":"private result"});
        let activity = json!({"type":"assistant.block","block":{"text":"private output"}});
        assert!(capture.observe(&activity).is_none());
        assert!(capture.observe(&terminal).is_none());

        let context = MessageContext {
            is_action: true,
            is_delegated: true,
        };
        assert_eq!(
            capture.accepted(context),
            Event::MessageAccepted {
                provider: ProviderId::Codex,
                is_action: true,
                is_delegated: true,
            }
        );
        assert!(capture
            .observe(&json!({"type":"user.message","text":"private echo"}))
            .is_none());
        assert!(capture
            .observe(&json!({"type":"telemetry.usage","usage":{"input_tokens":123}}))
            .is_none());
        assert_eq!(
            capture.observe(&terminal),
            Some(Event::TurnFinished {
                provider: ProviderId::Codex,
                outcome: TurnOutcome::Completed,
                is_action: true,
                is_delegated: true,
                has_agent_activity: false,
            })
        );
        for terminals in [
            [
                ("ok", TurnOutcome::Completed),
                ("error", TurnOutcome::Failed),
            ],
            [
                ("error", TurnOutcome::Failed),
                ("ok", TurnOutcome::Completed),
            ],
        ] {
            capture.accepted(context);
            capture.observe(&activity);
            capture.accepted(context);
            for (index, (reported, outcome)) in terminals.into_iter().enumerate() {
                assert_eq!(
                    capture.observe(&json!({"type":"turn.completed","outcome":reported})),
                    Some(Event::TurnFinished {
                        provider: ProviderId::Codex,
                        outcome,
                        is_action: true,
                        is_delegated: true,
                        has_agent_activity: index == 0,
                    })
                );
            }
        }
        capture.observe(&json!({"type":"assistant.started"}));
        capture.accepted(context);
        assert_eq!(
            capture.observe(&terminal),
            Some(Event::TurnFinished {
                provider: ProviderId::Codex,
                outcome: TurnOutcome::Completed,
                is_action: true,
                is_delegated: true,
                has_agent_activity: true,
            })
        );
    }

    #[test]
    fn capture_requires_a_known_terminal_and_does_not_guess_process_exit() {
        let mut capture = ConversationCapture::new(ProviderId::Claude);
        for (reported, expected) in [
            ("error", TurnOutcome::Failed),
            ("interrupted", TurnOutcome::Interrupted),
        ] {
            capture.accepted(MessageContext::default());
            assert!(capture
                .observe(&json!({"type":"session.state","state":"stopped"}))
                .is_none());
            assert!(capture
                .observe(&json!({"type":"turn.completed","outcome":"unknown"}))
                .is_none());
            assert_eq!(
                capture.observe(&json!({"type":"turn.completed","outcome":reported})),
                Some(Event::TurnFinished {
                    provider: ProviderId::Claude,
                    outcome: expected,
                    is_action: false,
                    is_delegated: false,
                    has_agent_activity: false,
                })
            );
        }
    }

    #[test]
    fn conversation_payloads_allow_only_closed_context_and_outcomes() {
        let message = json!({"name":"message_accepted","provider":"codex","is_action":false,"is_delegated":true});
        let terminal = json!({"name":"turn_finished","provider":"codex","outcome":"completed","is_action":false,"is_delegated":true,"has_agent_activity":true});
        for value in [message, terminal] {
            let event = parse(value.clone()).unwrap();
            let mut properties = payload(event, "id", Duration::ZERO)["properties"].clone();
            for key in [
                "$geoip_disable",
                "$lib",
                "$process_person_profile",
                "analytics_version",
                "app_version",
                "arch",
                "os",
            ] {
                properties.as_object_mut().unwrap().remove(key);
            }
            let mut expected = value.clone();
            expected.as_object_mut().unwrap().remove("name");
            assert_eq!(properties, expected);
            let mut extra = value;
            extra["text"] = json!("private prompt");
            assert!(parse(extra).is_err());
        }
        assert!(parse(json!({"name":"turn_finished","provider":"codex","outcome":"private error","is_action":false,"is_delegated":false,"has_agent_activity":false})).is_err());
    }

    #[test]
    fn payload_offsets_capture_time_before_batch_delivery() {
        let body = payload(Event::AppOpened, "id", Duration::from_millis(5_125));
        assert_eq!(body["offset"], 5_125);
        assert!(body.get("timestamp").is_none());
        assert!(body["properties"].get("offset").is_none());
    }
}
