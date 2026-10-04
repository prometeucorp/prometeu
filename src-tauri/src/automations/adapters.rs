//! Bounded effect adapters. Commands are argument vectors, never shell programs.
use prometeu_core::command::{CommandError, CommandPolicy, CommandRunner, OutputPolicy};
use serde_json::{json, Value};
use std::{process::Command, time::Duration};

fn gh_capture(runner: &dyn CommandRunner<Command>, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new("gh");
    command.args(args);
    command.env("GH_PROMPT_DISABLED", "1");
    let output = runner.run(&mut command, &[], CommandPolicy {
        timeout: Duration::from_secs(30),
        stdout: OutputPolicy::Capture { limit: 8 * 1024 * 1024 },
        stderr: OutputPolicy::Capture { limit: 1024 * 1024 },
    }).map_err(|error| -> String { match error {
        CommandError::Unavailable => "automation_github_cli_missing: Install GitHub CLI and run gh auth login --hostname github.com".into(),
        CommandError::Timeout => "automation_github_timeout: GitHub did not respond within 30 seconds".into(),
        _ => "automation_github_transport: GitHub response could not be read safely".into(),
    } })?;
    if !output.success {
        // Never persist CLI stderr: it can contain credentials or arbitrary remote content.
        return Err("automation_github_unavailable: Check connectivity and gh auth status --hostname github.com".into());
    }
    Ok(output.stdout)
}

pub(super) fn gh(runner: &dyn CommandRunner<Command>, args: &[&str]) -> Result<Value, String> {
    let stdout = gh_capture(runner, args)?;
    if stdout.is_empty()
        || args.starts_with(&["pr", "merge"])
        || args.starts_with(&["run", "rerun"])
    {
        return Ok(json!({"accepted": true}));
    }
    serde_json::from_slice(&stdout)
        .map_err(|_| "automation_github_response: Invalid JSON response".into())
}

pub(super) fn identity(runner: &dyn CommandRunner<Command>) -> Result<String, String> {
    gh(runner, &["api", "--hostname", "github.com", "user"])?["login"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            "automation_github_identity: GitHub did not return an authenticated identity".into()
        })
}

pub(super) fn require_identity(
    runner: &dyn CommandRunner<Command>,
    expected: &str,
) -> Result<(), String> {
    if expected.is_empty() || identity(runner)? != expected {
        return Err("automation_identity_changed: Reconnect the saved account or explicitly revise the workflow identity".into());
    }
    Ok(())
}

pub(super) fn valid_repository(repository: &str) -> bool {
    let parts: Vec<_> = repository.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
}

pub(super) fn pr_status(
    runner: &dyn CommandRunner<Command>,
    repository: &str,
    number: u64,
) -> Result<Value, String> {
    if !valid_repository(repository) || number == 0 {
        return Err("automation_invalid_pull_request".into());
    }
    let url = format!("https://github.com/{repository}/pull/{number}");
    gh(runner, &["pr", "view", &url, "--json", "number,url,title,body,state,isDraft,headRefOid,headRefName,baseRefName,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,reviews,comments,author,changedFiles"])
}

pub(super) fn authored(
    runner: &dyn CommandRunner<Command>,
    repository: &str,
    login: &str,
) -> Result<Vec<Value>, String> {
    if !valid_repository(repository) {
        return Err("automation_repository_required".into());
    }
    require_identity(runner, login)?;
    let repo = format!("github.com/{repository}");
    let list = gh(
        runner,
        &[
            "pr", "list", "--repo", &repo, "--author", login, "--state", "open", "--limit", "101",
            "--json", "number",
        ],
    )?;
    let list = list
        .as_array()
        .filter(|items| items.len() <= 100)
        .ok_or("automation_github_truncated: Narrow the workflow repository scope")?;
    let mut results = Vec::new();
    for item in list {
        let number = item["number"]
            .as_u64()
            .ok_or("automation_github_response")?;
        let mut pr = pr_details(runner, repository, number, login)?;
        if pr["author"]["login"].as_str() != Some(login) {
            return Err("automation_github_author_changed".into());
        }
        pr["repository"] = json!(repository);
        pr["identity"] = json!(login);
        results.push(pr);
    }
    require_identity(runner, login)?;
    Ok(results)
}

