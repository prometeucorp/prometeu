//! Linear OAuth runs locally with PKCE and no Prometeu server. A public client ID identifies the
//! integration; a fresh verifier replaces an embedded client secret. Listen on the registered fixed
//! loopback port, open authorization in the browser, exchange the returned code, persist the
//! credential privately, and publish status. The socket exists only during login. Tokens expire
//! after 24 hours and token() refreshes them before use. Credentials use a private file; the
//! requested scopes allow reading issues and assigning an unowned issue to the user.

use crate::i18n;
use crate::lock::lock;
use crate::oauth::{self, challenge, escape, form, now, random};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// The public OAuth client ID grants no access without user consent and the locally generated PKCE
/// verifier.
pub const CLIENT_ID: &str = "31c6d308b9f6d485d04c04ed94c26071";
const PORT: u16 = 17420;
const REDIRECT: &str = "http://localhost:17420/linear";
const AUTHORIZE: &str = "https://linear.app/oauth/authorize";
const TOKEN: &str = "https://api.linear.app/oauth/token";
const REVOKE: &str = "https://api.linear.app/oauth/revoke";
pub const GRAPHQL: &str = "https://api.linear.app/graphql";
/// Allow five minutes for the browser authorization callback.
const WAIT: Duration = Duration::from_secs(5 * 60);
/// Refresh early enough that an API call does not outlive its token.
const SLACK: u64 = 5 * 60;

/// Account identity used for assignment and displayed by the settings screen.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Who {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub email: String,
    /// The Linear organization and its urlKey.
    pub org: String,
    pub org_key: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Auth {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix timestamp in seconds.
    pub expires_at: u64,
    pub who: Who,
    /// Older credentials have no recorded scopes and keep read access until reauthorization.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Settings-screen connection status.
#[derive(Serialize, Clone)]
pub struct Status {
    pub connected: bool,
    pub can_assign: bool,
    pub who: Option<Who>,
    /// Retain pending browser-login status when the settings screen is reopened.
    pub busy: bool,
}

/// Permit one login flow at a time so repeated clicks cannot compete for the callback port.
static PENDING: AtomicBool = AtomicBool::new(false);

/* Commands */

#[tauri::command]
pub fn linear_status() -> Status {
    status()
}

/// Run browser waits and synchronous HTTP in spawn_blocking. Dropping reqwest::blocking's runtime
/// inside a Tokio async worker would panic.
#[tauri::command]
pub async fn linear_connect(app: AppHandle) -> Result<Status, String> {
    if PENDING.swap(true, Ordering::SeqCst) {
        return Err(i18n::t("err.linear.waiting"));
    }
    let result = blocking(connect).await;
    PENDING.store(false, Ordering::SeqCst);
    // Restore app focus after browser authorization.
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
    let now = status();
    let _ = app.emit("linear", &now);
    result.map(|_| now)
}

#[tauri::command]
pub async fn linear_disconnect(app: AppHandle) -> Status {
    let _ = blocking(|| {
        if let Some(auth) = load() {
            // Revocation is best effort; access expires after 24 hours, and removing the file also
            // removes the refresh credential.
            let _ = reqwest::blocking::Client::new()
                .post(REVOKE)
                .bearer_auth(&auth.access_token)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(form(&[("token", auth.access_token.as_str())]))
                .timeout(Duration::from_secs(10))
                .send();
        }
        let mut cache = lock(&CACHE);
        clear_issues(&mut cache);
        std::fs::remove_file(path()).map_err(i18n::io)
    })
    .await;
    let now = status();
    let _ = app.emit("linear", &now);
    now
}

/// Run token and graphql calls on a blocking thread because their synchronous HTTP cannot safely
/// run inside an async worker.
pub async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| i18n::ta("err.linear.taskDied", &[("cause", e.to_string())]))?
}

/* Authorization flow */

fn connect() -> Result<Auth, String> {
    let listener = TcpListener::bind(("127.0.0.1", PORT)).map_err(|e| {
        i18n::ta(
            "err.linear.port",
            &[("port", PORT.to_string()), ("cause", e.to_string())],
        )
    })?;
    listener.set_nonblocking(true).map_err(i18n::io)?;

    let verifier = random();
    let state = random();
    let url = format!(
        "{AUTHORIZE}?client_id={CLIENT_ID}&redirect_uri={}&response_type=code&scope=read,write\
         &state={state}&code_challenge={}&code_challenge_method=S256&prompt=consent",
        escape(REDIRECT),
        challenge(&verifier),
    );
    oauth::browse(&url).map_err(|_| i18n::t("err.linear.noBrowser"))?;

    let code = wait_for_code(&listener, &state, Instant::now() + WAIT)?;
    let mut auth = exchange(&code, &verifier)?;
    auth.who = whoami(&auth.access_token)?;
    let mut cache = lock(&CACHE);
    save(&auth)?;
    clear_issues(&mut cache);
    Ok(auth)
}

