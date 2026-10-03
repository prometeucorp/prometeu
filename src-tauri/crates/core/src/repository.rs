//! Shared branch selection rules over host-owned Git queries.
#[derive(serde::Serialize)]
pub struct Branches {
    pub all: Vec<String>,
    pub local: Vec<String>,
    pub default: String,
    /// Distinguish a non-Git folder from a newly initialized repository with no refs, so the
    /// launcher can enable valid controls.
    pub git: bool,
}

pub trait RepositoryReferences: Send + Sync {
    fn head(&self, path: &str) -> Result<Option<String>, String>;
    fn branches(&self, path: &str) -> Result<Branches, String>;
}
pub fn head_name(output: &str) -> Option<String> {
    let name = output.trim();
    (!name.is_empty() && name != "HEAD").then(|| name.to_string())
}
pub fn branches(query: impl Fn(&[&str]) -> String, git_repo: bool) -> Branches {
    let refs = |pattern: &str| -> Vec<String> {
        query(&[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            pattern,
        ])
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty() && !r.ends_with("/HEAD"))
        .map(str::to_string)
        .collect()
    };
    let locals = refs("refs/heads");
    let remotes = refs("refs/remotes");

    // Prefer the saved origin/HEAD, then conventional names, then the current branch.
    let head = query(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .trim()
        .to_string();
    let default = [head, "origin/main".to_string(), "origin/master".to_string()]
        .into_iter()
        .find(|r| !r.is_empty() && remotes.contains(r))
        .or_else(|| {
            let head = query(&["rev-parse", "--abbrev-ref", "HEAD"])
                .trim()
                .to_string();
            (!head.is_empty() && head != "HEAD").then_some(head)
        })
        .or_else(|| locals.first().cloned())
        .unwrap_or_default();

    // Put the selected base first, followed by local and remote branches.
    let mut all: Vec<String> = Vec::new();
    for name in [default.clone()]
        .into_iter()
        .chain(locals.iter().cloned())
        .chain(remotes)
    {
        if !name.is_empty() && !all.contains(&name) {
            all.push(name);
        }
    }
    Branches {
        all,
        local: locals,
        default,
        git: git_repo,
    }
}
