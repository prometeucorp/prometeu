//! Track account quota usage from live CLI events and background polling. Claude emits rate-limit
//! events; Codex emits updates and answers an initial rate-limit read. Poll inactive profiles with
//! their own credentials, retaining the previous reading on failure. Attribute each event to the
//! process's captured account even after selection changes. Cache all accounts and let presentation
//! choose which to display.

use crate::lock::lock;
use crate::{accounts, background, paths, usage_scheduler};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Quota window utilization from 0 to 100 and its reset time.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Window {
    /// Session, weekly, and model-specific window identities are translated by the frontend.
    pub kind: String,
    pub pct: f64,
    /// Unix reset timestamp in seconds for relative-time display.
    pub resets: u64,
    /// A stable bucket identity separates general and model/feature quotas without exposing
    /// provider payloads. Missing scope represents the legacy single-bucket format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Optional provider-supplied bucket label is escaped for display. Scope remains the identity
    /// used for merging sparse updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// An account's latest known usage and update time.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Agent {
    pub windows: Vec<Window>,
    /// Record when usage actually changes, in Unix seconds. Identical readings do not refresh the
    /// display timestamp or rewrite disk state.
    pub at: u64,
}

/// Key by account; claude and codex retain the original CLI-account entries.
pub type Usage = BTreeMap<String, Agent>;

fn state() -> &'static Mutex<Usage> {
    static STATE: OnceLock<Mutex<Usage>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(read()))
}

/// Return cached usage at startup before any live agent response.
#[tauri::command]
pub fn usage() -> Usage {
    lock(state()).clone()
}

pub fn forget(app: &AppHandle, account: &str) {
    let service = app.state::<Service>();
    lock(&service.schedule).forget(account);
    service.wake();
    let mut all = lock(state());
    all.remove(account);
    write(&all);
    let _ = app.emit("usage", all.clone());
}

/// Read rate_limit_info from a Claude rate-limit event.
pub fn claude(app: &AppHandle, account: &str, revision: u64, info: &Value) {
    note(app, account, claude_windows(info), revision);
}

/// Read rateLimits from Codex account rate-limit responses and notifications.
pub fn codex(app: &AppHandle, account: &str, revision: u64, limits: &Value) {
    let (windows, complete) = codex_windows(limits);
    if complete {
        note(app, account, windows, revision);
    } else {
        note_codex_update(app, account, windows, revision);
    }
}

/// Convert Claude utilization fractions into percentages.
fn claude_windows(info: &Value) -> Vec<Window> {
    let windows = &info["unifiedWindows"];
    [
        ("five_hour", "session"),
        ("seven_day", "weekly"),
        // The overage-included protocol field represents the model-specific weekly allowance on
        // plans that include it.
        ("seven_day_overage_included", "fable"),
    ]
    .iter()
    .filter_map(|(from, kind)| {
        let window = &windows[from];
        Some(Window {
            kind: (*kind).to_string(),
            pct: (window["utilization"].as_f64()? * 100.0).clamp(0.0, 100.0),
            resets: window["resetsAt"].as_u64()?,
            scope: None,
            label: None,
        })
    })
    .collect()
}

/// New complete snapshots include a map by limitId; older versions expose rateLimits only. Track
/// whether a response may replace all state or contains a sparse update.
fn codex_windows(value: &Value) -> (Vec<Window>, bool) {
    if value.get("rateLimits").is_none() {
        return (codex_snapshot_windows(value, None), false);
    }

    let mut windows = vec![];
    if let Some(by_id) = value["rateLimitsByLimitId"].as_object() {
        for (id, snapshot) in by_id {
            windows.extend(codex_snapshot_windows(snapshot, Some(id)));
        }
    }
    // Treat the map as authoritative, falling back to the legacy bucket only when the map is
    // absent.
    if windows.is_empty() {
        windows.extend(codex_snapshot_windows(&value["rateLimits"], None));
    }
    (windows, true)
}