/// Use the shared OAuth callback listener while mapping refusals to Linear-specific error codes.
fn wait_for_code(listener: &TcpListener, state: &str, deadline: Instant) -> Result<String, String> {
    oauth::wait_for_code(listener, "/linear", state, deadline, &page).map_err(|denied| match denied
    {
        oauth::Denied::Refused => i18n::t("err.linear.denied"),
        oauth::Denied::Error(why) => i18n::ta("err.linear.refused", &[("why", why)]),
        oauth::Denied::NoCode => i18n::t("err.linear.noCode"),
        oauth::Denied::Timeout => i18n::t("err.linear.timeout"),
        oauth::Denied::Broken(cause) => i18n::ta(
            "err.linear.portDied",
            &[("port", PORT.to_string()), ("cause", cause)],
        ),
    })
}

/// Provide localized OAuth completion text to the shared HTML page, since this browser page cannot
/// use the frontend catalog.
fn page(ok: bool, why: &str) -> String {
    let (title, text) = match (ok, i18n::pt()) {
        (true, true) => (
            "Linear conectado",
            "Pode fechar esta aba e voltar ao Prometeu.".to_string(),
        ),
        (true, false) => (
            "Linear connected",
            "You can close this tab and go back to Prometeu.".to_string(),
        ),
        (false, true) => (
            "Não deu",
            format!("O Linear não autorizou o Prometeu. {why}")
                .trim()
                .to_string(),
        ),
        (false, false) => (
            "Did not work",
            format!("Linear did not authorize Prometeu. {why}")
                .trim()
                .to_string(),
        ),
    };
    oauth::page(title, &text)
}

/// Exchange the code using the PKCE verifier, without a client secret.
fn exchange(code: &str, verifier: &str) -> Result<Auth, String> {
    let got = token_request(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", REDIRECT),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ])?;
    Ok(got)
}

fn refresh(auth: &Auth) -> Result<Auth, String> {
    let rt = auth
        .refresh_token
        .as_deref()
        .ok_or_else(|| i18n::t("err.linear.expired"))?;
    let mut got = token_request(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", rt),
        ("client_id", CLIENT_ID),
    ])?;
    // Keep the previous refresh token when a refresh response omits its replacement.
    if got.refresh_token.is_none() {
        got.refresh_token = auth.refresh_token.clone();
    }
    got.who = auth.who.clone();
    if got.scopes.is_empty() {
        got.scopes = auth.scopes.clone();
    }
    Ok(got)
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<GrantedScopes>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GrantedScopes {
    Text(String),
    List(Vec<String>),
}

impl GrantedScopes {
    fn names(self) -> Vec<String> {
        match self {
            Self::Text(value) => value
                .split(|ch: char| ch.is_whitespace() || ch == ',')
                .filter(|scope| !scope.is_empty())
                .map(str::to_string)
                .collect(),
            Self::List(values) => values,
        }
    }
}

fn token_request(fields: &[(&str, &str)]) -> Result<Auth, String> {
    let reply: TokenReply = reqwest::blocking::Client::new()
        .post(TOKEN)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(fields))
        .timeout(Duration::from_secs(20))
        .send()
        .map_err(|e| i18n::ta("err.linear.unreachable", &[("cause", e.to_string())]))?
        .json()
        .map_err(|e| i18n::ta("err.linear.garbled", &[("cause", e.to_string())]))?;
    if let Some(err) = reply.error {
        return Err(format!(
            "o Linear recusou: {err} {}",
            reply.error_description.unwrap_or_default()
        )
        .trim()
        .to_string());
    }
    let access_token = reply
        .access_token
        .ok_or_else(|| i18n::t("err.linear.noToken"))?;
    Ok(Auth {
        access_token,
        refresh_token: reply.refresh_token,
        expires_at: now() + reply.expires_in.unwrap_or(86_400),
        who: Who::default(),
        scopes: reply.scope.map(GrantedScopes::names).unwrap_or_default(),
    })
}

