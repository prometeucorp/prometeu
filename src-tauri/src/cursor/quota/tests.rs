use super::*;
use serde_json::json;

fn now() -> u64 {
    rfc3339("2026-10-25T16:00:00Z").unwrap()
}

fn weekly() -> Value {
    // Shape recorded from the dashboard; synthetic percentage and dates, no identity.
    json!({"isUnified":true,"usagePercent":1.38,"hasAvailableUsage":true,
        "hasNonZeroIncludedLimit":true,"nextResetTimestampUtc":"2026-11-01T13:38:16.869Z"})
}

fn monthly() -> Value {
    json!({"limitType":"user","isUnlimited":false,
        "billingCycleStart":"2026-10-01T00:00:00Z","billingCycleEnd":"2026-11-01T00:00:00Z",
        "individualUsage":{"plan":{"enabled":true,"limit":10000,
            "totalPercentUsed":20,"autoPercentUsed":0,"apiPercentUsed":40}}})
}

#[test]
fn unified_weekly_quota_never_uses_the_stale_monthly_summary() {
    let windows = windows(&weekly(), now(), || panic!("must not query monthly usage")).unwrap();
    assert_eq!(
        windows,
        vec![Window {
            kind: "weekly".into(),
            pct: 1.38,
            resets: rfc3339("2026-11-01T13:38:16Z").unwrap(),
            scope: None,
            label: None
        }]
    );
    // Canonical quota windows retain the existing persisted/IPC shape.
    let encoded = serde_json::to_value(&windows).unwrap();
    assert_eq!(encoded[0].as_object().unwrap().len(), 3);
    assert_eq!(
        serde_json::from_value::<Vec<Window>>(encoded).unwrap(),
        windows
    );
}

#[test]
fn unavailable_or_invalid_weekly_data_never_becomes_monthly_zero() {
    for (field, value) in [
        ("isUnified", Value::Null),
        ("isUnified", json!("true")),
        ("usagePercent", Value::Null),
        ("usagePercent", json!(-1)),
        ("usagePercent", json!(101)),
        ("usagePercent", json!("1.38")),
        ("includedLimitZero", json!(true)),
        ("hasNonZeroIncludedLimit", json!(false)),
        ("nextResetTimestampUtc", json!("2026-10-24T00:00:00Z")),
        ("nextResetTimestampUtc", json!("bad")),
    ] {
        let mut value_with_error = weekly();
        value_with_error[field] = value;
        assert!(
            windows(&value_with_error, now(), || panic!("no monthly fallback")).is_none(),
            "{field}"
        );
    }
    let mut valid = weekly();
    valid["includedLimitZero"] = json!(false);
    valid["hasNonZeroIncludedLimit"] = json!(false);
    valid["usagePercent"] = json!(100);
    valid["hasAvailableUsage"] = json!(false);
    assert_eq!(windows(&valid, now(), || None).unwrap()[0].pct, 100.0);
    valid["usagePercent"] = json!(0);
    assert_eq!(windows(&valid, now(), || None).unwrap()[0].pct, 0.0);
}

#[test]
fn explicit_legacy_plan_uses_reported_monthly_pools_only() {
    let legacy = json!({"isUnified":false});
    let windows = windows(&legacy, now(), || Some(monthly())).unwrap();
    assert_eq!(
        windows.iter().map(|w| w.pct).collect::<Vec<_>>(),
        vec![20.0, 0.0, 40.0]
    );
    assert_eq!(windows[0].kind, format!("duration:{}", 31 * 86400));
    assert_eq!(windows[0].scope.as_deref(), Some("general"));
    assert_eq!(windows[1].label.as_deref(), Some("Cursor"));
    let mut absent = monthly();
    absent["individualUsage"]["plan"]
        .as_object_mut()
        .unwrap()
        .remove("apiPercentUsed");
    assert_eq!(monthly_windows(&absent, now()).unwrap().len(), 2);
    absent["individualUsage"]["plan"]["autoPercentUsed"] = json!("bad");
    assert!(monthly_windows(&absent, now()).is_none());
    for (key, value) in [
        ("limitType", json!("team")),
        ("isUnlimited", json!(true)),
        ("billingCycleEnd", json!("2026-10-01T00:00:00Z")),
    ] {
        let mut unsupported = monthly();
        unsupported[key] = value;
        assert!(monthly_windows(&unsupported, now()).is_none());
    }
}

fn token(claims: Value) -> String {
    format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

#[test]
fn cli_cookie_accepts_only_unexpired_tokens_with_safe_identity() {
    let valid = token(json!({"sub":"auth0|user_123","exp":now()+3600}));
    assert_eq!(
        session_cookie(&valid, now()).unwrap(),
        format!("WorkosCursorSessionToken=user_123%3A%3A{valid}")
    );
    for claims in [
        json!({"sub":"user_123","exp":now()}),
        json!({"sub":"user_123","exp":now()+60}),
        json!({"sub":"user_123"}),
        json!({"sub":"auth0|","exp":now()+3600}),
        json!({"sub":"user;cookie=bad","exp":now()+3600}),
    ] {
        assert!(session_cookie(&token(claims), now()).is_none());
    }
    for value in [
        "",
        "not-a-jwt",
        "a.b.c.d",
        "a.b.c\r\nCookie:bad",
        "a.!!!!.c",
    ] {
        assert!(session_cookie(value, now()).is_none());
    }
}

#[test]
fn file_store_path_matches_cursor_platform_rules() {
    let home = std::path::Path::new("/home/test");
    if cfg!(target_os = "macos") {
        assert_eq!(
            auth_path(home, None).unwrap(),
            home.join(".cursor/auth.json")
        );
    } else {
        assert_eq!(
            auth_path(home, None).unwrap(),
            home.join(".config/cursor/auth.json")
        );
        assert_eq!(
            auth_path(home, Some("/config".into())).unwrap(),
            PathBuf::from("/config/cursor/auth.json")
        );
        assert!(auth_path(home, Some("relative".into())).is_none());
    }
}

#[test]
#[ignore = "reads the signed-in Cursor account quota over the network; no inference"]
fn live_cursor_quota() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let windows = quota().expect("the current Cursor login must expose a supported quota");
    assert!(!windows.is_empty());
    let current = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(windows
        .iter()
        .all(|w| w.resets > current && (0.0..=100.0).contains(&w.pct)));
}