fn codex_snapshot_windows(snapshot: &Value, map_id: Option<&String>) -> Vec<Window> {
    let id = snapshot["limitId"].as_str().or(map_id.map(String::as_str));
    let scope = match id {
        None | Some("codex") => "general".to_string(),
        Some(id) => id.to_string(),
    };
    let label = snapshot["limitName"].as_str().map(str::to_string);
    [("primary", "session"), ("secondary", "weekly")]
        .iter()
        .filter_map(|(from, fallback)| {
            let window = &snapshot[from];
            Some(Window {
                kind: codex_kind(
                    window["windowDurationMins"].as_u64().map(|mins| mins * 60),
                    fallback,
                ),
                pct: window["usedPercent"].as_f64()?.clamp(0.0, 100.0),
                resets: window["resetsAt"].as_u64()?,
                scope: Some(scope.clone()),
                label: label.clone(),
            })
        })
        .collect()
}

fn codex_kind(seconds: Option<u64>, fallback: &str) -> String {
    match seconds {
        Some(18_000) => "session".to_string(),
        Some(604_800) => "weekly".to_string(),
        Some(seconds) => format!("duration:{seconds}"),
        None => fallback.to_string(),
    }
}

/* Polling */

pub struct Service {
    schedule: Mutex<usage_scheduler::Schedule>,
    changed: Condvar,
    epoch: AtomicU64,
    started: Instant,
}

impl Service {
    pub fn new() -> Self {
        Self {
            schedule: Mutex::new(usage_scheduler::Schedule::default()),
            changed: Condvar::new(),
            epoch: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    fn wake(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.changed.notify_all();
    }
}

fn account_info(profiles: &[accounts::Profile]) -> Vec<usage_scheduler::AccountInfo> {
    let selected = accounts::selected_ids();
    profiles
        .iter()
        .map(|profile| usage_scheduler::AccountInfo {
            id: profile.id.clone(),
            provider: profile.provider,
            revision: profile.revision,
            selected: selected.contains(&profile.id),
            logging_in: accounts::logging_in(&profile.id),
        })
        .collect()
}

/// A panel, account selection, or foreground return requests only stale selected profiles.
#[tauri::command]
pub fn usage_refresh(app: AppHandle, provider: Option<crate::state::ProviderId>) {
    refresh(&app, provider);
}

pub fn refresh(app: &AppHandle, provider: Option<crate::state::ProviderId>) {
    let profiles = accounts::profiles().unwrap_or_default();
    let service = app.state::<Service>();
    let mut schedule = lock(&service.schedule);
    schedule.sync(&account_info(&profiles));
    schedule.request(provider, service.now());
    drop(schedule);
    service.wake();
}

struct Probe {
    identity: Option<accounts::Identity>,
    windows: Option<Vec<Window>>,
    complete: bool,
}

fn probe(profile: &accounts::Profile) -> Probe {
    use crate::state::ProviderId;
    match profile.provider {
        ProviderId::Antigravity => Probe {
            identity: None,
            windows: crate::antigravity::quota(),
            complete: true,
        },
        ProviderId::RetiredGemini => Probe {
            identity: None,
            windows: None,
            complete: true,
        },
        ProviderId::Claude => Probe {
            identity: crate::claude::account_status(profile).ok(),
            windows: fetch_claude(profile).map(|info| claude_api_windows(&info)),
            complete: true,
        },
        ProviderId::Codex => {
            let (identity, limits) = match crate::codex::account_probe(profile) {
                Ok((identity, limits)) => (Some(identity), limits),
                Err(_) => (None, None),
            };
            let (windows, complete) = match limits {
                Some(limits) => codex_windows(&limits),
                None => (
                    fetch_codex(profile)
                        .map(|reply| codex_api_windows(&reply))
                        .unwrap_or_default(),
                    true,
                ),
            };
            Probe {
                identity,
                windows: Some(windows),
                complete,
            }
        }
    }
}

fn reset_due(windows: &[Window], now_mono: u64) -> Option<u64> {
    let wall = now();
    windows
        .iter()
        .filter_map(|window| window.resets.checked_sub(wall))
        .filter(|seconds| *seconds > 1)
        .min()
        .map(|seconds| now_mono.saturating_add(seconds))
}

/// One coordinator sleeps until the next deadline; due accounts probe independently so a slow
/// provider cannot delay another account. An in-flight generation is checked before publication.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || loop {
        let service = app.state::<Service>();
        let profiles = accounts::profiles().unwrap_or_default();
        let context = app.state::<background::State>().snapshot();
        let now = service.now();
        let (tickets, delay) = {
            let mut schedule = lock(&service.schedule);
            schedule.sync(&account_info(&profiles));
            let tickets = schedule.begin(now, context);
            let delay = schedule.next_delay(now, context);
            (tickets, delay)
        };
        let epoch = service.epoch.load(Ordering::Relaxed);
        for ticket in tickets {
            let Some(profile) = profiles
                .iter()
                .find(|profile| profile.id == ticket.id)
                .cloned()
            else {
                continue;
            };
            let worker_app = app.clone();
            std::thread::spawn(move || {
                let result = probe(&profile);
                let service = worker_app.state::<Service>();
                let mut schedule = lock(&service.schedule);
                if !schedule.current(&ticket)
                    || accounts::logging_in(&profile.id)
                    || !accounts::registered(&profile.id, Some(profile.revision))
                {
                    return;
                }
                if let Some(identity) = result.identity {
                    let _ = accounts::set_identity(&worker_app, &profile.id, identity);
                }
                let windows = result.windows.filter(|windows| !windows.is_empty());
                let next_reset = windows
                    .as_ref()
                    .and_then(|windows| reset_due(windows, service.now()));
                let success = windows.is_some();
                if let Some(windows) = windows {
                    record(
                        &worker_app,
                        &profile.id,
                        windows,
                        result.complete,
                        Some(profile.revision),
                    );
                }
                schedule.finish(&ticket, service.now(), success, next_reset);
                drop(schedule);
                service.wake();
            });
        }
        let guard = lock(&service.schedule);
        if service.epoch.load(Ordering::Relaxed) == epoch {
            let _ = service
                .changed
                .wait_timeout(guard, Duration::from_secs(delay));
        }
    });
}