/* ---------- GraphQL ---------- */

/// Refresh and persist an expiring token before every Linear API use.
pub fn token() -> Result<String, String> {
    let auth = load().ok_or_else(|| i18n::t("err.linear.off"))?;
    if auth.expires_at > now() + SLACK {
        return Ok(auth.access_token);
    }
    let fresh = refresh(&auth)?;
    save(&fresh)?;
    Ok(fresh.access_token)
}

/// Execute a GraphQL query with variables. Return data or convert GraphQL errors into Err.
pub fn graphql(
    token: &str,
    query: &str,
    vars: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let reply: serde_json::Value = reqwest::blocking::Client::new()
        .post(GRAPHQL)
        .bearer_auth(token)
        .json(&serde_json::json!({ "query": query, "variables": vars }))
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| i18n::ta("err.linear.unreachable", &[("cause", e.to_string())]))?
        .error_for_status()
        .map_err(|e| match e.status() {
            Some(reqwest::StatusCode::UNAUTHORIZED) => i18n::t("err.linear.rejected"),
            Some(reqwest::StatusCode::TOO_MANY_REQUESTS) => i18n::t("err.linear.slowDown"),
            _ => i18n::ta("err.linear.http", &[("status", e.to_string())]),
        })?
        .json()
        .map_err(|e| i18n::ta("err.linear.garbled", &[("cause", e.to_string())]))?;
    if let Some(errs) = reply.get("errors").and_then(|e| e.as_array()) {
        let msg = errs
            .iter()
            .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(i18n::ta("err.linear.queryRefused", &[("why", msg)]));
    }
    reply
        .get("data")
        .cloned()
        .ok_or_else(|| i18n::t("err.linear.noData"))
}

fn whoami(token: &str) -> Result<Who, String> {
    let data = graphql(
        token,
        "{ viewer { id name email organization { name urlKey } } }",
        serde_json::json!({}),
    )?;
    let v = &data["viewer"];
    let s = |x: &serde_json::Value| x.as_str().unwrap_or("").to_string();
    Ok(Who {
        id: s(&v["id"]),
        name: s(&v["name"]),
        email: s(&v["email"]),
        org: s(&v["organization"]["name"]),
        org_key: s(&v["organization"]["urlKey"]),
    })
}

/* Persistence */

fn path() -> PathBuf {
    paths::root().join("linear.json")
}

pub fn load() -> Option<Auth> {
    serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()
}

fn save(auth: &Auth) -> Result<(), String> {
    let body = serde_json::to_string_pretty(auth).map_err(|e| e.to_string())?;
    paths::write_private(&path(), &body).map_err(|cause| {
        i18n::ta(
            "err.linear.write",
            &[("path", path().display().to_string()), ("cause", cause)],
        )
    })
}

pub fn status() -> Status {
    let auth = load();
    let can_assign = auth.as_ref().is_some_and(can_assign);
    let who = auth.map(|a| a.who);
    Status {
        connected: who.is_some(),
        can_assign,
        who,
        busy: PENDING.load(Ordering::SeqCst),
    }
}

fn can_assign(auth: &Auth) -> bool {
    auth.scopes.iter().any(|scope| scope == "write") && !auth.who.id.is_empty()
}

/* ---------- issues ---------- */