pub(super) fn pr_details(
    runner: &dyn CommandRunner<Command>,
    repository: &str,
    number: u64,
    login: &str,
) -> Result<Value, String> {
    let mut pr = pr_status(runner, repository, number)?;
    for (field, path) in [
        (
            "comments",
            format!("repos/{repository}/issues/{number}/comments?per_page=100"),
        ),
        (
            "reviews",
            format!("repos/{repository}/pulls/{number}/reviews?per_page=100"),
        ),
        (
            "inlineComments",
            format!("repos/{repository}/pulls/{number}/comments?per_page=100"),
        ),
    ] {
        let pages = gh(
            runner,
            &[
                "api",
                "--hostname",
                "github.com",
                "--paginate",
                "--slurp",
                &path,
            ],
        )?;
        let mut values = Vec::new();
        for page in pages.as_array().ok_or("automation_github_response")? {
            for value in page.as_array().ok_or("automation_github_response")? {
                if value["user"]["login"].as_str() == Some(login) {
                    continue;
                }
                values.push(json!({"id":value["id"],"author":value["user"]["login"],"body":value["body"].as_str().unwrap_or("").chars().take(12_000).collect::<String>(),"state":value["state"],"updatedAt":value["updated_at"],"submittedAt":value["submitted_at"],"url":value["html_url"],"path":value["path"],"line":value["line"],"startLine":value["start_line"],"side":value["side"],"diffHunk":value["diff_hunk"],"commitId":value["commit_id"],"originalCommitId":value["original_commit_id"],"inReplyToId":value["in_reply_to_id"]}));
            }
        }
        pr[field] = json!(values);
    }
    pr["reviewThreads"] = review_threads(
        runner,
        repository,
        number,
        login,
        pr["headRefOid"].as_str().ok_or("automation_head_sha")?,
    )?;
    pr["actionRequired"] = json!(
        has_actionable_threads(&pr["reviewThreads"])
            || pr["reviewDecision"] == "CHANGES_REQUESTED"
            || pr["statusCheckRollup"]
                .as_array()
                .is_some_and(|checks| checks.iter().any(|check| matches!(
                    check["conclusion"].as_str().or(check["state"].as_str()),
                    Some(
                        "FAILURE"
                            | "ERROR"
                            | "TIMED_OUT"
                            | "CANCELLED"
                            | "ACTION_REQUIRED"
                            | "STARTUP_FAILURE"
                    )
                )))
    );
    let pages = gh(
        runner,
        &[
            "api",
            "--hostname",
            "github.com",
            "--paginate",
            "--slurp",
            &format!("repos/{repository}/pulls/{number}/files?per_page=100"),
        ],
    )?;
    pr["riskContext"] = risk_context(&pages, pr["changedFiles"].as_u64());
    pr["ciEvidence"] = ci_evidence(
        runner,
        repository,
        number,
        pr["headRefOid"].as_str().ok_or("automation_head_sha")?,
    )?;
    Ok(pr)
}

