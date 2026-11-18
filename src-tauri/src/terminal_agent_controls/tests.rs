use super::*;
use serde_json::{json, Value};

const CLAUDE: &str = include_str!("../native_ui_readiness/fixtures/claude-fresh-busy.json");
const CURSOR: &str = include_str!("../native_ui_readiness/fixtures/cursor-fresh-busy.json");
const CODEX: &str = include_str!("fixtures/codex-busy.json");
const ANTIGRAVITY: &str = include_str!("fixtures/antigravity-busy.json");
const ANTIGRAVITY_1_5_10: &str = include_str!("fixtures/antigravity-1.5.10-busy.json");

pub(crate) fn snapshot(body: &str) -> (String, Snapshot) {
    let view: Value = serde_json::from_str(body).unwrap();
    let state = serde_json::from_value(json!({
        "v": 1, "session": "fixture", "generation": "fixture-generation", "seq": 100,
        "cols": view["cols"], "rows": view["rows"], "scrollback": 1000,
        "active": "normal", "viewportLines": view["lines"],
        "viewportCharacterColumns": view["lines"].as_array().unwrap().iter().map(|line|
            line.as_str().unwrap().chars().enumerate().map(|(col, _)| col as u16).collect::<Vec<_>>()
        ).collect::<Vec<_>>(),
        "viewportStyles": vec![vec![json!({"start":0,"end":view["cols"],"dim":false,"inverse":false,
            "foreground":{"mode":"default","value":-1},"background":{"mode":"default","value":-1}})]; view["rows"].as_u64().unwrap() as usize],
        "appearance": {"foreground":"#d8c2b3", "background":"#141110", "cursor":"#d8c2b3"},
        "normal": {"ansi":"", "cursor":view["cursor"], "scrollTop":0, "scrollBottom":39,
            "savedCursor":{"x":0,"y":0,"origin":false,"attributes":""},"wrapCell":null},
        "alternate": null, "modes": {"applicationCursorKeysMode":false,"applicationKeypadMode":false,
            "bracketedPasteMode":view["bracketedPaste"],"insertMode":false,"originMode":false,
            "reverseWraparoundMode":false,"sendFocusMode":false,"wraparoundMode":true,
            "mouseTrackingMode":"none","mouseEncoding":"DEFAULT","cursorHidden":view["cursorHidden"],
            "cursorStyle":"block","cursorBlink":false},
        "rehydrate":"","title":view["title"],"titleTruncated":false,"links":[],
        "pendingEscape":"","pendingUtf8":""
    })).unwrap();
    (view["version"].as_str().unwrap().into(), state)
}

#[test]
fn cursor_working_observation_requires_its_current_complete_busy_composer() {
    for (body, working) in [
        (CURSOR, true),
        (include_str!("fixtures/cursor-scrolled-busy.json"), true),
        (include_str!("fixtures/cursor-interrupted.json"), false),
        (
            include_str!("../native_ui_readiness/fixtures/cursor-fresh-rest.json"),
            false,
        ),
        (
            include_str!("../native_ui_readiness/fixtures/cursor-fresh-trust.json"),
            false,
        ),
    ] {
        let (version, state) = snapshot(body);
        let controls = NativeControls::verified(ProviderId::Cursor, &version).unwrap();
        assert_eq!(controls.observes_working(&state), working);
    }
    let (version, state) = snapshot(CURSOR);
    let controls = NativeControls::verified(ProviderId::Cursor, &version).unwrap();
    let mut incomplete = state.clone();
    incomplete.pending_escape = "\x1b[".into();
    assert!(!controls.observes_working(&incomplete));
    let mut stale = state;
    stale.normal.cursor.y = 0;
    assert!(!controls.observes_working(&stale));
    let (version, state) = snapshot(CLAUDE);
    assert!(!NativeControls::verified(ProviderId::Claude, &version)
        .unwrap()
        .observes_working(&state));
}

#[test]
fn current_native_busy_footers_choose_different_interrupt_keys() {
    for (provider, body, expected) in [
        (ProviderId::Claude, CLAUDE, b"\x1b".as_slice()),
        (
            ProviderId::Claude,
            include_str!("fixtures/claude-busy.json"),
            b"\x1b".as_slice(),
        ),
        (ProviderId::Cursor, CURSOR, b"\x03".as_slice()),
        (
            ProviderId::Cursor,
            include_str!("fixtures/cursor-busy.json"),
            b"\x03".as_slice(),
        ),
        (
            ProviderId::Cursor,
            include_str!("fixtures/cursor-scrolled-busy.json"),
            b"\x03".as_slice(),
        ),
        (ProviderId::Codex, CODEX, b"\x1b".as_slice()),
        (ProviderId::Antigravity, ANTIGRAVITY, b"\x1b".as_slice()),
        (
            ProviderId::Antigravity,
            ANTIGRAVITY_1_5_10,
            b"\x1b".as_slice(),
        ),
    ] {
        let (version, state) = snapshot(body);
        let controls = NativeControls::verified(provider, &version).unwrap();
        assert_eq!(controls.interrupt(&state, true), Some(expected));
        assert_eq!(controls.decision(&state, false), Err(Unavailable::Inactive));
    }
}