#[derive(Serialize, Deserialize, Clone)]
pub struct IssueState {
    pub name: String,
    /// Group issues by Linear state type, such as backlog or started. The name is the team's custom
    /// label.
    pub kind: String,
    pub color: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Label {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    /// Use Linear's suggested branch name so it can associate the PR with this issue.
    pub branch_name: String,
    /// Linear priority: 0 none, 1 urgent, 2 high, 3 medium, 4 low.
    pub priority: u8,
    pub priority_label: String,
    pub state: IssueState,
    pub team: String,
    pub project: Option<String>,
    pub labels: Vec<Label>,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Issues {
    pub issues: Vec<Issue>,
    /// Open, unassigned issues in teams joined by the authenticated user.
    #[serde(default)]
    pub available: Vec<Issue>,
    /// An availability failure must not hide successfully fetched assigned issues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_error: Option<String>,
    /// Unix timestamp in seconds for relative update times.
    pub fetched_at: u64,
}

/// Reuse recently fetched lists; explicit refresh bypasses the cache.
const FRESH: u64 = 120;
/// Fetch at most ten pages of 50 issues.
const PAGES: usize = 10;

/// Keep the list in memory and persist a copy for immediate display after restart.
struct IssueCache {
    generation: u64,
    snapshot: Option<Issues>,
}

static CACHE: Mutex<IssueCache> = Mutex::new(IssueCache {
    generation: 0,
    snapshot: None,
});
// Serialize refreshes without holding the cache lock across Linear requests.
static REFRESH: Mutex<()> = Mutex::new(());

fn clear_issues(cache: &mut IssueCache) {
    cache.generation = cache.generation.wrapping_add(1);
    cache.snapshot = None;
    let _ = std::fs::remove_file(issues_path());
}

const ISSUES_QUERY: &str = r#"query Mine($after: String) {
  viewer {
    assignedIssues(first: 50, after: $after, orderBy: updatedAt,
      filter: { state: { type: { nin: ["completed", "canceled"] } } }) {
      pageInfo { hasNextPage endCursor }
      nodes {
        id identifier title description url branchName priority priorityLabel updatedAt
        state { name type color } team { key } project { name } labels { nodes { name color } }
      }
    }
  }
}"#;

const AVAILABLE_QUERY: &str = r#"query Available($after: String) {
  issues(first: 50, after: $after, orderBy: updatedAt,
    filter: { assignee: { null: true },
      team: { members: { some: { isMe: { eq: true } } } },
      state: { type: { nin: ["completed", "canceled"] } } }) {
    pageInfo { hasNextPage endCursor }
    nodes {
      id identifier title description url branchName priority priorityLabel updatedAt
      state { name type color } team { key } project { name } labels { nodes { name color } }
    }
  }
}"#;

const CLAIMABLE_QUERY: &str = r#"query Claimable($id: String!) {
  issue(id: $id) {
    id assignee { id } state { type }
    team { members(first: 1, filter: { isMe: { eq: true } }) { nodes { id } } }
  }
}"#;

const CLAIM_QUERY: &str = r#"mutation Claim($id: String!, $user: String!) {
  issueUpdate(id: $id, input: { assigneeId: $user }) {
    success
    issue {
      id identifier title description url branchName priority priorityLabel updatedAt
      assignee { id }
      state { name type color } team { key } project { name } labels { nodes { name color } }
    }
  }
}"#;

/// List assigned issues excluding completed and cancelled states. Force bypasses the two-minute
/// cache.
#[tauri::command]
pub async fn linear_issues(force: bool) -> Result<Issues, String> {
    blocking(move || issues(force)).await
}

fn issues(force: bool) -> Result<Issues, String> {
    let _refresh = lock(&REFRESH);
    loop {
        let (generation, snapshot) = {
            let cache = lock(&CACHE);
            (cache.generation, cache.snapshot.clone())
        };
        let cached = snapshot.or_else(load_issues);
        if let Some(c) = &cached {
            if !force && c.fetched_at.saturating_add(FRESH) > now() {
                let mut cache = lock(&CACHE);
                if cache.generation != generation {
                    continue;
                }
                cache.snapshot = Some(c.clone());
                return Ok(c.clone());
            }
        }
        let fresh = match fetch_issues(cached.as_ref()) {
            Ok(fresh) => fresh,
            Err(error) => {
                if lock(&CACHE).generation != generation {
                    continue;
                }
                return Err(error);
            }
        };
        let mut cache = lock(&CACHE);
        // A claim or account change invalidates any snapshot fetched before it completed.
        if cache.generation != generation {
            continue;
        }
        cache.snapshot = Some(fresh.clone());
        // A cache write failure must not discard a successfully fetched list.
        if let Ok(body) = serde_json::to_string(&fresh) {
            let _ = paths::write_private(&issues_path(), &body);
        }
        return Ok(fresh);
    }
}

fn fetch_issues(previous: Option<&Issues>) -> Result<Issues, String> {
    let token = token()?;
    let issues = fetch_pages(&token, ISSUES_QUERY, &["viewer", "assignedIssues"])?;
    let available = fetch_pages(&token, AVAILABLE_QUERY, &["issues"]);
    Ok(issue_lists(issues, available, previous))
}

