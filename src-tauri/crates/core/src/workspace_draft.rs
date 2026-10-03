//! Existing desktop launcher input shared with other application hosts.
use crate::session::launch::Launch;
/// Keep launcher inputs together so adding an option does not expand the command's parameter list.
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub project: String,
    /// Additional repositories receive sibling worktrees on the same branch. This requires
    /// worktrees because the agent needs a common parent directory.
    #[serde(default)]
    pub extras: Vec<String>,
    /// An empty branch name means no branch should be created.
    pub branch: String,
    /// The starting ref for a new branch.
    pub base: String,
    /// None keeps older callers' branch-creation behavior.
    #[serde(default)]
    pub new_branch: Option<bool>,
    /// Selected local or remote ref when reusing a branch.
    #[serde(default)]
    pub source: Option<String>,
    /// Create the branch in a separate worktree when enabled; otherwise switch the original clone.
    pub worktree: bool,
    pub title: String,
    pub stage: String,
    pub prompt: String,
    pub inject: Vec<String>,
    /// The Linear issue that opened the launcher, when present.
    #[serde(default)]
    pub issue: Option<crate::domain::IssueRef>,
    /// The launcher's "Start with" skill as `<package>/<skill>`; empty means none. Older callers
    /// omit it (ADR 0057).
    #[serde(default)]
    pub kickoff: String,
    /// Model and effort persist on the workspace. Plan mode applies only to the initial
    /// conversation.
    #[serde(flatten)]
    pub launch: Launch,
}

/// Open with the kickoff line when there is one, then attach initial context through @path
/// mentions supported by the agent, then the person's prompt.
pub fn first_message(opening: Option<&str>, prompt: &str, inject: &[String]) -> Option<String> {
    let mentions = inject
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let path = p.trim();
            match path.chars().any(char::is_whitespace) {
                true => format!("@\"{path}\""),
                false => format!("@{path}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let parts: Vec<String> = [
        opening.unwrap_or_default().to_string(),
        mentions,
        prompt.trim().to_string(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_attachments_use_the_existing_mention_format_with_or_without_text() {
        let files = vec![
            "/mnt/c/spec ' ação.txt".into(),
            "src/lib.rs".into(),
            " ".into(),
        ];
        assert_eq!(
            first_message(None, "", &files).as_deref(),
            Some("@\"/mnt/c/spec ' ação.txt\" @src/lib.rs")
        );
        assert_eq!(
            first_message(Some("Use this skill."), "  Read these.  ", &files).as_deref(),
            Some("Use this skill.\n\n@\"/mnt/c/spec ' ação.txt\" @src/lib.rs\n\nRead these.")
        );
        assert_eq!(first_message(None, " ", &[]), None);
    }
}