fn graphql(
    runner: &dyn CommandRunner<Command>,
    query: &str,
    variables: &[(&str, String)],
) -> Result<Value, String> {
    let mut args = vec![
        "api".to_owned(),
        "--hostname".into(),
        "github.com".into(),
        "graphql".into(),
        "-f".into(),
        format!("query={query}"),
    ];
    for (name, value) in variables {
        args.push(if *name == "number" { "-F" } else { "-f" }.into());
        args.push(format!("{name}={value}"));
    }
    let result = gh(runner, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    if result.get("errors").is_some() {
        return Err("automation_github_review_response".into());
    }
    result
        .get("data")
        .cloned()
        .ok_or_else(|| "automation_github_review_response".into())
}

fn thread_comments(
    runner: &dyn CommandRunner<Command>,
    id: &str,
    first: &Value,
    login: &str,
) -> Result<Vec<Value>, String> {
    let mut page = first.clone();
    let mut comments = Vec::new();
    for index in 0..5 {
        for comment in page["nodes"]
            .as_array()
            .ok_or("automation_github_review_response")?
        {
            if comment["author"]["login"].as_str() == Some(login) {
                continue;
            }
            comments.push(json!({"id":comment["id"],"author":comment["author"]["login"],"body":comment["body"].as_str().unwrap_or("").chars().take(12_000).collect::<String>(),"url":comment["url"],"updatedAt":comment["updatedAt"]}));
        }
        if page["pageInfo"]["hasNextPage"] == false {
            return Ok(comments);
        }
        if index == 4 {
            break;
        }
        let cursor = page["pageInfo"]["endCursor"]
            .as_str()
            .ok_or("automation_github_review_response")?
            .to_owned();
        let data=graphql(runner,"query ThreadComments($id:ID!,$cursor:String){node(id:$id){... on PullRequestReviewThread{comments(first:100,after:$cursor){nodes{id body url updatedAt author{login}} pageInfo{hasNextPage endCursor}}}}}",&[("id",id.into()),("cursor",cursor)])?;
        page = data["node"]["comments"].clone();
    }
    Err("automation_github_reviews_truncated: Review conversation exceeds the bounded pagination limit".into())
}

fn review_threads(
    runner: &dyn CommandRunner<Command>,
    repository: &str,
    number: u64,
    login: &str,
    sha: &str,
) -> Result<Value, String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or("automation_repository_required")?;
    let mut variables = vec![
        ("owner", owner.into()),
        ("name", name.into()),
        ("number", number.to_string()),
    ];
    let mut threads = Vec::new();
    for _ in 0..5 {
        let data=graphql(runner,"query ReviewThreads($owner:String!,$name:String!,$number:Int!,$cursor:String){repository(owner:$owner,name:$name){pullRequest(number:$number){headRefOid reviewThreads(first:100,after:$cursor){nodes{id isResolved isOutdated path line originalLine startLine diffSide comments(first:100){nodes{id body url updatedAt author{login}} pageInfo{hasNextPage endCursor}}} pageInfo{hasNextPage endCursor}}}}}",&variables)?;
        let pr = &data["repository"]["pullRequest"];
        if pr["headRefOid"].as_str() != Some(sha) {
            return Err("automation_pr_head_changed".into());
        }
        let page = &pr["reviewThreads"];
        for thread in page["nodes"]
            .as_array()
            .ok_or("automation_github_review_response")?
        {
            let id = thread["id"]
                .as_str()
                .ok_or("automation_github_review_response")?;
            let comments = thread_comments(runner, id, &thread["comments"], login)?;
            threads.push(json!({"id":id,"isResolved":thread["isResolved"],"isOutdated":thread["isOutdated"],"path":thread["path"],"line":thread["line"],"originalLine":thread["originalLine"],"startLine":thread["startLine"],"diffSide":thread["diffSide"],"comments":comments}));
        }
        if page["pageInfo"]["hasNextPage"] == false {
            return Ok(json!(threads));
        }
        variables.retain(|(key, _)| *key != "cursor");
        variables.push((
            "cursor",
            page["pageInfo"]["endCursor"]
                .as_str()
                .ok_or("automation_github_review_response")?
                .into(),
        ));
    }
    Err(
        "automation_github_reviews_truncated: Too many review threads to advance the event cursor"
            .into(),
    )
}
fn has_actionable_threads(threads: &Value) -> bool {
    threads.as_array().is_some_and(|threads| {
        threads.iter().any(|thread| {
            thread["isResolved"] == false
                && thread["isOutdated"] == false
                && thread["comments"]
                    .as_array()
                    .is_some_and(|comments| !comments.is_empty())
        })
    })
}

fn ci_evidence(
    runner: &dyn CommandRunner<Command>,
    repository: &str,
    number: u64,
    sha: &str,
) -> Result<Value, String> {
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("automation_head_sha".into());
    }
    let response = gh(
        runner,
        &[
            "api",
            "--hostname",
            "github.com",
            &format!("repos/{repository}/actions/runs?head_sha={sha}&per_page=100"),
        ],
    )?;
    let mut results = Vec::new();
    let mut omitted = false;
    for run in response["workflow_runs"]
        .as_array()
        .ok_or("automation_ci_response")?
    {
        if !matches!(run["conclusion"].as_str(), Some("failure" | "timed_out")) {
            continue;
        }
        if run["head_sha"].as_str() != Some(sha)
            || run["repository"]["full_name"].as_str() != Some(repository)
            || !run["pull_requests"]
                .as_array()
                .is_some_and(|prs| prs.iter().any(|pr| pr["number"].as_u64() == Some(number)))
        {
            continue;
        }
        if results.len() >= 3 {
            omitted = true;
            continue;
        }
        let id = run["id"].as_u64().ok_or("automation_ci_run_id")?;
        let logs = gh_capture(
            runner,
            &[
                "run",
                "view",
                &id.to_string(),
                "--repo",
                &format!("github.com/{repository}"),
                "--log-failed",
            ],
        )?;
        let text = String::from_utf8_lossy(&logs);
        results.push(json!({"runId":id,"attempt":run["run_attempt"],"conclusion":run["conclusion"],"url":run["html_url"],"failedLogs":text.chars().take(12_000).collect::<String>(),"truncated":text.chars().count()>12_000}));
    }
    Ok(
        json!({"runs":results,"truncated":omitted || response["total_count"].as_u64().is_some_and(|count|count>100),"source":"untrusted_ci_logs"}),
    )
}