/// Query Claude's native usage endpoint with its credential. Missing or expired authentication
/// preserves the last reading until a later refresh or turn.
fn fetch_claude(profile: &accounts::Profile) -> Option<Value> {
    reqwest::blocking::Client::new()
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(claude_token(profile)?)
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()
}

/// Read Claude's OAuth credential from a file first, then the macOS Keychain through security when
/// needed. Claude Code on Linux keeps it only in the file.
fn claude_token(profile: &accounts::Profile) -> Option<String> {
    let body = std::fs::read_to_string(profile.home.join(".credentials.json"))
        .ok()
        .or_else(|| keychain(profile))?;
    let creds: Value = serde_json::from_str(&body).ok()?;
    Some(creds["claudeAiOauth"]["accessToken"].as_str()?.to_string())
}

#[cfg(not(target_os = "macos"))]
fn keychain(_: &accounts::Profile) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn keychain(profile: &accounts::Profile) -> Option<String> {
    use sha2::{Digest, Sha256};
    let service = if profile.managed || std::env::var_os("CLAUDE_CONFIG_DIR").is_some() {
        let hash = format!(
            "{:x}",
            Sha256::digest(profile.home.to_string_lossy().as_bytes())
        );
        format!("Claude Code-credentials-{}", &hash[..8])
    } else {
        "Claude Code-credentials".into()
    };
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", &service, "-w"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Use Codex's CLI User-Agent for its status endpoint to receive JSON rather than a Cloudflare
/// challenge page.
fn fetch_codex(profile: &accounts::Profile) -> Option<Value> {
    let (token, account) = codex_auth(&profile.home)?;
    reqwest::blocking::Client::new()
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(token)
        .header("chatgpt-account-id", account)
        .header("User-Agent", "codex_cli_rs")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()
}

fn codex_auth(home: &std::path::Path) -> Option<(String, String)> {
    let body = std::fs::read_to_string(home.join("auth.json")).ok()?;
    let auth: Value = serde_json::from_str(&body).ok()?;
    let tokens = &auth["tokens"];
    Some((
        tokens["access_token"].as_str()?.to_string(),
        tokens["account_id"].as_str()?.to_string(),
    ))
}

/// Parse Claude endpoint percentages, RFC 3339 resets, and model-scoped weekly windows.
fn claude_api_windows(info: &Value) -> Vec<Window> {
    info["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|limit| {
            let kind = match limit["kind"].as_str()? {
                "session" => "session",
                "weekly_all" => "weekly",
                "weekly_scoped" => "fable",
                _ => return None,
            };
            Some(Window {
                kind: kind.to_string(),
                pct: limit["percent"].as_f64()?.clamp(0.0, 100.0),
                resets: rfc3339(limit["resets_at"].as_str()?)?,
                scope: None,
                label: None,
            })
        })
        .collect()
}

