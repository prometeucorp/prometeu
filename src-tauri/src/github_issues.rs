//! GitHub issues and PRs through the GitHub App user credential (ADR 0088). Lists cover the
//! repositories where the App is installed and the person has access.
use crate::github_auth;
use crate::{i18n, lock::lock, AppState};
use prometeu_core::command::{CommandPolicy, CommandRunner, OutputPolicy};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::State;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Mine,
    Available,
    Authored,
    Reviews,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub description: Option<String>,
    pub repository: String,
    pub number: u64,
    pub kind: String,
    pub author: String,
    pub draft: bool,
    pub updated_at: String,
    pub labels: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct Issues {
    pub login: String,
    /// Repositories where the App is installed and the person has access.
    pub repositories: Vec<String>,
    pub items: Vec<Item>,
    pub fetched_at: u64,
    pub truncated: bool,
}

#[derive(Default)]
struct Cache {
    /// Changes on connect, disconnect and claim so late responses cannot repopulate it.
    generation: u64,
    lists: BTreeMap<String, Issues>,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// Drops every cached list after an account change or a claim.
pub fn forget() {
    let mut cache = lock(cache());
    cache.generation = cache.generation.wrapping_add(1);
    cache.lists.clear();
}

fn run(runner: &dyn CommandRunner<Command>, command: &mut Command) -> Result<Vec<u8>, String> {
    command.env("GIT_TERMINAL_PROMPT", "0");
    let output = runner
        .run(
            command,
            &[],
            CommandPolicy {
                timeout: Duration::from_secs(30),
                stdout: OutputPolicy::Capture {
                    limit: 8 * 1024 * 1024,
                },
                stderr: OutputPolicy::Capture { limit: 1024 * 1024 },
            },
        )
        .map_err(|error| i18n::ta("err.github.command", &[("cause", format!("{error:?}"))]))?;
    if !output.success {
        return Err(i18n::io(String::from_utf8_lossy(&output.stderr)));
    }
    Ok(output.stdout)
}

pub(crate) fn valid_repository(value: &str) -> bool {
    let Some((owner, repository)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && owner.len() <= 100
        && owner.as_bytes()[0].is_ascii_alphanumeric()
        && owner
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && !repository.is_empty()
        && repository.len() <= 100
        && !matches!(repository, "." | "..")
        && repository
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

/// Search qualifiers. Available issues are searched per installation account and then limited to
/// the installed repositories, since installations can select only some repositories.
fn query(scope: Scope, login: &str, account: &str) -> String {
    let filter = match scope {
        Scope::Mine => format!("is:issue assignee:{login}"),
        Scope::Available => format!("is:issue no:assignee user:{account}"),
        Scope::Authored => format!("is:pr author:{login}"),
        Scope::Reviews => format!("is:pr review-requested:{login}"),
    };
    format!("is:open {filter}")
}

/// Parse only canonical public GitHub issue/PR URLs. Reject credentials, query, fragments and ports.
pub(crate) fn target(value: &str) -> Option<(String, String, u64)> {
    let rest = value.strip_prefix("https://github.com/")?;
    let parts: Vec<_> = rest.split('/').collect();
    if parts.len() != 4
        || !matches!(parts[2], "issues" | "pull")
        || !parts[3].bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    let number = parts[3].parse().ok().filter(|n: &u64| *n > 0)?;
    valid_repository(&repo).then(|| (repo.to_ascii_lowercase(), parts[2].into(), number))
}

fn item(value: &Value) -> Result<Item, String> {
    let bad = || i18n::t("err.github.response");
    let url = value["html_url"].as_str().ok_or_else(bad)?;
    let (repository, kind, number) = target(url).ok_or_else(bad)?;
    if value["number"].as_u64() != Some(number) {
        return Err(bad());
    }
    Ok(Item {
        id: format!("github:{repository}/{kind}/{number}"),
        identifier: format!("{repository}#{number}"),
        title: value["title"].as_str().ok_or_else(bad)?.into(),
        url: url.into(),
        description: value["body"].as_str().map(str::to_owned),
        repository,
        number,
        kind: if kind == "pull" { "pr" } else { "issue" }.into(),
        author: value["user"]["login"].as_str().unwrap_or_default().into(),
        draft: value["draft"].as_bool().unwrap_or(false),
        updated_at: value["updated_at"].as_str().ok_or_else(bad)?.into(),
        labels: value["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|label| label["name"].as_str().map(str::to_owned))
            .collect(),
    })
}

/// Installation accounts and their accessible repositories, at most 500 per installation.
fn installed() -> Result<(Vec<String>, BTreeSet<String>), String> {
    let found = github_auth::get("/user/installations", &[("per_page", "100".into())])?;
    let mut accounts = Vec::new();
    let mut repositories = BTreeSet::new();
    for installation in found["installations"].as_array().into_iter().flatten() {
        let (Some(id), Some(account)) = (
            installation["id"].as_u64(),
            installation["account"]["login"]
                .as_str()
                .filter(|login| github_auth::valid_login(login)),
        ) else {
            continue;
        };
        accounts.push(account.to_owned());
        for page in 1..=5 {
            let value = github_auth::get(
                &format!("/user/installations/{id}/repositories"),
                &[("per_page", "100".into()), ("page", page.to_string())],
            )?;
            let rows = value["repositories"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            repositories.extend(
                rows.iter()
                    .filter_map(|repo| repo["full_name"].as_str())
                    .map(str::to_ascii_lowercase),
            );
            if rows.len() < 100 {
                break;
            }
        }
    }
    Ok((accounts, repositories))
}

/// Up to five pages of 100 results, most recently updated first.
fn search(query: &str, items: &mut Vec<Item>) -> Result<bool, String> {
    let mut truncated = false;
    let start = items.len();
    for page in 1..=5 {
        let value = github_auth::get(
            "/search/issues",
            &[
                ("q", query.into()),
                ("sort", "updated".into()),
                ("order", "desc".into()),
                ("per_page", "100".into()),
                ("page", page.to_string()),
            ],
        )?;
        let rows = value["items"]
            .as_array()
            .ok_or_else(|| i18n::t("err.github.response"))?;
        truncated |= value["incomplete_results"].as_bool().unwrap_or(false);
        for row in rows {
            items.push(item(row)?);
        }
        let total = value["total_count"]
            .as_u64()
            .ok_or_else(|| i18n::t("err.github.response"))?;
        if (items.len() - start) as u64 >= total || rows.len() < 100 {
            break;
        }
        if page == 5 {
            truncated = true;
        }
    }
    Ok(truncated)
}

fn fetch(scope: Scope, login: String) -> Result<Issues, String> {
    let (accounts, repositories) = installed()?;
    let mut result = Issues {
        login,
        repositories: repositories.iter().cloned().collect(),
        items: vec![],
        fetched_at: now(),
        truncated: false,
    };
    if scope == Scope::Available {
        for account in &accounts {
            result.truncated |= search(&query(scope, &result.login, account), &mut result.items)?;
        }
        result
            .items
            .retain(|item| repositories.contains(&item.repository));
        result.items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        result.items.dedup_by(|a, b| a.id == b.id);
    } else {
        result.truncated = search(&query(scope, &result.login, ""), &mut result.items)?;
    }
    Ok(result)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command(async)]
pub fn github_issues(scope: Scope, force: bool) -> Result<Issues, String> {
    let (login, _) = github_auth::token()?;
    issues(cache(), scope, force, login, fetch)
}

fn issues(
    cache: &Mutex<Cache>,
    scope: Scope,
    force: bool,
    login: String,
    fetch: impl FnOnce(Scope, String) -> Result<Issues, String>,
) -> Result<Issues, String> {
    let key = format!("{login}:{scope:?}");
    let generation = {
        let cache = lock(cache);
        if let Some(found) = cache
            .lists
            .get(&key)
            .filter(|found| !force && now().saturating_sub(found.fetched_at) < 120)
        {
            return Ok(found.clone());
        }
        cache.generation
    };
    // Network work stays outside the cache lock so independent scopes can load together.
    let found = fetch(scope, login)?;
    let mut cache = lock(cache);
    // A slower request from before an account change or claim must not repopulate the cache.
    if cache.generation == generation {
        cache.lists.insert(key, found.clone());
    }
    Ok(found)
}

/// Assigns an open, unassigned issue to the connected person.
#[tauri::command(async)]
pub fn github_claim(url: String) -> Result<Item, String> {
    let (repository, _, number) = target(&url)
        .filter(|(_, kind, _)| kind == "issues")
        .ok_or_else(|| i18n::t("err.github.url"))?;
    let (login, _) = github_auth::token()?;
    let path = format!("/repos/{repository}/issues/{number}");
    let current = github_auth::get(&path, &[])?;
    claimable(&current)?;
    // GitHub has no conditional assignment; another client can assign it after this check.
    let (status, updated) = github_auth::api(
        Method::POST,
        &format!("{path}/assignees"),
        &[],
        Some(serde_json::json!({ "assignees": [login] })),
    )?;
    // People without triage access are silently ignored, so check the result.
    let assigned = updated["assignees"].as_array().is_some_and(|people| {
        people
            .iter()
            .any(|person| person["login"].as_str() == Some(login.as_str()))
    });
    if status != 201 || !assigned {
        return Err(i18n::t("err.github.claimFailed"));
    }
    forget();
    item(&updated)
}

fn claimable(issue: &Value) -> Result<(), String> {
    let unassigned =
        issue["assignees"].as_array().is_some_and(Vec::is_empty) && issue["assignee"].is_null();
    if issue["state"] != "open" || !issue["pull_request"].is_null() || !unassigned {
        return Err(i18n::t("err.github.notAvailable"));
    }
    Ok(())
}

#[tauri::command]
pub fn github_issue_open(url: String) -> Result<(), String> {
    target(&url).ok_or_else(|| i18n::t("err.github.url"))?;
    crate::oauth::browse(&url).map_err(i18n::io)
}

pub(crate) fn remote_repository(remote: &str) -> Option<String> {
    let path = remote
        .trim()
        .strip_prefix("git@github.com:")
        .or_else(|| remote.trim().strip_prefix("https://github.com/"))
        .or_else(|| remote.trim().strip_prefix("ssh://git@github.com/"))?;
    let path = path
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_end_matches('/'));
    valid_repository(path).then(|| path.to_ascii_lowercase())
}

fn git(runner: &dyn CommandRunner<Command>, path: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(path).args(args);
    String::from_utf8(run(runner, &mut command)?)
        .map(|s| s.trim().into())
        .map_err(i18n::io)
}

#[derive(Serialize)]
pub struct Project {
    pub project: String,
    pub repository: String,
}

fn project_remotes(
    runner: &dyn CommandRunner<Command>,
    path: &Path,
) -> Result<Vec<(String, String)>, String> {
    let names = git(runner, path, &["remote"])?;
    Ok(names
        .lines()
        .filter(|name| !name.starts_with('-'))
        .filter_map(|name| {
            let url = git(runner, path, &["remote", "get-url", name]).ok()?;
            Some((name.to_owned(), remote_repository(&url)?))
        })
        .collect())
}

#[tauri::command(async)]
pub fn github_projects(state: State<AppState>) -> Vec<Project> {
    let projects = lock(&state.board).projects.clone();
    projects
        .into_iter()
        .flat_map(|project| {
            project_remotes(state.command_runner.as_ref(), Path::new(&project.path))
                .unwrap_or_default()
                .into_iter()
                .map(move |(_, repository)| Project {
                    project: project.id.clone(),
                    repository,
                })
        })
        .collect()
}

#[derive(Serialize)]
pub struct Prepared {
    pub base: String,
    pub branch: String,
    pub source: String,
}

/// Fetch the PR head through the base repository, including forks, without checking out the clone.
fn prepare(
    runner: &dyn CommandRunner<Command>,
    path: &Path,
    repository: &str,
    number: u64,
    pull: impl FnOnce(&str, u64) -> Result<Value, String>,
) -> Result<Prepared, String> {
    let remote = project_remotes(runner, path)?
        .into_iter()
        .find(|(_, found)| found == repository)
        .map(|(name, _)| name)
        .ok_or_else(|| i18n::t("err.github.project"))?;
    let pr = pull(repository, number)?;
    if pr["state"].as_str() != Some("open") {
        return Err(i18n::t("err.github.closed"));
    }
    let base = pr["base"]["ref"]
        .as_str()
        .ok_or_else(|| i18n::t("err.github.response"))?;
    git(runner, path, &["check-ref-format", "--branch", base])?;
    // Each preparation gets an isolated branch. Fork heads such as main and existing review
    // branches can never replace local commits or move the original checkout.
    let branch = format!(
        "github-pr-{number}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let source = format!("prometeu-pr-{number}/{branch}");
    let head = format!("refs/remotes/{source}");
    git(
        runner,
        path,
        &[
            "fetch",
            "--no-tags",
            &remote,
            &format!("+refs/pull/{number}/head:{head}"),
            &format!("+refs/heads/{base}:refs/remotes/{remote}/{base}"),
        ],
    )?;
    Ok(Prepared {
        base: format!("{remote}/{base}"),
        branch,
        source,
    })
}

#[tauri::command(async)]
pub fn github_prepare(
    state: State<AppState>,
    project: String,
    url: String,
) -> Result<Prepared, String> {
    let (repository, _, number) = target(&url)
        .filter(|(_, kind, _)| kind == "pull")
        .ok_or_else(|| i18n::t("err.github.url"))?;
    let path = lock(&state.board)
        .projects
        .iter()
        .find(|p| p.id == project)
        .map(|p| p.path.clone())
        .ok_or_else(|| i18n::t("err.github.project"))?;
    prepare(
        state.command_runner.as_ref(),
        Path::new(&path),
        &repository,
        number,
        |repository, number| github_auth::get(&format!("/repos/{repository}/pulls/{number}"), &[]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::command::{CommandError, CommandOutput};
    use serde_json::json;

    #[test]
    fn validates_repositories_and_external_links() {
        assert!(valid_repository("owner/.github"));
        assert!(valid_repository("owner/_repo"));
        assert!(valid_repository("owner/-repo"));
        for bad in [
            "-flags/repo",
            "owner/repo is:closed",
            "owner/repo/other",
            "https://github.com/o/r",
            "o/..",
        ] {
            assert!(!valid_repository(bad), "{bad}");
        }
        for bad in [
            "https://github.com.evil/o/r/issues/1",
            "https://github.com/o/r/pull/1?x=1",
            "https://github.com/o/r/pull/0",
            "file:///o/r/issues/1",
            "https://github.com/o/r/pull/1#fragment",
        ] {
            assert!(target(bad).is_none());
        }
        assert_eq!(
            target("https://github.com/Owner/Repo/pull/7"),
            Some(("owner/repo".into(), "pull".into(), 7))
        );
        for value in [
            "git@github.com:Owner/Repo.git",
            "https://github.com/Owner/Repo.git",
            "ssh://git@github.com/Owner/Repo.git",
        ] {
            assert_eq!(remote_repository(value).as_deref(), Some("owner/repo"));
        }
        assert!(remote_repository("https://github.com.evil/owner/repo").is_none());
    }

    #[test]
    fn scopes_search_personal_items_and_unassigned_issues_per_installation() {
        assert_eq!(
            query(Scope::Mine, "user", "org"),
            "is:open is:issue assignee:user"
        );
        assert_eq!(
            query(Scope::Authored, "user", ""),
            "is:open is:pr author:user"
        );
        assert_eq!(
            query(Scope::Reviews, "user", ""),
            "is:open is:pr review-requested:user"
        );
        assert_eq!(
            query(Scope::Available, "user", "org"),
            "is:open is:issue no:assignee user:org"
        );
        let value = json!({"html_url":"https://github.com/org/repo/issues/7", "number":7, "title":"Fix cache", "updated_at":"2026-10-03T00:00:00Z", "body":null});
        let mapped = item(&value).unwrap();
        assert_eq!(mapped.id, "github:org/repo/issues/7");
        assert!(!mapped.draft);
        let reference: prometeu_core::domain::IssueRef =
            serde_json::from_value(serde_json::to_value(mapped).unwrap()).unwrap();
        assert_eq!(reference.url, "https://github.com/org/repo/issues/7");
    }

    #[test]
    fn claims_only_open_unassigned_issues() {
        let open = json!({"state":"open", "assignee":null, "assignees":[]});
        assert!(claimable(&open).is_ok());
        assert!(claimable(&json!({"state":"closed", "assignee":null, "assignees":[]})).is_err());
        assert!(claimable(
            &json!({"state":"open", "assignee":{"login":"x"}, "assignees":[{"login":"x"}]})
        )
        .is_err());
        assert!(claimable(
            &json!({"state":"open", "assignee":null, "assignees":[], "pull_request":{}})
        )
        .is_err());
    }

    #[derive(Default)]
    struct PrepareRunner {
        fetched: Mutex<Vec<String>>,
    }
    impl CommandRunner<Command> for PrepareRunner {
        fn run(
            &self,
            command: &mut Command,
            _: &[u8],
            _: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            let args: Vec<_> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(command.get_program(), "git");
            assert_eq!(&args[..2], ["-C", "/fixture"]);
            let output = match args[2..]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["remote"] => "origin\nupstream\n".into(),
                ["remote", "get-url", "origin"] => "git@github.com:contributor/repo.git".into(),
                ["remote", "get-url", "upstream"] => "https://github.com/org/repo.git".into(),
                ["check-ref-format", "--branch", "main"] => "main".into(),
                ["fetch", "--no-tags", "upstream", ..] => {
                    *lock(&self.fetched) = args[2..].to_vec();
                    String::new()
                }
                _ => panic!("unexpected Git mutation: {args:?}"),
            };
            Ok(CommandOutput {
                success: true,
                stdout: output.into_bytes(),
                stderr: vec![],
            })
        }
    }

    fn open_pull(repository: &str, number: u64) -> Result<Value, String> {
        assert_eq!((repository, number), ("org/repo", 42));
        Ok(json!({"state":"open", "head":{"ref":"main"}, "base":{"ref":"main"}}))
    }

    #[test]
    fn fork_reviews_use_upstream_and_an_isolated_head_without_changing_the_clone() {
        let runner = PrepareRunner::default();
        let remotes = project_remotes(&runner, Path::new("/fixture")).unwrap();
        assert_eq!(
            remotes,
            [
                ("origin".into(), "contributor/repo".into()),
                ("upstream".into(), "org/repo".into())
            ]
        );
        let prepared = prepare(&runner, Path::new("/fixture"), "org/repo", 42, open_pull).unwrap();
        assert_eq!(prepared.base, "upstream/main");
        assert!(prepared.branch.starts_with("github-pr-42-"));
        assert_eq!(prepared.source.split_once('/').unwrap().1, prepared.branch);
        assert_eq!(
            *lock(&runner.fetched),
            [
                "fetch",
                "--no-tags",
                "upstream",
                &format!("+refs/pull/42/head:refs/remotes/{}", prepared.source),
                "+refs/heads/main:refs/remotes/upstream/main"
            ]
        );
        let next = prepare(&runner, Path::new("/fixture"), "org/repo", 42, open_pull).unwrap();
        assert_ne!(next.branch, prepared.branch);
        assert!(prepare(&runner, Path::new("/fixture"), "other/repo", 42, open_pull).is_err());
        let closed = |_: &str, _: u64| Ok(json!({"state":"closed", "base":{"ref":"main"}}));
        assert!(prepare(&runner, Path::new("/fixture"), "org/repo", 42, closed).is_err());
    }

    fn list(login: &str) -> Issues {
        Issues {
            login: login.into(),
            repositories: vec![],
            items: vec![],
            fetched_at: now(),
            truncated: false,
        }
    }

    #[test]
    fn lists_are_cached_per_account_and_late_results_cannot_survive_a_change() {
        let cache = Mutex::new(Cache::default());
        let first = issues(&cache, Scope::Mine, false, "first".into(), |_, login| {
            Ok(list(&login))
        })
        .unwrap();
        assert_eq!(first.login, "first");
        let cached = issues(&cache, Scope::Mine, false, "first".into(), |_, _| {
            panic!("cached")
        })
        .unwrap();
        assert_eq!(cached.login, "first");
        // A list fetched while the account changed is returned but not cached.
        let late = issues(
            &cache,
            Scope::Authored,
            false,
            "first".into(),
            |_, login| {
                let mut cache = lock(&cache);
                cache.generation += 1;
                cache.lists.clear();
                Ok(list(&login))
            },
        )
        .unwrap();
        assert_eq!(late.login, "first");
        assert!(lock(&cache).lists.is_empty());
        let other = issues(&cache, Scope::Mine, false, "second".into(), |_, login| {
            Ok(list(&login))
        })
        .unwrap();
        assert_eq!(other.login, "second");
    }
}