fn issue_lists(
    issues: Vec<Issue>,
    available: Result<Vec<Issue>, String>,
    previous: Option<&Issues>,
) -> Issues {
    let (available, available_error) = match available {
        Ok(available) => (available, None),
        Err(error) => (
            previous
                .map(|old| old.available.clone())
                .unwrap_or_default(),
            Some(error),
        ),
    };
    Issues {
        issues,
        available,
        available_error,
        fetched_at: now(),
    }
}

fn fetch_pages(token: &str, query: &str, path: &[&str]) -> Result<Vec<Issue>, String> {
    let mut all = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..PAGES {
        let data = graphql(token, query, serde_json::json!({ "after": after }))?;
        let node = path.iter().fold(&data, |value, key| &value[*key]);
        let page: Page = serde_json::from_value(node.clone())
            .map_err(|e| i18n::ta("err.linear.garbled", &[("cause", e.to_string())]))?;
        all.extend(page.nodes.into_iter().map(Issue::from));
        if !page.page_info.has_next_page {
            break;
        }
        after = Some(
            page.page_info
                .end_cursor
                .ok_or_else(|| i18n::t("err.linear.noData"))?,
        );
    }
    Ok(all)
}

/// Assign one still-available team issue to the authenticated user.
#[tauri::command]
pub async fn linear_claim(id: String) -> Result<Issue, String> {
    blocking(move || claim(&id)).await
}

fn claim(id: &str) -> Result<Issue, String> {
    if id.is_empty() || id.len() > 128 {
        return Err(i18n::t("err.linear.notAvailable"));
    }
    let mut cache = lock(&CACHE);
    let auth = load().ok_or_else(|| i18n::t("err.linear.off"))?;
    if !can_assign(&auth) {
        return Err(i18n::t("err.linear.writeScope"));
    }
    let token = token()?;
    let current = graphql(&token, CLAIMABLE_QUERY, serde_json::json!({ "id": id }))?;
    ensure_claimable(&current["issue"])?;
    // Linear has no conditional issueUpdate; another client can assign the issue after this check.
    let updated = graphql(
        &token,
        CLAIM_QUERY,
        serde_json::json!({ "id": id, "user": &auth.who.id }),
    )?;
    if updated["issueUpdate"]["success"] != true {
        return Err(i18n::t("err.linear.claimFailed"));
    }
    if updated["issueUpdate"]["issue"]["assignee"]["id"] != auth.who.id {
        return Err(i18n::t("err.linear.claimFailed"));
    }
    let raw: Raw = serde_json::from_value(updated["issueUpdate"]["issue"].clone())
        .map_err(|e| i18n::ta("err.linear.garbled", &[("cause", e.to_string())]))?;
    clear_issues(&mut cache);
    Ok(raw.into())
}

fn ensure_claimable(issue: &serde_json::Value) -> Result<(), String> {
    let active = issue["state"]["type"]
        .as_str()
        .is_some_and(|kind| kind != "completed" && kind != "canceled");
    let member = issue["team"]["members"]["nodes"]
        .as_array()
        .is_some_and(|nodes| !nodes.is_empty());
    if issue["id"].as_str().is_none() || !issue["assignee"].is_null() || !active || !member {
        return Err(i18n::t("err.linear.notAvailable"));
    }
    Ok(())
}

fn issues_path() -> PathBuf {
    paths::root().join("linear-issues.json")
}

fn load_issues() -> Option<Issues> {
    parse_issue_cache(&std::fs::read_to_string(issues_path()).ok()?)
}

fn parse_issue_cache(body: &str) -> Option<Issues> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let has_available = value.get("available").is_some();
    let mut issues: Issues = serde_json::from_value(value).ok()?;
    if !has_available {
        issues.fetched_at = 0;
    }
    Some(issues)
}

/// Open only Linear issue URLs in the system browser; reject other destinations and schemes.
#[tauri::command]
pub fn linear_open(url: String) -> Result<(), String> {
    if !url.starts_with("https://linear.app/") {
        return Err(i18n::t("err.linear.notALink"));
    }
    oauth::browse(&url).map_err(|_| i18n::t("err.linear.noBrowser"))
}

/* Translate GraphQL response shapes into domain types. */

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    page_info: PageInfo,
    nodes: Vec<Raw>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    id: String,
    identifier: String,
    title: String,
    description: Option<String>,
    url: String,
    branch_name: String,
    priority: u8,
    priority_label: String,
    updated_at: String,
    state: RawState,
    team: Option<RawKey>,
    project: Option<RawName>,
    labels: RawLabels,
}

#[derive(Deserialize)]
struct RawState {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    color: String,
}