/// Codex reports general, model/feature, and code-review quotas with explicit durations. Primary is
/// not necessarily a five-hour window.
fn codex_api_windows(reply: &Value) -> Vec<Window> {
    let mut windows = codex_api_bucket(&reply["rate_limit"], "general", None);
    if let Some(additional) = reply["additional_rate_limits"].as_array() {
        for limit in additional {
            let id = limit["metered_feature"]
                .as_str()
                .or(limit["normal_model_slug"].as_str())
                .unwrap_or("additional");
            let label = limit["limit_name"].as_str();
            windows.extend(codex_api_bucket(&limit["rate_limit"], id, label));
        }
    }
    if !reply["code_review_rate_limit"].is_null() {
        let review = &reply["code_review_rate_limit"];
        let rate = review.get("rate_limit").unwrap_or(review);
        windows.extend(codex_api_bucket(rate, "code_review", None));
    }
    windows
}

fn codex_api_bucket(rate: &Value, scope: &str, label: Option<&str>) -> Vec<Window> {
    [
        ("primary_window", "session"),
        ("secondary_window", "weekly"),
    ]
    .iter()
    .filter_map(|(from, fallback)| {
        let window = &rate[from];
        Some(Window {
            kind: codex_kind(window["limit_window_seconds"].as_u64(), fallback),
            pct: window["used_percent"].as_f64()?.clamp(0.0, 100.0),
            resets: window["reset_at"].as_u64()?,
            scope: Some(scope.to_string()),
            label: label.map(str::to_string),
        })
    })
    .collect()
}

/// Parse the endpoint's timestamp into Unix seconds, accepting fractional seconds and an offset or
/// Z without adding a date dependency.
pub(crate) fn rfc3339(text: &str) -> Option<u64> {
    let (date, rest) = text.split_once('T')?;
    let mut ymd = date.splitn(3, '-').map(|part| part.parse::<i64>().ok());
    let (y, m, d) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let at = rest.find(['Z', '+', '-'])?;
    let (clock, zone) = rest.split_at(at);
    let mut hms = clock
        .splitn(3, ':')
        .map(|part| part.split('.').next()?.parse::<i64>().ok());
    let (h, min, s) = (hms.next()??, hms.next()??, hms.next()??);
    let offset = match zone {
        "Z" | "z" => 0,
        _ => {
            let (oh, om) = zone[1..].split_once(':')?;
            let secs = oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60;
            if zone.starts_with('-') {
                -secs
            } else {
                secs
            }
        }
    };
    let unix = days(y, m, d) * 86400 + h * 3600 + min * 60 + s - offset;
    u64::try_from(unix).ok()
}

