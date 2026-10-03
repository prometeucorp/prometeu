use super::*;
use crate::process::ProcessIdentity;
use serde_json::json;

#[derive(Default)]
struct Conversation(ProcessIdentity);
impl HostedConversation for Conversation {
    fn identity(&self) -> &ProcessIdentity {
        &self.0
    }
    fn alive(&self) -> bool {
        true
    }
    fn waits_for_turn(&self) -> bool {
        false
    }
    fn working(&self) -> bool {
        false
    }
    fn changing_account(&self) -> Result<bool, String> {
        Ok(false)
    }
    fn retire(&mut self) {}
}
struct Fixture {
    board: Mutex<Board>,
    host: SessionHost<Conversation>,
    events: Mutex<Vec<&'static str>>,
    fail_prepare: bool,
    fail_spawn: bool,
    resumed: bool,
    permission: Option<Permission>,
}
impl Fixture {
    fn new() -> Self {
        Self {
            board: Mutex::new(serde_json::from_value(json!({"stages":[],"workspaces":[{
                "id":"w","title":"workspace","repo":"/repo","repo_name":"repo","branch":"main","worktree":"/work tree","stage":"review",
                "tabs":[{"id":"s","title":"session","status":"desligada","note":"old notice","agent_session":"provider-id","pending_prompt":"queued"}]
            }]})).unwrap()),
            host: SessionHost::default(), events: Mutex::new(vec![]),
            fail_prepare: false, fail_spawn: false, resumed: true, permission: Some(Permission::Auto),
        }
    }
    fn service(&self) -> LaunchService<'_, Conversation> {
        LaunchService {
            board: &self.board,
            host: &self.host,
            preparation: self,
            launcher: self,
            effects: self,
        }
    }
    fn effect(&self, event: &'static str) {
        assert!(
            self.board.try_lock().is_ok(),
            "native effects must release the board"
        );
        assert!(
            self.host.conversations.try_lock().is_ok(),
            "native effects must release processes"
        );
        lock(&self.events).push(event);
    }
    fn existing(&self) -> ProcessIdentity {
        let chat = Conversation::default();
        let identity = chat.0.clone();
        lock(&self.host.conversations).insert("s".into(), chat);
        self.host.mark_ready("s");
        lock(&self.host.work).entry("s".into()).or_default();
        identity
    }
}
impl ResumePreparation for Fixture {
    fn prepare(&self, session: &str, snapshot: ResumeSnapshot) -> Result<PreparedResume, String> {
        self.effect("prepare");
        assert_eq!(snapshot.workspace.stage, "review");
        if self.fail_prepare {
            return Err("invalid worktree".into());
        }
        let previous = snapshot
            .workspace
            .tabs
            .iter()
            .find(|tab| tab.id == session)
            .unwrap()
            .agent_session
            .clone();
        Ok(PreparedResume {
            request: LaunchRequest {
                session: session.into(),
                workspace: snapshot.workspace.id,
                worktree: snapshot.workspace.worktree,
                settings: Launch {
                    model: "selected".into(),
                    permission: Some(Permission::Auto),
                    ..Default::default()
                },
                mode: StartMode::Resume {
                    provider_session: previous,
                },
            },
            warning: Some("skill removed".into()),
        })
    }
}
impl ConversationLauncher<Conversation> for Fixture {
    fn launch(&self, request: &LaunchRequest) -> Result<Launched<Conversation>, String> {
        self.effect("launch");
        assert_eq!(request.worktree, "/work tree");
        if let StartMode::Resume { provider_session } = &request.mode {
            assert_eq!(provider_session.as_deref(), Some("provider-id"));
            assert_eq!(request.settings.model, "selected");
            assert!(request.settings.permission == self.permission);
            assert!(!self.host.is_ready(&request.session));
            assert!(lock(&self.host.work).is_empty());
            assert!(!lock(&self.host.conversations).contains_key(&request.session));
        }
        if self.fail_spawn {
            return Err("spawn failed".into());
        }
        Ok(Launched {
            conversation: Conversation::default(),
            resumed: self.resumed,
        })
    }
}
impl LaunchEffects<Conversation> for Fixture {
    fn revoke(&self, _: &str) {
        self.effect("revoke");
    }
    fn stopped(&self, _: &str) {
        self.effect("stopped");
    }
    fn warning(&self, _: &Conversation, detail: &str) {
        self.effect("warning");
        assert_eq!(detail, "skill removed");
        assert!(lock(&self.host.conversations).is_empty());
    }
    fn publish(&self) {
        self.effect("publish");
        assert!(lock(&self.host.conversations).contains_key("s"));
        let mut board = lock(&self.board);
        let tab = board.tab_mut("s").unwrap();
        assert!(tab.status == Status::Pronta);
        assert!(tab.note.is_none());
        assert_eq!(tab.pending_prompt.as_deref(), Some("queued"));
    }
    fn ready(&self, session: &str) {
        self.effect("ready");
        assert_eq!(lock(&self.events).iter().rev().nth(1), Some(&"publish"));
        self.host.mark_ready(session);
    }
}

