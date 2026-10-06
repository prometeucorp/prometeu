//! GitHub notifications (ADR 0088). The Cloud feed carries metadata only; titles and comment text
//! come from GitHub on demand with the local App credential. Read state belongs to the webview.

use crate::github_issues::valid_repository;
use crate::{github_auth, i18n};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const KINDS: &[&str] = &[
    "review_approved",
    "changes_requested",
    "review_requested",
    "commented",
    "mentioned",
    "assigned",
    "merged",
    "closed",
    "ci_failed",
];
const TARGETS: &[&str] = &["issue_comment", "review", "review_comment", "workflow_run"];

#[derive(Deserialize, Serialize, Clone)]
pub struct Notification {
    pub id: String,
    pub kind: String,
    pub repository: String,
    pub number: u64,
    pub subject: String,
    pub actor: String,
    pub target: Option<String>,
    pub target_id: Option<String>,
    pub url: String,
    pub created_at: String,
}

#[derive(Deserialize, Serialize)]
pub struct Linked {
    pub login: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Feed {
    pub github: Option<Linked>,
    pub notifications: Vec<Notification>,
    pub more: bool,
}

/// None without a Prometeu account. Rows outside the contract are dropped, not trusted.
#[tauri::command]
pub async fn github_feed(after: Option<String>) -> Result<Option<Feed>, String> {
    crate::linear::blocking(move || {
        let mut path = "/api/notifications".to_owned();
        if let Some(after) = after.filter(|value| !value.is_empty()) {
            if after.len() > 18 || !after.bytes().all(|c| c.is_ascii_digit()) {
                return Err(i18n::t("err.cloud.response"));
            }
            path = format!("{path}?after={after}");
        }
        let Some((status, value)) =
            crate::cloud::api(Method::GET, &path, None, Duration::from_secs(15))?
        else {
            return Ok(None);
        };
        if status == 401 {
            return Ok(None);
        }
        if status != 200 {
            return Err(i18n::t("err.cloud.response"));
        }
        let mut feed: Feed =
            serde_json::from_value(value).map_err(|_| i18n::t("err.cloud.response"))?;
        feed.notifications.retain(valid);
        Ok(Some(feed))
    })
    .await
}

fn valid(row: &Notification) -> bool {
    KINDS.contains(&row.kind.as_str())
        && matches!(row.subject.as_str(), "pr" | "issue")
        && valid_repository(&row.repository)
        && row
            .target
            .as_deref()
            .is_none_or(|target| TARGETS.contains(&target))
        && row.target_id.as_deref().is_none_or(|id| {
            !id.is_empty() && id.len() <= 20 && id.bytes().all(|c| c.is_ascii_digit())
        })
        && row.url.starts_with("https://github.com/")
        && row.id.len() <= 20
}

#[derive(Deserialize)]
pub struct SubjectKey {
    pub repository: String,
    pub number: u64,
}

#[derive(Serialize, PartialEq, Debug)]
pub struct Subject {
    pub repository: String,
    pub number: u64,
    pub title: String,
    /// OPEN, CLOSED or MERGED.
    pub state: String,
}

/// Titles and states for up to 50 issues or PRs in one GraphQL request. Missing or inaccessible
/// items are omitted.
#[tauri::command]
pub async fn github_subjects(keys: Vec<SubjectKey>) -> Result<Vec<Subject>, String> {
    crate::linear::blocking(move || {
        let keys: Vec<_> = keys
            .into_iter()
            .filter(|key| {
                valid_repository(&key.repository) && key.number > 0 && key.number <= i32::MAX as u64
            })
            .take(50)
            .collect();
        if keys.is_empty() {
            return Ok(vec![]);
        }
        let (query, variables) = subjects_query(&keys);
        Ok(subjects(&keys, &github_auth::graphql(&query, variables)?))
    })
    .await
}

fn subjects_query(keys: &[SubjectKey]) -> (String, Value) {
    let mut parameters = Vec::new();
    let mut fields = Vec::new();
    let mut variables = serde_json::Map::new();
    for (index, key) in keys.iter().enumerate() {
        let (owner, name) = key.repository.split_once('/').unwrap_or_default();
        parameters.push(format!(
            "$o{index}: String!, $n{index}: String!, $i{index}: Int!"
        ));
        fields.push(format!(
            "s{index}: repository(owner: $o{index}, name: $n{index}) {{ issueOrPullRequest(number: $i{index}) {{ \
             ... on Issue {{ title state }} ... on PullRequest {{ title state }} }} }}"
        ));
        variables.insert(format!("o{index}"), json!(owner));
        variables.insert(format!("n{index}"), json!(name));
        variables.insert(format!("i{index}"), json!(key.number));
    }
    (
        format!(
            "query({}) {{ {} }}",
            parameters.join(", "),
            fields.join(" ")
        ),
        Value::Object(variables),
    )
}

fn subjects(keys: &[SubjectKey], data: &Value) -> Vec<Subject> {
    keys.iter()
        .enumerate()
        .filter_map(|(index, key)| {
            let node = &data[format!("s{index}")]["issueOrPullRequest"];
            Some(Subject {
                repository: key.repository.clone(),
                number: key.number,
                title: node["title"].as_str()?.into(),
                state: node["state"].as_str()?.into(),
            })
        })
        .collect()
}

/// The current text of a comment or review, or the name of a workflow run.
#[tauri::command]
pub async fn github_detail(
    repository: String,
    number: u64,
    target: String,
    id: String,
) -> Result<String, String> {
    crate::linear::blocking(move || {
        if !valid_repository(&repository)
            || id.is_empty()
            || id.len() > 20
            || !id.bytes().all(|c| c.is_ascii_digit())
        {
            return Err(i18n::t("err.github.response"));
        }
        let (path, field) = match target.as_str() {
            "issue_comment" => (format!("/repos/{repository}/issues/comments/{id}"), "body"),
            "review" => (
                format!("/repos/{repository}/pulls/{number}/reviews/{id}"),
                "body",
            ),
            "review_comment" => (format!("/repos/{repository}/pulls/comments/{id}"), "body"),
            "workflow_run" => (format!("/repos/{repository}/actions/runs/{id}"), "name"),
            _ => return Err(i18n::t("err.github.response")),
        };
        let value = github_auth::get(&path, &[])?;
        Ok(value[field].as_str().unwrap_or_default().to_owned())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Notification {
        Notification {
            id: "7001".into(),
            kind: "review_approved".into(),
            repository: "example/app".into(),
            number: 12,
            subject: "pr".into(),
            actor: "fixture-reviewer".into(),
            target: Some("review".into()),
            target_id: Some("9001".into()),
            url: "https://github.com/example/app/pull/12#pullrequestreview-9001".into(),
            created_at: "2026-10-05T12:00:00Z".into(),
        }
    }

    #[test]
    fn shared_cloud_fixture_feed_uses_the_production_decoder() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/cloud-api.json")).unwrap();
        let feed: Feed = serde_json::from_value(fixture["github_notifications"].clone()).unwrap();
        assert_eq!(
            feed.github.unwrap().login.as_deref(),
            Some("fixture-contributor")
        );
        assert!(feed.notifications.iter().all(valid));
        assert_eq!(feed.notifications.len(), 2);
    }

    #[test]
    fn rows_outside_the_contract_are_dropped() {
        assert!(valid(&row()));
        for change in [
            |r: &mut Notification| r.kind = "deployed".into(),
            |r: &mut Notification| r.repository = "../etc".into(),
            |r: &mut Notification| r.url = "https://evil.example/".into(),
            |r: &mut Notification| r.target = Some("gist".into()),
            |r: &mut Notification| r.target_id = Some("1;2".into()),
        ] {
            let mut bad = row();
            change(&mut bad);
            assert!(!valid(&bad));
        }
    }

    #[test]
    fn subjects_are_batched_with_variables_and_missing_items_are_omitted() {
        let keys = vec![
            SubjectKey {
                repository: "org/app".into(),
                number: 5,
            },
            SubjectKey {
                repository: "org/gone".into(),
                number: 9,
            },
        ];
        let (query, variables) = subjects_query(&keys);
        assert!(query.starts_with("query($o0: String!, $n0: String!, $i0: Int!, $o1"));
        assert_eq!(variables["n0"], "app");
        assert_eq!(variables["i1"], 9);
        let data = json!({"s0": {"issueOrPullRequest": {"title": "Ship it", "state": "MERGED"}}, "s1": null});
        assert_eq!(
            subjects(&keys, &data),
            [Subject {
                repository: "org/app".into(),
                number: 5,
                title: "Ship it".into(),
                state: "MERGED".into()
            }]
        );
    }
}