/// Compute civil days since 1970-01-01 using Howard Hinnant's days_from_civil algorithm.
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Persist and publish changed readings only. Repeated rate-limit events must not rewrite disk
/// state or reset the displayed update time.
fn note(app: &AppHandle, agent: &str, windows: Vec<Window>, revision: u64) {
    let service = app.state::<Service>();
    let mut schedule = lock(&service.schedule);
    if !accounts::registered(agent, Some(revision)) {
        return;
    }
    let reset = reset_due(&windows, service.now());
    record(app, agent, windows, true, Some(revision));
    schedule.live(agent, service.now(), reset);
    drop(schedule);
    service.wake();
}

fn record(
    app: &AppHandle,
    account: &str,
    windows: Vec<Window>,
    complete: bool,
    revision: Option<u64>,
) {
    let mut all = lock(state());
    if !accounts::registered(account, revision) {
        return;
    }
    if !remember(&mut all, account, windows, complete) {
        return;
    }
    write(&all);
    let _ = app.emit("usage", &*all);
}

fn remember(all: &mut Usage, account: &str, windows: Vec<Window>, complete: bool) -> bool {
    if windows.is_empty() {
        return false;
    }
    let windows = if complete {
        windows
    } else {
        merge_codex(
            all.get(account)
                .map(|data| data.windows.as_slice())
                .unwrap_or_default(),
            windows,
        )
    };
    if all.get(account).is_some_and(|old| old.windows == windows) {
        return false;
    }
    all.insert(account.to_string(), Agent { windows, at: now() });
    true
}

/// Merge sparse app-server notifications by bucket and window, preserving omitted windows and
/// presentation fields from the complete snapshot.
fn merge_codex(old: &[Window], mut updates: Vec<Window>) -> Vec<Window> {
    if updates.is_empty() {
        return old.to_vec();
    }
    // Without a bucket identity, older Codex responses retain legacy snapshot semantics because
    // sparse updates cannot be distinguished.
    if updates.iter().any(|window| window.scope.is_none()) {
        return updates;
    }
    for update in &mut updates {
        if update.label.is_none() {
            update.label = old
                .iter()
                .find(|window| same_scope(window, update))
                .and_then(|window| window.label.clone());
        }
    }
    let mut merged: Vec<Window> = old
        .iter()
        .filter(|window| {
            !updates
                .iter()
                .any(|update| same_scope(update, window) && update.kind == window.kind)
        })
        .cloned()
        .collect();
    merged.extend(updates);
    merged.sort_by_key(|window| {
        old.iter()
            .position(|known| same_scope(known, window) && known.kind == window.kind)
            .unwrap_or(old.len())
    });
    merged
}

fn same_scope(a: &Window, b: &Window) -> bool {
    a.scope.as_deref().unwrap_or("general") == b.scope.as_deref().unwrap_or("general")
}

fn note_codex_update(app: &AppHandle, account: &str, windows: Vec<Window>, revision: u64) {
    let service = app.state::<Service>();
    let mut schedule = lock(&service.schedule);
    if !accounts::registered(account, Some(revision)) {
        return;
    }
    let reset = reset_due(&windows, service.now());
    record(app, account, windows, false, Some(revision));
    schedule.live(account, service.now(), reset);
    drop(schedule);
    service.wake();
}

fn path() -> std::path::PathBuf {
    paths::root().join("usage.json")
}