#[test]
fn claude_compact_busy_control_follows_the_current_cursor_and_footer() {
    let (version, original) = snapshot(include_str!("fixtures/claude-busy.json"));
    let control = NativeControls::verified(ProviderId::Claude, &version).unwrap();
    assert!(original.modes.cursor_hidden);
    assert_eq!(control.interrupt(&original, true), Some(b"\x1b".as_slice()));
    let mut stale_cursor = original.clone();
    stale_cursor.normal.cursor.y += 2;
    assert_eq!(control.interrupt(&stale_cursor, true), None);
    let mut modal = original;
    modal.viewport_lines[30] = "Select an option".into();
    assert_eq!(control.interrupt(&modal, true), None);
}

#[test]
fn cursor_scrolled_history_uses_the_current_footer_not_an_offscreen_banner() {
    let (version, original) = snapshot(include_str!("fixtures/cursor-scrolled-busy.json"));
    assert!(!original
        .viewport_lines
        .iter()
        .any(|line| line.trim() == "Cursor Agent"));
    let control = NativeControls::verified(ProviderId::Cursor, &version).unwrap();
    assert_eq!(control.interrupt(&original, true), Some(b"\x03".as_slice()));
    let mut stale = original.clone();
    stale.normal.cursor.y = 0;
    assert_eq!(control.interrupt(&stale, true), None);
    let mut missing_footer = original.clone();
    missing_footer.viewport_lines[37].clear();
    assert_eq!(control.interrupt(&missing_footer, true), None);
    let mut unaligned_hint = original;
    unaligned_hint.viewport_lines[34] = "  → Add a follow-up  ctrl+c to stop".into();
    assert_eq!(control.interrupt(&unaligned_hint, true), None);
}

#[test]
fn rest_and_trust_captures_cannot_supply_an_interrupt_binding() {
    for (provider, body) in [
        (
            ProviderId::Claude,
            include_str!("../native_ui_readiness/fixtures/claude-fresh-initial-ready.json"),
        ),
        (
            ProviderId::Claude,
            include_str!("../native_ui_readiness/fixtures/claude-fresh-trust.json"),
        ),
        (
            ProviderId::Claude,
            include_str!("fixtures/claude-interrupted.json"),
        ),
        (
            ProviderId::Cursor,
            include_str!("../native_ui_readiness/fixtures/cursor-fresh-rest.json"),
        ),
        (
            ProviderId::Cursor,
            include_str!("../native_ui_readiness/fixtures/cursor-fresh-trust.json"),
        ),
        (
            ProviderId::Cursor,
            include_str!("fixtures/cursor-interrupted.json"),
        ),
        (
            ProviderId::Codex,
            include_str!("fixtures/codex-interrupted.json"),
        ),
        (
            ProviderId::Codex,
            include_str!("fixtures/codex-hook-review.json"),
        ),
        (
            ProviderId::Codex,
            include_str!("fixtures/codex-mcp-startup.json"),
        ),
        (
            ProviderId::Codex,
            include_str!("fixtures/codex-mcp-cancelled.json"),
        ),
        (
            ProviderId::Antigravity,
            include_str!("fixtures/antigravity-interrupted.json"),
        ),
        (
            ProviderId::Antigravity,
            include_str!("fixtures/antigravity-1.5.10-interrupted.json"),
        ),
    ] {
        let (version, state) = snapshot(body);
        assert_eq!(
            NativeControls::verified(provider, &version)
                .unwrap()
                .interrupt(&state, true),
            None
        );
    }
}