#[test]
fn resume_prepares_outside_locks_and_installs_before_publication_and_readiness() {
    for resumed in [false, true] {
        let fixture = Fixture {
            resumed,
            ..Fixture::new()
        };
        let old = fixture.existing();
        assert_eq!(fixture.service().resume("s"), Ok(resumed));
        assert_eq!(
            *lock(&fixture.events),
            ["prepare", "revoke", "stopped", "launch", "warning", "publish", "ready"]
        );
        assert_ne!(lock(&fixture.host.conversations)["s"].0, old);
        assert!(fixture.host.is_ready("s"));
        assert_eq!(
            lock(&fixture.board).workspace_of("s").unwrap().stage,
            "review"
        );
    }
}

#[test]
fn invalid_preparation_preserves_the_previous_process_and_pending_input() {
    let fixture = Fixture {
        fail_prepare: true,
        ..Fixture::new()
    };
    let old = fixture.existing();
    assert_eq!(
        fixture.service().resume("s"),
        Err("invalid worktree".into())
    );
    assert_eq!(lock(&fixture.host.conversations)["s"].0, old);
    assert!(fixture.host.is_ready("s"));
    assert_eq!(*lock(&fixture.events), ["prepare"]);
    assert_eq!(
        lock(&fixture.board)
            .tab_mut("s")
            .unwrap()
            .pending_prompt
            .as_deref(),
        Some("queued")
    );
}

#[test]
fn failed_spawn_retires_the_old_process_without_publishing_success_or_losing_the_queue() {
    let fixture = Fixture {
        fail_spawn: true,
        ..Fixture::new()
    };
    fixture.existing();
    assert_eq!(fixture.service().resume("s"), Err("spawn failed".into()));
    assert!(lock(&fixture.host.conversations).is_empty());
    assert!(!fixture.host.is_ready("s"));
    assert_eq!(
        *lock(&fixture.events),
        ["prepare", "revoke", "stopped", "launch"]
    );
    let mut board = lock(&fixture.board);
    let tab = board.tab_mut("s").unwrap();
    assert_eq!(tab.pending_prompt.as_deref(), Some("queued"));
    assert_eq!(tab.note.as_deref(), Some("old notice"));
}

#[test]
fn missing_session_has_no_effects_and_new_tabs_do_not_publish_readiness_early() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.service().resume("missing"),
        Err(code("err.session.noTab"))
    );
    assert!(lock(&fixture.events).is_empty());
    fixture
        .service()
        .start(
            &LaunchRequest {
                session: "new".into(),
                workspace: "w".into(),
                worktree: "/work tree".into(),
                settings: Launch::default(),
                mode: StartMode::Fresh,
            },
            None,
        )
        .unwrap();
    assert_eq!(*lock(&fixture.events), ["launch"]);
    assert!(lock(&fixture.host.conversations).contains_key("new"));
    assert!(!fixture.host.is_ready("new"));
    assert!(lock(&fixture.board).tab_mut("new").is_none());
}

#[test]
fn launch_settings_keep_legacy_defaults_and_explicit_empty_tool_selections() {
    let inherited: Launch = serde_json::from_value(json!({"agent":"", "model":"model"})).unwrap();
    assert_eq!(inherited.agent, ProviderId::Claude);
    assert!(inherited.mcp.is_none());
    assert!(inherited.plugin_packages().is_none());
    assert!(!inherited.plan);
    let explicit: Launch =
        serde_json::from_value(json!({"agent":"codex", "mcp":[], "plugins":[], "skills":null}))
            .unwrap();
    assert_eq!(explicit.agent, ProviderId::Codex);
    assert_eq!(explicit.mcp, Some(vec![]));
    assert_eq!(explicit.plugin_packages(), Some(vec![]));
    let selected: Launch = serde_json::from_value(
        json!({"plugins":["plugin"], "skills":["skill-a"], "permission":"ask"}),
    )
    .unwrap();
    assert_eq!(
        selected.plugin_packages(),
        Some(vec!["plugin".into(), "skill-a".into()])
    );
}

#[test]
fn delegation_permission_overrides_prepared_settings_including_an_explicit_unset() {
    for permission in [None, Some(Permission::Ask)] {
        let fixture = Fixture {
            permission,
            ..Fixture::new()
        };
        lock(&fixture.board).delegations.push(
            serde_json::from_value(json!({
                "id":"s", "owner":"owner", "workspace":"w", "task":"task",
                "request_key":"key", "request_hash":"hash", "repository_heads":{},
                "permission":permission, "executions":[], "background":null, "requests":[]
            }))
            .unwrap(),
        );
        fixture.service().resume("s").unwrap();
    }
}
