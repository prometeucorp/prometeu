//! Read Git state for the changes UI: branch, counters, and structured diffs. Workspace lifecycle
//! remains in the parent module.

use super::{git, head_branch, worktree_of};
use crate::AppState;
use std::path::Path;
use tauri::State;

pub use prometeu_core::git::FileChange;

#[tauri::command(async)]
pub fn workspace_branch(state: State<AppState>, id: String) -> Option<String> {
    head_branch(&worktree_of(&state, &id)?)
}

pub(super) fn ahead_of(worktree: &Path, base: &str) -> (String, u32) {
    let since = match base.is_empty() {
        true => String::new(),
        false => git(worktree, &["merge-base", base, "HEAD"])
            .trim()
            .to_string(),
    };
    match since.is_empty() {
        true => ("HEAD".to_string(), 0),
        false => {
            let count = git(
                worktree,
                &["rev-list", "--count", &format!("{since}..HEAD")],
            )
            .trim()
            .parse()
            .unwrap_or(0);
            (since, count)
        }
    }
}

pub(super) fn changes_in(worktree: &Path) -> Vec<FileChange> {
    changes_since(worktree, "HEAD")
}

fn changes_since(worktree: &Path, since: &str) -> Vec<FileChange> {
    let mut patches = patch_map(&git(
        worktree,
        &["diff", "--no-color", "--no-renames", "-U3", since],
    ));

    let mut out: Vec<FileChange> = git(worktree, &["diff", "--numstat", "--no-renames", since])
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let added = fields.next()?.parse().unwrap_or(0);
            let removed = fields.next()?.parse().unwrap_or(0);
            let path = fields.next()?.to_string();
            let patch = patches.remove(&path).unwrap_or_default();
            Some(FileChange {
                path,
                added,
                removed,
                new_file: patch.new,
                deleted: patch.deleted,
                dirty: false,
                patch: patch.body,
            })
        })
        .collect();

    for path in git(worktree, &["ls-files", "--others", "--exclude-standard"]).lines() {
        let text = std::fs::read_to_string(worktree.join(path)).unwrap_or_default();
        let added = text.lines().count() as u32;
        let patch = match added {
            0 => String::new(),
            count => std::iter::once(format!("@@ -0,0 +1,{count} @@"))
                .chain(text.lines().map(|line| format!("+{line}")))
                .collect::<Vec<_>>()
                .join("\n"),
        };
        out.push(FileChange {
            path: path.to_string(),
            added,
            removed: 0,
            new_file: true,
            deleted: false,
            dirty: true,
            patch: cap(patch),
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[derive(Default)]
pub(super) struct Patch {
    pub(super) body: String,
    pub(super) new: bool,
    pub(super) deleted: bool,
}

pub(super) fn patch_map(text: &str) -> std::collections::HashMap<String, Patch> {
    let mut out = std::collections::HashMap::new();
    let mut path = String::new();
    let mut old = String::new();
    let mut body: Vec<&str> = Vec::new();
    let mut flags = (false, false);
    let mut in_hunk = false;

    let mut flush = |path: &mut String, body: &mut Vec<&str>, flags: &mut (bool, bool)| {
        if !path.is_empty() {
            let (new, deleted) = std::mem::take(flags);
            out.insert(
                std::mem::take(path),
                Patch {
                    body: cap(body.join("\n")),
                    new,
                    deleted,
                },
            );
        }
        body.clear();
        *flags = (false, false);
    };

    for line in text.lines() {
        if line.starts_with("diff --git ") {
            flush(&mut path, &mut body, &mut flags);
            old.clear();
            in_hunk = false;
        } else if line.starts_with("new file mode") {
            flags.0 = true;
        } else if line.starts_with("deleted file mode") {
            flags.1 = true;
        } else if let Some(value) = line.strip_prefix("--- a/") {
            old = value.to_string();
        } else if let Some(value) = line.strip_prefix("+++ ") {
            path = match value.strip_prefix("b/") {
                Some(value) => value.to_string(),
                None => std::mem::take(&mut old),
            };
        } else if line.starts_with("@@") {
            in_hunk = true;
            body.push(line);
        } else if in_hunk {
            body.push(line);
        }
    }
    flush(&mut path, &mut body, &mut flags);
    out
}

fn cap(patch: String) -> String {
    match patch.len() > 400_000 {
        true => String::new(),
        false => patch,
    }
}