#[test]
fn customized_hints_partial_paints_and_transcript_mentions_are_rejected() {
    for (provider, body, original, replacement) in [
        (
            ProviderId::Claude,
            CLAUDE,
            "esc to interrupt",
            "ctrl+x to interrupt",
        ),
        (ProviderId::Cursor, CURSOR, "ctrl+c to stop", "esc to stop"),
        (
            ProviderId::Codex,
            CODEX,
            "esc to interrupt",
            "ctrl+x to interrupt",
        ),
        (
            ProviderId::Antigravity,
            ANTIGRAVITY,
            "esc to cancel",
            "ctrl+x to cancel",
        ),
        (
            ProviderId::Antigravity,
            ANTIGRAVITY_1_5_10,
            "esc to cancel",
            "ctrl+x to cancel",
        ),
    ] {
        let (version, mut state) = snapshot(body);
        let controls = NativeControls::verified(provider, &version).unwrap();
        for line in &mut state.viewport_lines {
            *line = line.replace(original, replacement);
        }
        assert_eq!(
            controls.decision(&state, true),
            Err(Unavailable::ConflictingBinding)
        );
        state.viewport_lines[4] = original.into();
        assert_eq!(controls.interrupt(&state, true), None);

        let (_, mut state) = snapshot(body);
        state.pending_escape = "\x1b[".into();
        assert_eq!(
            controls.decision(&state, true),
            Err(Unavailable::IncompleteSnapshot)
        );
        state.pending_escape.clear();
        state.pending_utf8 = "ww==".into();
        assert_eq!(
            controls.decision(&state, true),
            Err(Unavailable::IncompleteSnapshot)
        );
        state.pending_utf8.clear();
        state.viewport_lines.pop();
        assert_eq!(
            controls.decision(&state, true),
            Err(Unavailable::IncompleteSnapshot)
        );
    }
}

#[test]
fn documented_defaults_and_unknown_versions_do_not_authorize_unobserved_bindings() {
    let (_, state) = snapshot(CLAUDE);
    for (provider, version) in [
        (ProviderId::Codex, "codex-cli 0.169.0"),
        (ProviderId::Antigravity, "1.5.9"),
    ] {
        assert_eq!(
            NativeControls::verified(provider, version)
                .unwrap()
                .decision(&state, true),
            Err(Unavailable::UnsupportedLayout)
        );
    }
    for provider in [
        ProviderId::Claude,
        ProviderId::Codex,
        ProviderId::Antigravity,
        ProviderId::Cursor,
    ] {
        assert!(matches!(
            NativeControls::verified(provider, "unknown-build"),
            Err(Unavailable::UnknownVersion)
        ));
    }
}

#[test]
fn evidence_versions_match_the_profile_and_preserve_provenance() {
    let profiles: Value = serde_json::from_str(include_str!("evidence.json")).unwrap();
    for (name, provider) in [
        ("claude", ProviderId::Claude),
        ("codex", ProviderId::Codex),
        ("antigravity", ProviderId::Antigravity),
        ("cursor", ProviderId::Cursor),
    ] {
        let profile = &profiles[name];
        assert!(NativeControls::verified(provider, profile["version"].as_str().unwrap()).is_ok());
        assert!(!profile["evidence"].as_array().unwrap().is_empty());
    }
}

#[test]
fn native_stop_results_are_model_turns_not_startup_mcp_cancellation() {
    for (provider, source) in [
        (
            ProviderId::Claude,
            include_str!("fixtures/claude-result.json"),
        ),
        (
            ProviderId::Codex,
            include_str!("fixtures/codex-result.json"),
        ),
        (
            ProviderId::Antigravity,
            include_str!("fixtures/antigravity-result.json"),
        ),
        (
            ProviderId::Antigravity,
            include_str!("fixtures/antigravity-1.5.10-result.json"),
        ),
    ] {
        let result: Value = serde_json::from_str(source).unwrap();
        assert!(NativeControls::verified(provider, result["version"].as_str().unwrap()).is_ok());
        assert_eq!(result["promptObservedByHook"], true);
        assert_eq!(result["interrupt"], "escape");
        assert_eq!(result["interruptCount"], 1);
        assert_eq!(result["explicitPostInterruptIdleOrDone"], true);
        assert_eq!(result["processRemainsAlive"], true);
        assert!(result["nativeRootStates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|state| state == "Working"));
    }
}

#[test]
fn cursor_stop_is_one_ctrl_c_after_model_output_and_preserves_native_abort_evidence() {
    let result: Value = serde_json::from_str(include_str!("fixtures/cursor-result.json")).unwrap();
    assert!(
        NativeControls::verified(ProviderId::Cursor, result["version"].as_str().unwrap()).is_ok()
    );
    assert_eq!(result["interrupt"], "ctrl-c");
    assert_eq!(result["interruptCount"], 1);
    assert_eq!(result["promptObservedInNativeUi"], true);
    assert_eq!(result["modelOutputObservedBeforeInterrupt"], true);
    assert_eq!(result["explicitPostInterruptAbort"], true);
    assert!(result["nativeRootStopReasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason == "aborted"));
    assert_eq!(result["explicitPostInterruptIdleOrDone"], false);
    assert_eq!(result["nativeRootStates"], json!([]));
    assert_eq!(result["processRemainsAlive"], true);
    assert_eq!(result["interruptedPromptRestoredAsDraft"], true);
    assert_eq!(result["processReaped"], true);
}
