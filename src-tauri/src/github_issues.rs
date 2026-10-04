//! Personal GitHub inbox through the existing gh credential. No tokens enter Prometeu storage.
use crate::{i18n, lock::lock, paths, AppState};
use prometeu_core::command::{CommandPolicy, CommandRunner, OutputPolicy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::State;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Mine,
    Repositories,
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
    pub repositories: Vec<String>,
    pub items: Vec<Item>,
    pub fetched_at: u64,
    pub truncated: bool,
}

#[derive(Default, Deserialize, Serialize)]
struct Settings {
    #[serde(default)]
    accounts: BTreeMap<String, Vec<String>>,
}

#[derive(Default)]
struct Cache {
    login: String,
    lists: BTreeMap<String, Issues>,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}
// ponytail: one lock serializes refreshes/settings writes; use per-account locks if contention matters.
static REFRESH: Mutex<()> = Mutex::new(());

fn run(runner: &dyn CommandRunner<Command>, command: &mut Command) -> Result<Vec<u8>, String> {
    command
        .env("GH_HOST", "github.com")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
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

fn gh(runner: &dyn CommandRunner<Command>, args: &[&str]) -> Result<Value, String> {
    let mut command = Command::new("gh");
    command.current_dir(paths::home()).args(args);
    serde_json::from_slice(&run(runner, &mut command)?).map_err(|_| i18n::t("err.github.response"))
}

fn viewer(runner: &dyn CommandRunner<Command>) -> Result<String, String> {
    let user = gh(runner, &["api", "--hostname", "github.com", "user"]).map_err(|_| {
        *lock(cache()) = Cache::default();
        i18n::t("err.github.auth")
    })?;
    user["login"]
        .as_str()
        .filter(|name| {
            !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
        .map(str::to_owned)
        .ok_or_else(|| i18n::t("err.github.response"))
}

fn valid_repository(value: &str) -> bool {
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

fn repositories(values: Vec<String>) -> Result<Vec<String>, String> {
    let mut values: Vec<_> = values
        .into_iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    if values.len() > 20 || values.iter().any(|s| !valid_repository(s)) {
        return Err(i18n::t("err.github.repositories"));
    }
    values.sort();
    values.dedup();
    Ok(values)
}

fn settings() -> Result<Settings, String> {
    match std::fs::read(paths::root().join("github-issues.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(i18n::io),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(i18n::io(error)),
    }
}

fn query(scope: Scope, login: &str, selected: &[String]) -> String {
    let filter = match scope {
        Scope::Mine => format!("is:issue assignee:{login}"),
        Scope::Repositories => format!(
            "is:issue {}",
            selected
                .iter()
                .map(|repo| format!("repo:{repo}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
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

fn fetch(
    runner: &dyn CommandRunner<Command>,
    scope: Scope,
    login: String,
    selected: Vec<String>,
) -> Result<Issues, String> {
    let mut result = Issues {
        login,
        repositories: selected,
        items: vec![],
        fetched_at: now(),
        truncated: false,
    };
    if scope == Scope::Repositories && result.repositories.is_empty() {
        return Ok(result);
    }
    let search = query(scope, &result.login, &result.repositories);
    for page in 1..=5 {
        let value = gh(
            runner,
            &[
                "api",
                "--hostname",
                "github.com",
                "search/issues",
                "--method",
                "GET",
                "-f",
                &format!("q={search}"),
                "-f",
                "sort=updated",
                "-f",
                "order=desc",
                "-f",
                "per_page=100",
                "-f",
                &format!("page={page}"),
            ],
        )?;
        let rows = value["items"]
            .as_array()
            .ok_or_else(|| i18n::t("err.github.response"))?;
        result.truncated |= value["incomplete_results"].as_bool().unwrap_or(false);
        for row in rows {
            result.items.push(item(row)?);
        }
        let total = value["total_count"]
            .as_u64()
            .ok_or_else(|| i18n::t("err.github.response"))?;
        if result.items.len() as u64 >= total || rows.len() < 100 {
            break;
        }
        if page == 5 {
            result.truncated = true;
        }
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
pub fn github_issues(state: State<AppState>, scope: Scope, force: bool) -> Result<Issues, String> {
    let _refresh = lock(&REFRESH);
    let login = viewer(state.command_runner.as_ref())?;
    let selected = settings()?.accounts.remove(&login).unwrap_or_default();
    let key = format!("{scope:?}:{selected:?}");
    {
        let mut cache = lock(cache());
        if cache.login != login {
            cache.login = login.clone();
            cache.lists.clear();
        }
        if let Some(found) = cache
            .lists
            .get(&key)
            .filter(|found| !force && now().saturating_sub(found.fetched_at) < 120)
        {
            return Ok(found.clone());
        }
    }
    let found = fetch(state.command_runner.as_ref(), scope, login, selected)?;
    lock(cache()).lists.insert(key, found.clone());
    Ok(found)
}

#[tauri::command(async)]
pub fn github_repositories(
    state: State<AppState>,
    selected: Vec<String>,
    login: String,
) -> Result<Vec<String>, String> {
    let selected = repositories(selected)?;
    let _refresh = lock(&REFRESH);
    if viewer(state.command_runner.as_ref())? != login {
        return Err(i18n::t("err.github.accountChanged"));
    }
    let mut settings = settings()?;
    settings.accounts.insert(login, selected.clone());
    paths::write_private(
        &paths::root().join("github-issues.json"),
        &serde_json::to_string(&settings).map_err(i18n::io)?,
    )
    .map_err(i18n::io)?;
    lock(cache()).lists.clear();
    Ok(selected)
}

#[tauri::command]
pub fn github_issue_open(url: String) -> Result<(), String> {
    target(&url).ok_or_else(|| i18n::t("err.github.url"))?;
    crate::oauth::browse(&url).map_err(i18n::io)
}

fn remote_repository(remote: &str) -> Option<String> {
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
) -> Result<Prepared, String> {
    let remote = project_remotes(runner, path)?
        .into_iter()
        .find(|(_, found)| found == repository)
        .map(|(name, _)| name)
        .ok_or_else(|| i18n::t("err.github.project"))?;
    let pr = gh(
        runner,
        &[
            "api",
            "--hostname",
            "github.com",
            &format!("repos/{repository}/pulls/{number}"),
        ],
    )?;
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::command::{CommandError, CommandOutput};
    use serde_json::json;

    #[test]
    fn validates_repository_selection_and_external_links() {
        assert!(valid_repository("owner/.github"));
        assert!(valid_repository("owner/_repo"));
        assert!(valid_repository("owner/-repo"));
        assert_eq!(
            repositories(vec!["Owner/Repo".into(), "owner/repo".into()]).unwrap(),
            ["owner/repo"]
        );
        for bad in [
            "-flags/repo",
            "owner/repo is:closed",
            "owner/repo/other",
            "https://github.com/o/r",
            "o/..",
        ] {
            assert!(repositories(vec![bad.into()]).is_err(), "{bad}");
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
    fn personal_queries_are_independent_of_repository_selection() {
        let selected = vec!["org/repo".into()];
        assert_eq!(
            query(Scope::Mine, "user", &selected),
            "is:open is:issue assignee:user"
        );
        assert_eq!(
            query(Scope::Authored, "user", &selected),
            "is:open is:pr author:user"
        );
        assert_eq!(
            query(Scope::Reviews, "user", &selected),
            "is:open is:pr review-requested:user"
        );
        assert_eq!(
            query(Scope::Repositories, "user", &selected),
            "is:open is:issue repo:org/repo"
        );
        let value = json!({"html_url":"https://github.com/org/repo/issues/7", "number":7, "title":"Fix cache", "updated_at":"2026-10-03T00:00:00Z", "body":null});
        let mapped = item(&value).unwrap();
        assert_eq!(mapped.id, "github:org/repo/issues/7");
        assert!(!mapped.draft);
        let reference: prometeu_core::domain::IssueRef =
            serde_json::from_value(serde_json::to_value(mapped).unwrap()).unwrap();
        assert_eq!(reference.url, "https://github.com/org/repo/issues/7");
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
            let output = if command.get_program() == "gh" {
                assert!(args.contains(&"repos/org/repo/pulls/42".into()));
                serde_json::to_string(
                    &json!({"state":"open", "head":{"ref":"main"}, "base":{"ref":"main"}}),
                )
                .unwrap()
            } else {
                assert_eq!(command.get_program(), "git");
                assert_eq!(&args[..2], ["-C", "/fixture"]);
                match args[2..]
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
                }
            };
            Ok(CommandOutput {
                success: true,
                stdout: output.into_bytes(),
                stderr: vec![],
            })
        }
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
        let prepared = prepare(&runner, Path::new("/fixture"), "org/repo", 42).unwrap();
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
        let next = prepare(&runner, Path::new("/fixture"), "org/repo", 42).unwrap();
        assert_ne!(next.branch, prepared.branch);
        assert!(prepare(&runner, Path::new("/fixture"), "other/repo", 42).is_err());
    }

    struct SearchRunner;
    impl CommandRunner<Command> for SearchRunner {
        fn run(
            &self,
            command: &mut Command,
            _: &[u8],
            policy: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            assert_eq!(policy.timeout, Duration::from_secs(30));
            let args: Vec<_> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert!(args.contains(&"q=is:open is:pr review-requested:user".into()));
            let page: u64 = args
                .iter()
                .find_map(|a| a.strip_prefix("page=").and_then(|s| s.parse().ok()))
                .unwrap();
            let items: Vec<_> = (0..100).map(|index| { let number = (page - 1) * 100 + index + 1; json!({"html_url":format!("https://github.com/org/repo/pull/{number}"),"number":number,"title":"Review","updated_at":"2026-10-03T00:00:00Z"}) }).collect();
            Ok(CommandOutput {
                success: true,
                stdout: serde_json::to_vec(
                    &json!({"total_count":501,"items":items,"incomplete_results":false}),
                )
                .unwrap(),
                stderr: vec![],
            })
        }
    }
    #[test]
    fn paginates_with_an_explicit_limit_and_skips_an_empty_repository_scope() {
        let result = fetch(&SearchRunner, Scope::Reviews, "user".into(), vec![]).unwrap();
        assert_eq!(result.items.len(), 500);
        assert!(result.truncated);
        assert!(
            fetch(&SearchRunner, Scope::Repositories, "user".into(), vec![])
                .unwrap()
                .items
                .is_empty()
        );
    }
}