fn risk_context(pages: &Value, expected: Option<u64>) -> Value {
    let Some(pages) = pages.as_array() else {
        return json!({"complete":false,"files":[],"source":"github_pull_diff"});
    };
    let mut files = Vec::new();
    let mut complete = true;
    for page in pages {
        let Some(items) = page.as_array() else {
            complete = false;
            continue;
        };
        for item in items {
            let patch = item["patch"].as_str();
            complete &= patch.is_some_and(|patch| {
                let additions = patch.lines().filter(|line| line.starts_with('+')).count() as u64;
                let deletions = patch.lines().filter(|line| line.starts_with('-')).count() as u64;
                item["additions"].as_u64() == Some(additions)
                    && item["deletions"].as_u64() == Some(deletions)
            });
            files.push(json!({"path":item["filename"],"status":item["status"],"additions":item["additions"],"deletions":item["deletions"],"patch":patch}));
        }
    }
    complete &= expected == Some(files.len() as u64) && !files.is_empty();
    let mut context = json!({"complete":complete,"files":files,"source":"github_pull_diff"});
    if serde_json::to_vec(&context).map_or(true, |bytes| bytes.len() > 16 * 1024) {
        context = json!({"complete":false,"files":[],"source":"github_pull_diff","reason":"diff_exceeds_16_kib"});
    }
    context
}

pub(super) fn linear_assigned(expected: &str, project: &str) -> Result<Vec<Value>, String> {
    if expected.is_empty() || project.is_empty() {
        return Err("automation_linear_mapping_required: Select a Linear identity and explicit Linear project mapping".into());
    }
    let token = crate::linear::token()
        .map_err(|_| "automation_linear_disconnected: Connect Linear in Settings")?;
    let mut after: Option<String> = None;
    let mut items = Vec::new();
    for _ in 0..5 {
        let data = crate::linear::graphql(&token,
            "query AutomationAssigned($after: String, $project: ID!) { viewer { id assignedIssues(first: 100, after: $after, filter: {project: {id: {eq: $project}}, state: {type: {nin: [\"completed\", \"canceled\"]}}}) { nodes { id identifier title description url updatedAt project { id name } assignee { id } state { type name } } pageInfo { hasNextPage endCursor } } } }",
            json!({"after": after, "project": project}))
            .map_err(|_| "automation_linear_unavailable: Check the Linear connection and network; the assignment cursor was preserved")?;
        if data["viewer"]["id"].as_str() != Some(expected) {
            return Err(
                "automation_identity_changed: Linear account differs from the workflow identity"
                    .into(),
            );
        }
        let page = &data["viewer"]["assignedIssues"];
        for item in page["nodes"]
            .as_array()
            .ok_or("automation_linear_response")?
        {
            if item["project"]["id"].as_str() != Some(project)
                || item["assignee"]["id"].as_str() != Some(expected)
            {
                return Err("automation_linear_mapping_mismatch".into());
            }
            items.push(item.clone());
        }
        if page["pageInfo"]["hasNextPage"] == false {
            // A second credential snapshot catches disconnect/account switches while fetching.
            if crate::linear::load().map(|a| a.who.id).as_deref() != Some(expected) {
                return Err("automation_identity_changed".into());
            }
            return Ok(items);
        }
        after = Some(
            page["pageInfo"]["endCursor"]
                .as_str()
                .ok_or("automation_linear_response")?
                .to_owned(),
        );
    }
    Err("automation_linear_truncated: Too many assigned issues; baseline was not advanced".into())
}