#[derive(Deserialize)]
struct RawKey {
    key: String,
}

#[derive(Deserialize)]
struct RawName {
    name: String,
}

#[derive(Deserialize)]
struct RawLabels {
    nodes: Vec<Label>,
}

impl From<Raw> for Issue {
    fn from(r: Raw) -> Issue {
        Issue {
            id: r.id,
            identifier: r.identifier,
            title: r.title,
            description: r.description.filter(|d| !d.trim().is_empty()),
            url: r.url,
            branch_name: r.branch_name,
            priority: r.priority,
            priority_label: r.priority_label,
            state: IssueState {
                name: r.state.name,
                kind: r.state.kind,
                color: r.state.color,
            },
            team: r.team.map(|t| t.key).unwrap_or_default(),
            project: r.project.map(|p| p.name),
            labels: r.labels.nodes,
            updated_at: r.updated_at,
        }
    }
}

/* Utilities */

#[cfg(test)]
mod tests {
    use super::*;
    // Shared OAuth helpers retain these tests because the assertions cover Linear's callback
    // contract.
    use crate::oauth::{parse_query, request_target, unescape};

    #[test]
    fn verifier_has_the_rfc_length() {
        let v = random();
        assert!((43..=128).contains(&v.len()), "{}", v.len());
        assert!(v.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(random(), random());
    }

    #[test]
    fn token_body_is_urlencoded() {
        assert_eq!(
            form(&[
                ("grant_type", "authorization_code"),
                ("redirect_uri", REDIRECT)
            ]),
            "grant_type=authorization_code&redirect_uri=http%3A%2F%2Flocalhost%3A17420%2Flinear"
        );
    }

    #[test]
    fn redirect_roundtrips_with_escaping() {
        assert_eq!(escape(REDIRECT), "http%3A%2F%2Flocalhost%3A17420%2Flinear");
        assert_eq!(unescape(&escape(REDIRECT)), REDIRECT);
        assert_eq!(unescape("a+b%20c%zz"), "a b c%zz");
        assert_eq!(unescape("fim%2"), "fim%2");
    }

    #[test]
    fn reads_request_target_and_query() {
        let req = "GET /linear?code=abc&state=xyz&error_description=User+denied HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let target = request_target(req).unwrap();
        let (route, query) = target.split_once('?').unwrap();
        assert_eq!(route, "/linear");
        let q = parse_query(query);
        assert_eq!(q["code"], "abc");
        assert_eq!(q["state"], "xyz");
        assert_eq!(q["error_description"], "User denied");
        assert!(request_target("POST /linear HTTP/1.1").is_none());
        assert!(request_target("").is_none());
    }

    /// Optional project, team, and description fields must not invalidate an otherwise valid issue
    /// page.
    #[test]
    fn reads_an_issues_page() {
        let json = serde_json::json!({
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": [{
                "id": "abc", "identifier": "MES-7", "title": "Conectar o Linear",
                "description": "  ", "url": "https://linear.app/mesonn/issue/MES-7/x",
                "branchName": "gustavo/mes-7-conectar-o-linear", "priority": 2,
                "priorityLabel": "High", "updatedAt": "2026-08-23T20:00:00.000Z",
                "state": { "name": "In Progress", "type": "started", "color": "#f2c94c" },
                "team": null, "project": null, "labels": { "nodes": [{ "name": "bug", "color": "#eb5757" }] }
            }]
        });
        let page: Page = serde_json::from_value(json).unwrap();
        assert!(!page.page_info.has_next_page);
        let issue = Issue::from(page.nodes.into_iter().next().unwrap());
        assert_eq!(issue.identifier, "MES-7");
        assert_eq!(issue.description, None);
        assert_eq!(issue.state.kind, "started");
        assert_eq!(issue.team, "");
        assert_eq!(issue.project, None);
        assert_eq!(issue.labels[0].name, "bug");
        assert_eq!(issue.branch_name, "gustavo/mes-7-conectar-o-linear");
    }

    #[test]
    fn old_credentials_and_issue_cache_keep_read_access() {
        let auth: Auth = serde_json::from_value(serde_json::json!({
            "access_token": "old", "refresh_token": null, "expires_at": 1,
            "who": { "name": "Ada", "email": "ada@example.com", "org": "Work", "org_key": "work" }
        }))
        .unwrap();
        assert_eq!(auth.who.id, "");
        assert!(auth.scopes.is_empty());
        assert!(!can_assign(&auth));

        let cached: Issues = serde_json::from_value(serde_json::json!({
            "issues": [], "fetched_at": 1
        }))
        .unwrap();
        assert!(cached.available.is_empty());
        assert!(cached.available_error.is_none());
        assert_eq!(
            parse_issue_cache(r#"{"issues":[],"fetched_at":123}"#)
                .unwrap()
                .fetched_at,
            0
        );
        assert_eq!(
            parse_issue_cache(r#"{"issues":[],"available":[],"fetched_at":123}"#)
                .unwrap()
                .fetched_at,
            123
        );
    }

    #[test]
    fn available_failure_keeps_assigned_issues_and_reports_error() {
        let assigned: Issue = serde_json::from_value(serde_json::json!({
            "id": "assigned", "identifier": "TEAM-1", "title": "Assigned",
            "description": null, "url": "https://linear.app/team/issue/TEAM-1/assigned",
            "branch_name": "team-1-assigned", "priority": 0, "priority_label": "None",
            "state": { "name": "Todo", "kind": "unstarted", "color": "#fff" },
            "team": "TEAM", "project": null, "labels": [], "updated_at": "2026-10-03T00:00:00Z"
        }))
        .unwrap();
        let fresh = issue_lists(vec![assigned.clone()], Err("available failed".into()), None);
        assert_eq!(fresh.issues[0].identifier, "TEAM-1");
        assert!(fresh.available.is_empty());
        assert_eq!(fresh.available_error.as_deref(), Some("available failed"));
        assert_eq!(
            serde_json::to_value(&fresh).unwrap()["available_error"],
            "available failed"
        );
        let mut prior_issue = assigned;
        prior_issue.identifier = "TEAM-2".into();
        let previous = issue_lists(vec![], Ok(vec![prior_issue]), None);
        let retried = issue_lists(
            fresh.issues,
            Err("still unavailable".into()),
            Some(&previous),
        );
        assert_eq!(retried.issues[0].identifier, "TEAM-1");
        assert_eq!(retried.available[0].identifier, "TEAM-2");
        assert_eq!(
            retried.available_error.as_deref(),
            Some("still unavailable")
        );
    }

    #[test]
    fn granted_write_scope_is_required_to_claim() {
        let auth: Auth = serde_json::from_value(serde_json::json!({
            "access_token": "new", "refresh_token": null, "expires_at": 1,
            "scopes": ["read", "write"],
            "who": { "id": "user-1", "name": "Ada", "email": "ada@example.com", "org": "Work", "org_key": "work" }
        }))
        .unwrap();
        assert!(can_assign(&auth));
        assert_eq!(
            serde_json::from_value::<GrantedScopes>(serde_json::json!("read write"))
                .unwrap()
                .names(),
            vec!["read", "write"]
        );
        assert_eq!(
            serde_json::from_value::<GrantedScopes>(serde_json::json!(["read", "write"]))
                .unwrap()
                .names(),
            vec!["read", "write"]
        );
    }

    #[test]
    fn claim_requires_unassigned_active_issue_in_joined_team() {
        let mut issue = serde_json::json!({
            "id": "issue-1", "assignee": null, "state": { "type": "unstarted" },
            "team": { "members": { "nodes": [{ "id": "user-1" }] } }
        });
        assert!(ensure_claimable(&issue).is_ok());
        issue["assignee"] = serde_json::json!({ "id": "other-user" });
        assert!(ensure_claimable(&issue).is_err());
        issue["assignee"] = serde_json::Value::Null;
        issue["state"]["type"] = serde_json::json!("completed");
        assert!(ensure_claimable(&issue).is_err());
        issue["state"]["type"] = serde_json::json!("unstarted");
        issue["team"]["members"]["nodes"] = serde_json::json!([]);
        assert!(ensure_claimable(&issue).is_err());
    }

    #[test]
    fn page_reports_the_outcome() {
        let _guard = i18n::TEST_LANG.lock().unwrap_or_else(|e| e.into_inner());
        i18n::set_lang("pt-BR".into());
        assert!(page(true, "").contains("Pode fechar esta aba"));
        assert!(page(false, "missing code").contains("missing code"));
        i18n::set_lang("en".into());
        assert!(page(true, "").contains("You can close this tab"));
        assert!(page(false, "no code").contains("no code"));
        i18n::set_lang("pt-BR".into());
    }
}