fn read() -> Usage {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

/// A persistence failure only loses the next startup's cached display; disk state is not the
/// authoritative quota.
fn write(usage: &Usage) {
    let Ok(json) = serde_json::to_string(usage) else {
        return;
    };
    if let Err(error) = paths::write_private(&path(), &json) {
        eprintln!("não gravei usage.json: {error}");
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn late_updates_from_previous_accounts_do_not_change_current_quota() {
        let mut all: Usage = serde_json::from_value(
            json!({"codex":{"windows":[{"kind":"weekly","pct":12,"resets":100}],"at":1}}),
        )
        .unwrap();
        let window = |pct| Window {
            kind: "session".into(),
            pct,
            resets: 200,
            scope: Some("general".into()),
            label: None,
        };
        remember(&mut all, "account-a", vec![window(20.0)], true);
        remember(&mut all, "account-b", vec![window(2.0)], true);
        let newer = all["account-b"].clone();
        remember(&mut all, "account-a", vec![window(90.0)], false);
        assert_eq!(all["account-b"], newer);
        assert_eq!(all["account-a"].windows[0].pct, 90.0);
        assert_eq!(all["codex"].windows[0].pct, 12.0);
        assert!(!remember(&mut all, "account-b", vec![], true));
        assert_eq!(all["account-b"], newer);
    }

    #[test]
    fn claude_translates_received_windows() {
        let info = json!({
            "status": "allowed",
            "unifiedWindows": {
                "five_hour": { "utilization": 0.22, "resetsAt": 1788238200u64 },
                "seven_day": { "utilization": 0.5, "resetsAt": 1788501600u64 },
                "seven_day_overage_included": {
                    "utilization": 0.72,
                    "resetsAt": 1788501600u64
                }
            }
        });
        let windows = claude_windows(&info);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].kind, "session");
        // A utilization fraction of 0.22 represents 22 percent.
        assert!((windows[0].pct - 22.0).abs() < 0.001);
        assert_eq!(windows[1].kind, "weekly");
        assert_eq!(windows[1].resets, 1788501600);
        assert_eq!(windows[2].kind, "fable");
        assert!((windows[2].pct - 72.0).abs() < 0.001);
    }

    #[test]
    fn claude_does_not_invent_missing_windows() {
        assert!(claude_windows(&json!({ "status": "allowed" })).is_empty());
    }

    #[test]
    fn claude_poll_translates_limits() {
        let info = json!({
            "limits": [
                { "kind": "session", "percent": 26, "resets_at": "2026-09-02T05:10:00.504892+00:00" },
                { "kind": "weekly_all", "percent": 7, "resets_at": "2026-09-04T06:00:00+00:00" },
                { "kind": "weekly_scoped", "percent": 4, "resets_at": "2026-09-04T06:00:00Z",
                  "scope": { "model": { "display_name": "Fable" } } },
                { "kind": "outra_coisa", "percent": 1, "resets_at": "2026-09-04T06:00:00Z" }
            ]
        });
        let windows = claude_api_windows(&info);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].kind, "session");
        assert!((windows[0].pct - 26.0).abs() < 0.001);
        assert_eq!(windows[1].kind, "weekly");
        assert_eq!(windows[2].kind, "fable");
        assert_eq!(windows[1].resets, windows[2].resets);
    }

    #[test]
    fn codex_poll_separates_general_and_model_quotas() {
        let reply = json!({
            "rate_limit": {
                "primary_window": { "used_percent": 6, "limit_window_seconds": 604800, "reset_at": 1789079183u64 },
                "secondary_window": null
            },
            "additional_rate_limits": [{
                "limit_name": "GPT-5.3-Codex-Spark",
                "metered_feature": "codex_bengalfox",
                "rate_limit": {
                    "primary_window": { "used_percent": 1, "limit_window_seconds": 18000, "reset_at": 1788503277u64 },
                    "secondary_window": { "used_percent": 0, "limit_window_seconds": 604800, "reset_at": 1789090077u64 }
                }
            }],
            "code_review_rate_limit": null
        });
        let windows = codex_api_windows(&reply);
        assert_eq!(windows.len(), 3);
        // Primary may represent a weekly general quota rather than five hours.
        assert_eq!(windows[0].kind, "weekly");
        assert_eq!(windows[0].scope.as_deref(), Some("general"));
        assert!((windows[0].pct - 6.0).abs() < 0.001);
        assert_eq!(windows[1].kind, "session");
        assert_eq!(windows[1].scope.as_deref(), Some("codex_bengalfox"));
        assert_eq!(windows[1].label.as_deref(), Some("GPT-5.3-Codex-Spark"));
        assert_eq!(windows[2].kind, "weekly");
    }

    #[test]
    fn rfc3339_converts_to_unix_time() {
        // date -u -j -f "%Y-%m-%dT%H:%M:%S" "2026-09-02T05:10:00" +%s
        assert_eq!(
            rfc3339("2026-09-02T05:10:00.504892+00:00"),
            Some(1788325800)
        );
        assert_eq!(rfc3339("2026-09-02T05:10:00Z"), Some(1788325800));
        // Cover the Unix epoch and offsets crossing a day boundary.
        assert_eq!(rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339("2026-09-02T02:10:00-03:00"), Some(1788325800));
        assert_eq!(rfc3339("2026-09-02T07:10:00+02:00"), Some(1788325800));
        assert_eq!(rfc3339("not a date"), None);
    }

    #[test]
    fn codex_translates_primary_and_secondary_windows() {
        let limits = json!({
            "primary": { "usedPercent": 0, "windowDurationMins": 300, "resetsAt": 1788245287u64 },
            "secondary": { "usedPercent": 13, "windowDurationMins": 10080, "resetsAt": 1788789678u64 }
        });
        let (windows, complete) = codex_windows(&limits);
        assert!(!complete);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].kind, "session");
        assert!((windows[0].pct - 0.0).abs() < 0.001);
        assert!((windows[1].pct - 13.0).abs() < 0.001);
        assert_eq!(windows[1].resets, 1788789678);
    }

    #[test]
    fn codex_prefers_snapshots_with_all_buckets() {
        let limits = json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": { "usedPercent": 6, "windowDurationMins": 10080, "resetsAt": 1789079183u64 }
            },
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "primary": { "usedPercent": 6, "windowDurationMins": 10080, "resetsAt": 1789079183u64 }
                },
                "codex_bengalfox": {
                    "limitId": "codex_bengalfox",
                    "limitName": "GPT-5.3-Codex-Spark",
                    "primary": { "usedPercent": 1, "windowDurationMins": 300, "resetsAt": 1788503277u64 },
                    "secondary": { "usedPercent": 0, "windowDurationMins": 10080, "resetsAt": 1789090077u64 }
                }
            }
        });
        let (windows, complete) = codex_windows(&limits);
        assert!(complete);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].scope.as_deref(), Some("general"));
        assert_eq!(windows[0].kind, "weekly");
        assert_eq!(windows[1].scope.as_deref(), Some("codex_bengalfox"));
        assert_eq!(windows[1].kind, "session");
    }

    #[test]
    fn sparse_codex_updates_preserve_other_buckets_and_windows() {
        let full = json!({
            "rateLimits": {},
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "primary": { "usedPercent": 6, "windowDurationMins": 10080, "resetsAt": 1789079183u64 }
                },
                "spark": {
                    "limitId": "spark",
                    "limitName": "Spark",
                    "primary": { "usedPercent": 1, "windowDurationMins": 300, "resetsAt": 1788503277u64 },
                    "secondary": { "usedPercent": 2, "windowDurationMins": 10080, "resetsAt": 1789090077u64 }
                }
            }
        });
        let (old, _) = codex_windows(&full);
        let (update, complete) = codex_windows(&json!({
            "limitId": "spark",
            "primary": { "usedPercent": 9, "windowDurationMins": 300, "resetsAt": 1788504277u64 }
        }));
        assert!(!complete);
        let merged = merge_codex(&old, update);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].scope.as_deref(), Some("general"));
        assert_eq!(merged[1].pct, 9.0);
        assert_eq!(merged[1].label.as_deref(), Some("Spark"));
        assert_eq!(merged[2].kind, "weekly");
        assert_eq!(merged[2].pct, 2.0);
    }

    #[test]
    fn legacy_usage_without_scope_remains_readable() {
        let window: Window = serde_json::from_value(json!({
            "kind": "session", "pct": 4, "resets": 1788330820u64
        }))
        .unwrap();
        assert_eq!(window.scope, None);
        assert_eq!(window.label, None);
        let (update, _) = codex_windows(&json!({
            "limitId": "codex",
            "primary": { "usedPercent": 7, "windowDurationMins": 300, "resetsAt": 1788331820u64 }
        }));
        let merged = merge_codex(&[window], update);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].pct, 7.0);
    }
}