/// Re-fetch immediately before mutation; GitHub then atomically checks the same head SHA.
pub(super) fn merge(
    runner: &dyn CommandRunner<Command>,
    repository: &str,
    number: u64,
    login: &str,
    sha: &str,
) -> Result<Value, String> {
    require_identity(runner, login)?;
    let pr = pr_status(runner, repository, number)?;
    check_merge_status(&pr, login, sha)?;
    let url = format!("https://github.com/{repository}/pull/{number}");
    gh(
        runner,
        &["pr", "merge", &url, "--squash", "--match-head-commit", sha],
    )
}

pub(super) fn check_merge_status(pr: &Value, login: &str, sha: &str) -> Result<(), String> {
    if sha.len() != 40
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
        || pr["headRefOid"].as_str() != Some(sha)
    {
        return Err("automation_merge_stale_head".into());
    }
    if pr["author"]["login"].as_str() != Some(login)
        || pr["state"] != "OPEN"
        || pr["isDraft"] != false
        || pr["mergeable"] != "MERGEABLE"
        || pr["mergeStateStatus"] != "CLEAN"
        || pr["reviewDecision"] != "APPROVED"
    {
        return Err("automation_merge_policy: Pull request must be authored by the saved identity, open, non-draft, approved and cleanly mergeable".into());
    }
    let checks = pr["statusCheckRollup"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("automation_merge_checks_missing")?;
    if checks.iter().any(|check| {
        !(check["state"] == "SUCCESS"
            || (check["status"] == "COMPLETED" && check["conclusion"] == "SUCCESS"))
    }) {
        return Err("automation_merge_checks_incomplete".into());
    }
    Ok(())
}

pub(super) fn rerun(
    runner: &dyn CommandRunner<Command>,
    repository: &str,
    number: u64,
    login: &str,
    sha: &str,
    run_id: u64,
) -> Result<Value, String> {
    require_identity(runner, login)?;
    let pr = pr_status(runner, repository, number)?;
    if pr["headRefOid"].as_str() != Some(sha)
        || pr["author"]["login"].as_str() != Some(login)
        || pr["state"] != "OPEN"
    {
        return Err("automation_rerun_stale_pull_request".into());
    }
    let endpoint = format!("repos/{repository}/actions/runs/{run_id}");
    let run = gh(runner, &["api", "--hostname", "github.com", &endpoint])?;
    if run["head_sha"].as_str() != Some(sha)
        || run["repository"]["full_name"].as_str() != Some(repository)
        || !run["pull_requests"]
            .as_array()
            .is_some_and(|prs| prs.iter().any(|pr| pr["number"].as_u64() == Some(number)))
        || run["run_attempt"].as_u64().is_none()
        || run["status"] != "completed"
        || !matches!(run["conclusion"].as_str(), Some("failure" | "timed_out"))
    {
        return Err("automation_rerun_policy: Only failed completed runs for the current PR head can be retried".into());
    }
    let repo = format!("github.com/{repository}");
    gh(
        runner,
        &[
            "run",
            "rerun",
            &run_id.to_string(),
            "--repo",
            &repo,
            "--failed",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::command::CommandOutput;
    use std::sync::Mutex;
    struct Runner {
        replies: Mutex<Vec<Value>>,
        calls: Mutex<Vec<Vec<String>>>,
    }
    impl CommandRunner<Command> for Runner {
        fn run(
            &self,
            command: &mut Command,
            _: &[u8],
            policy: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            assert_eq!(policy.timeout, Duration::from_secs(30));
            self.calls.lock().unwrap().push(
                command
                    .get_args()
                    .map(|a| a.to_string_lossy().to_string())
                    .collect(),
            );
            let value = self.replies.lock().unwrap().remove(0);
            Ok(CommandOutput {
                success: true,
                stdout: serde_json::to_vec(&value).unwrap(),
                stderr: vec![],
            })
        }
    }
    fn pull() -> Value {
        json!({"headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "author":{"login":"owner"}, "state":"OPEN", "isDraft":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":"APPROVED","statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}]})
    }
    #[test]
    fn merge_binds_native_mutation_to_checked_sha() {
        let runner = Runner {
            replies: Mutex::new(vec![json!({"login":"owner"}), pull(), json!({})]),
            calls: Mutex::default(),
        };
        merge(
            &runner,
            "owner/repo",
            8,
            "owner",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let calls = runner.calls.lock().unwrap();
        assert_eq!(
            calls[2],
            [
                "pr",
                "merge",
                "https://github.com/owner/repo/pull/8",
                "--squash",
                "--match-head-commit",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ]
        );
    }
    #[test]
    fn stale_head_or_identity_never_mutates() {
        let runner = Runner {
            replies: Mutex::new(vec![json!({"login":"other"})]),
            calls: Mutex::default(),
        };
        assert!(merge(
            &runner,
            "owner/repo",
            8,
            "owner",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_err());
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        let runner = Runner {
            replies: Mutex::new(vec![json!({"login":"owner"}), pull()]),
            calls: Mutex::default(),
        };
        assert!(merge(
            &runner,
            "owner/repo",
            8,
            "owner",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        )
        .is_err());
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
    }
    #[test]
    fn checks_fail_closed_for_empty_pending_and_neutral() {
        let mut pr = pull();
        for checks in [
            json!([]),
            json!([{"status":"IN_PROGRESS"}]),
            json!([{"status":"COMPLETED","conclusion":"NEUTRAL"}]),
        ] {
            pr["statusCheckRollup"] = checks;
            assert!(
                check_merge_status(&pr, "owner", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .is_err()
            );
        }
    }
    #[test]
    fn risk_evidence_requires_complete_bounded_text_diffs() {
        let complete = json!([[{"filename":"app.rs","status":"modified","additions":1,"deletions":1,"patch":"@@ -1 +1 @@\n-old\n+new"}]]);
        assert_eq!(risk_context(&complete, Some(1))["complete"], true);
        assert_eq!(risk_context(&complete, Some(2))["complete"], false);
        let mut truncated = complete.clone();
        truncated[0][0]["additions"] = json!(2);
        assert_eq!(risk_context(&truncated, Some(1))["complete"], false);
        let binary = json!([[{"filename":"image.png","additions":0,"deletions":0}]]);
        assert_eq!(risk_context(&binary, Some(1))["complete"], false);
        let huge = json!([[{"filename":"app.rs","additions":1,"deletions":0,"patch":format!("+{}","x".repeat(20_000))}]]);
        let context = risk_context(&huge, Some(1));
        assert_eq!(context["complete"], false);
        assert!(serde_json::to_vec(&context).unwrap().len() < 16 * 1024);
    }
    #[test]
    fn unresolved_current_review_threads_include_locations_and_paginate_comments() {
        let first = json!({"data":{"repository":{"pullRequest":{"headRefOid":"sha","reviewThreads":{"nodes":[{"id":"thread","isResolved":false,"isOutdated":false,"path":"src/app.rs","line":12,"comments":{"nodes":[{"id":"self","body":"my reply","author":{"login":"owner"}},{"id":"review","body":"Please fix","author":{"login":"reviewer"}}],"pageInfo":{"hasNextPage":true,"endCursor":"comments-next"}}}],"pageInfo":{"hasNextPage":true,"endCursor":"threads-next"}}}}}});
        let comments = json!({"data":{"node":{"comments":{"nodes":[{"id":"review2","body":"More details","author":{"login":"reviewer"}}],"pageInfo":{"hasNextPage":false}}}}});
        let second = json!({"data":{"repository":{"pullRequest":{"headRefOid":"sha","reviewThreads":{"nodes":[{"id":"old","isResolved":false,"isOutdated":true,"path":"old.rs","comments":{"nodes":[{"id":"review3","body":"Old feedback","author":{"login":"reviewer"}}],"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}}}}}});
        let runner = Runner {
            replies: Mutex::new(vec![first, comments, second]),
            calls: Mutex::default(),
        };
        let threads = review_threads(&runner, "owner/repo", 42, "owner", "sha").unwrap();
        assert_eq!(threads[0]["path"], "src/app.rs");
        assert_eq!(threads[0]["line"], 12);
        assert_eq!(threads[0]["comments"].as_array().unwrap().len(), 2);
        assert!(has_actionable_threads(&threads));
        assert!(!has_actionable_threads(&json!([threads[1].clone()])));
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls[1].iter().any(|arg| arg == "cursor=comments-next"));
        assert!(calls[2].iter().any(|arg| arg == "cursor=threads-next"));
    }
}
