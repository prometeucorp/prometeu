//! Projects and workspaces on a remote machine (ADR 0101). Git runs there through the host; the
//! board entry is this app's, with `host` naming the machine.
use crate::state::{ConversationMode, Project, Repo, Workspace};
use prometeu_core::lock::lock;
use prometeu_core::workspace_draft::Draft;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

use super::connection::Connection;
use crate::AppState;

const GIT: Duration = Duration::from_secs(120);

/// Project IDs keep the machine apart from any local folder with the same path.
pub fn project_id(machine: &str, path: &str) -> String {
    format!("remote:{machine}:{path}")
}

fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("repo")
        .to_string()
}

fn absolute(path: &str) -> Result<String, String> {
    let path = path.trim();
    if !path.starts_with('/') || path.contains('\0') {
        return Err(crate::i18n::t("err.remote.path"));
    }
    let trimmed = path.trim_end_matches('/');
    Ok(if trimmed.is_empty() { "/" } else { trimmed }.to_string())
}

/// `host/owner/repo` for any spelling Git accepts for the same remote (HTTPS with or without
/// credentials, `ssh://` with a port, scp-like `git@host:owner/repo`, with or without `.git`),
/// so a local project and a clone on a machine can be recognized as one repository. Local paths
/// name no host and give `None`.
pub fn normalize_origin(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let (authority, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => {
            let (authority, path) = url.split_once(':')?;
            if authority.contains('/') {
                return None;
            }
            (authority, path)
        }
    };
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let path = path.trim_matches('/');
    (!host.is_empty() && !path.is_empty()).then(|| format!("{host}/{path}").to_lowercase())
}

/// The URL of a clone's `origin`, as Git reports it.
pub fn origin_url(connection: &Connection, path: &str) -> Option<String> {
    connection
        .exec(
            &["git", "-C", path, "remote", "get-url", "origin"],
            None,
            GIT,
        )
        .ok()
        .filter(|e| e.code == Some(0))
        .map(|e| e.stdout.trim().to_string())
        .filter(|url| !url.is_empty())
}

fn remote_origin(connection: &Connection, path: &str) -> Option<String> {
    normalize_origin(&origin_url(connection, path)?)
}

/// The URL of a local clone's `origin`.
pub fn local_origin_url(path: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", path, "remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!url.is_empty()).then_some(url)
}

/// Where a workspace for `draft.project` runs on the machine: a registered clone there, the clone
/// on the machine of a local project's repository (matched by origin), or a fresh clone of it
/// under `~/code` that preparation creates. A bare absolute path also works.
struct Target {
    path: String,
    clone: Option<String>,
    origin: Option<String>,
}

fn resolve(
    state: &State<AppState>,
    connection: &Connection,
    machine: &str,
    project: &str,
    home: &str,
) -> Result<Target, String> {
    let (remote, local) = {
        let board = lock(&state.board);
        let remote = board
            .projects
            .iter()
            .find(|p| p.id == project && p.host.as_deref() == Some(machine))
            .map(|p| p.path.clone());
        let local = board
            .projects
            .iter()
            .find(|p| p.id == project && p.host.is_none())
            .cloned();
        (remote, local)
    };
    if let Some(path) = remote {
        return Ok(Target {
            path: absolute(&path)?,
            clone: None,
            origin: None,
        });
    }
    let Some(local) = local else {
        return Ok(Target {
            path: absolute(project)?,
            clone: None,
            origin: None,
        });
    };
    let url = local_origin_url(&local.path).ok_or_else(|| {
        crate::i18n::ta("err.remote.noOrigin", &[("project", local.name.clone())])
    })?;
    let origin = normalize_origin(&url).ok_or_else(|| {
        crate::i18n::ta("err.remote.noOrigin", &[("project", local.name.clone())])
    })?;
    let known = lock(&state.board)
        .projects
        .iter()
        .find(|p| {
            p.host.as_deref() == Some(machine) && p.origin.as_deref() == Some(origin.as_str())
        })
        .map(|p| p.path.clone());
    if let Some(path) = known {
        return Ok(Target {
            path,
            clone: None,
            origin: Some(origin),
        });
    }
    let name = basename(&local.path);
    let path = format!("{home}/code/{name}");
    // A missing destination is the first workspace for this repository on the machine.
    if connection.exec(&["test", "-e", &path], None, GIT)?.code != Some(0) {
        return Ok(Target {
            path,
            clone: Some(url),
            origin: Some(origin),
        });
    }
    match toplevel(connection, &path)? {
        Some(top)
            if top == path
                && remote_origin(connection, &path).as_deref() == Some(origin.as_str()) =>
        {
            Ok(Target {
                path,
                clone: None,
                origin: Some(origin),
            })
        }
        _ => Err(crate::i18n::ta("err.remote.cloneTaken", &[("path", path)])),
    }
}

/// The repository root containing `path`, or `None` for a folder outside Git.
fn toplevel(connection: &Connection, path: &str) -> Result<Option<String>, String> {
    let exists = connection.exec(&["test", "-d", path], None, GIT)?;
    if exists.code != Some(0) {
        return Err(crate::i18n::ta(
            "err.remote.noFolder",
            &[("path", path.to_string())],
        ));
    }
    let top = connection.exec(
        &["git", "-C", path, "rev-parse", "--show-toplevel"],
        None,
        GIT,
    )?;
    Ok((top.code == Some(0)).then(|| top.stdout.trim().to_string()))
}

/// Register a folder on the machine, or clone `url` into it first.
#[tauri::command(async)]
pub fn remote_add_project(
    app: AppHandle,
    state: State<AppState>,
    machine: String,
    path: String,
    url: Option<String>,
) -> Result<Project, String> {
    let connection = app.state::<super::Service>().connected(&machine)?;
    let mut path = absolute(&path)?;
    if let Some(url) = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()) {
        if url.starts_with('-') {
            return Err(crate::i18n::t("err.remote.path"));
        }
        let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
        connection
            .exec(
                &["mkdir", "-p", if parent.is_empty() { "/" } else { parent }],
                None,
                GIT,
            )?
            .ok()?;
        connection
            .exec(
                &["git", "clone", "--", &url, &path],
                None,
                Duration::from_secs(600),
            )?
            .ok()?;
    }
    if let Some(top) = toplevel(&connection, &path)? {
        path = top;
    }
    let project = Project {
        id: project_id(&machine, &path),
        name: basename(&path),
        origin: remote_origin(&connection, &path),
        path,
        group: None,
        host: Some(machine),
    };
    let project = {
        let mut board = lock(&state.board);
        match board.projects.iter().find(|p| p.id == project.id) {
            Some(existing) => existing.clone(),
            None => {
                board.projects.push(project.clone());
                project
            }
        }
    };
    crate::state::publish(&app);
    Ok(project)
}

/// Folders under `path` on the machine, for choosing a project. Git repositories are marked.
#[tauri::command(async)]
pub fn remote_list_dir(
    app: AppHandle,
    machine: String,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    let connection = app.state::<super::Service>().connected(&machine)?;
    let home = connection.home().unwrap_or_else(|| "/".into());
    let path = match path.filter(|p| !p.trim().is_empty()) {
        Some(path) => absolute(&path)?,
        None => home.clone(),
    };
    let script = r#"cd "$1" || exit 3
for entry in *; do
  [ -d "$entry" ] || continue
  if [ -e "$entry/.git" ]; then echo "g $entry"; else echo "d $entry"; fi
done"#;
    let out = connection
        .exec(
            &["sh", "-c", script, "prometeu-list", &path],
            None,
            Duration::from_secs(20),
        )?
        .ok()?;
    let entries: Vec<serde_json::Value> = out
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(_, name)| *name != "." && *name != "..")
        .map(|(kind, name)| serde_json::json!({"name": name, "git": kind == "g"}))
        .collect();
    Ok(serde_json::json!({"path": path, "home": home, "entries": entries}))
}

fn default_base(connection: &Connection, repo: &str) -> String {
    connection
        .exec(
            &[
                "git",
                "-C",
                repo,
                "symbolic-ref",
                "--short",
                "refs/remotes/origin/HEAD",
            ],
            None,
            GIT,
        )
        .ok()
        .filter(|e| e.code == Some(0))
        .map(|e| e.stdout.trim().to_string())
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| "HEAD".into())
}

/// Publish a preparing card at once and create the checkout on the machine in the background,
/// like a local workspace. The first conversation always runs in terminal mode.
pub fn create(
    app: AppHandle,
    state: State<AppState>,
    mut draft: Draft,
    machine: &str,
) -> Result<Workspace, String> {
    if !crate::agents::capabilities(draft.launch.agent).remote_machines {
        return Err(crate::i18n::t("err.remote.provider"));
    }
    if !draft.extras.is_empty() {
        return Err(crate::i18n::t("err.remote.extras"));
    }
    let service = app.state::<super::Service>();
    let connection = service.connected(machine)?;
    let home = connection
        .home()
        .ok_or_else(|| crate::i18n::t(super::connection::OFFLINE))?;
    let target = resolve(&state, &connection, machine, &draft.project, &home)?;
    let repo_path = target.path.clone();
    // A clone preparation makes is a repository by definition.
    let top = match target.clone {
        Some(_) => Some(repo_path.clone()),
        None => toplevel(&connection, &repo_path)?,
    };
    if (draft.worktree || !draft.branch.trim().is_empty()) && top.is_none() {
        return Err(crate::i18n::ta(
            "err.session.notGit",
            &[("path", repo_path.clone())],
        ));
    }
    let repo_name = basename(&repo_path);
    draft.launch.conversation_mode = Some(ConversationMode::Terminal);
    let (root, branch) = match (draft.worktree, draft.branch.trim().is_empty()) {
        (true, true) => return Err(crate::i18n::t("err.session.worktreeNeedsBranch")),
        (true, false) => (
            format!(
                "{home}/{}/worktrees/{repo_name}/{}",
                super::install::remote_root(),
                crate::paths::dir_name(draft.branch.trim())
            ),
            draft.branch.trim().to_string(),
        ),
        (false, false) => (repo_path.clone(), draft.branch.trim().to_string()),
        (false, true) => {
            let head = connection
                .exec(
                    &["git", "-C", &repo_path, "branch", "--show-current"],
                    None,
                    GIT,
                )
                .ok()
                .filter(|e| e.code == Some(0))
                .map(|e| e.stdout.trim().to_string())
                .unwrap_or_default();
            (repo_path.clone(), head)
        }
    };
    if draft.base.trim().is_empty() && top.is_some() && target.clone.is_none() {
        draft.base = default_base(&connection, &repo_path);
    }
    let project = project_id(machine, &repo_path);
    let ws = Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        title: if draft.title.trim().is_empty() {
            branch.clone()
        } else {
            draft.title.clone()
        },
        project: project.clone(),
        repo: repo_path.clone(),
        repo_name: repo_name.clone(),
        branch: branch.clone(),
        worktree: root.clone(),
        repos: vec![Repo {
            path: repo_path.clone(),
            name: repo_name,
            worktree: root,
            base: draft.base.clone(),
            pr: None,
        }],
        archived: false,
        pinned: false,
        unread: false,
        model: draft.launch.model.clone(),
        effort: draft.launch.effort.clone(),
        agent: draft.launch.agent,
        port: None,
        issue: draft.issue.clone(),
        pr: None,
        cleaned: false,
        preserve_branches: if draft.worktree && draft.new_branch == Some(false) {
            vec![repo_path.clone()]
        } else {
            Vec::new()
        },
        shared: false,
        share_team: None,
        audience: None,
        remote_control: false,
        rights: None,
        confirm_messages: true,
        preparing: true,
        failed: None,
        mcp: None,
        plugins: None,
        skills: None,
        tabs: Vec::new(),
        active: None,
        host: Some(machine.to_string()),
    };
    {
        let mut board = lock(&state.board);
        if !board.projects.iter().any(|p| p.id == project) {
            board.projects.push(Project {
                id: project.clone(),
                name: ws.repo_name.clone(),
                path: repo_path,
                group: None,
                origin: target.origin.clone(),
                host: Some(machine.to_string()),
            });
        }
        board.workspaces.push(ws.clone());
    }
    crate::state::publish(&app);
    if ws.issue.is_none() {
        crate::naming::rename_later(&app, &ws.id, &draft.prompt, &ws.title, &draft.launch);
    }
    let (background, id) = (app.clone(), ws.id.clone());
    std::thread::spawn(move || {
        let result = prepare(&background, &connection, &id, draft, target.clone);
        if let Err(error) = result {
            let state = background.state::<AppState>();
            if let Some(ws) = lock(&state.board).workspace_mut(&id) {
                ws.preparing = false;
                ws.failed = Some(error);
            }
            crate::state::publish(&background);
        }
    });
    Ok(ws)
}

fn prepare(
    app: &AppHandle,
    connection: &Arc<Connection>,
    id: &str,
    mut draft: Draft,
    clone: Option<String>,
) -> Result<(), String> {
    if let Some(url) = clone {
        clone_for(app, connection, id, &mut draft, &url)?;
    }
    let draft = &draft;
    let (repo, root, branch) = {
        let state = app.state::<AppState>();
        let board = lock(&state.board);
        let ws = board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .ok_or_else(|| crate::i18n::t("err.session.noWorkspace"))?;
        (ws.repo.clone(), ws.worktree.clone(), ws.branch.clone())
    };
    if draft.worktree {
        let parent = root
            .rsplit_once('/')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default();
        connection
            .exec(&["mkdir", "-p", &parent], None, GIT)?
            .ok()?;
        let existing = draft.new_branch == Some(false);
        let mut argv = vec!["git", "-C", &repo, "worktree", "add"];
        if existing {
            argv.extend(["--", &root, &branch]);
        } else {
            argv.extend(["-b", &branch, "--", &root, &draft.base]);
        }
        connection.exec(&argv, None, GIT)?.ok()?;
    } else if !draft.branch.trim().is_empty() {
        let current = connection
            .exec(&["git", "-C", &repo, "branch", "--show-current"], None, GIT)?
            .ok()?;
        if current.trim() != branch {
            let mut argv = vec!["git", "-C", &repo, "switch"];
            if draft.new_branch == Some(false) {
                argv.push(&branch);
            } else {
                argv.extend(["-c", &branch, &draft.base]);
            }
            connection.exec(&argv, None, GIT)?.ok()?;
        }
    }
    crate::session::start_remote_conversation(app, id, draft)
}

/// Clone a local project's repository on the machine for its first workspace there, then fill in
/// what the launcher could not know before the clone existed: the default base and, when the
/// workspace uses the clone itself, its branch.
fn clone_for(
    app: &AppHandle,
    connection: &Connection,
    id: &str,
    draft: &mut Draft,
    url: &str,
) -> Result<(), String> {
    let repo = {
        let state = app.state::<AppState>();
        let board = lock(&state.board);
        board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.repo.clone())
            .ok_or_else(|| crate::i18n::t("err.session.noWorkspace"))?
    };
    let parent = repo.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
    connection.exec(&["mkdir", "-p", parent], None, GIT)?.ok()?;
    connection
        .exec(
            &["git", "clone", "--", url, &repo],
            None,
            Duration::from_secs(600),
        )?
        .ok()?;
    if draft.base.trim().is_empty() {
        draft.base = default_base(connection, &repo);
    }
    let head = connection
        .exec(&["git", "-C", &repo, "branch", "--show-current"], None, GIT)?
        .ok()?
        .trim()
        .to_string();
    let state = app.state::<AppState>();
    if let Some(ws) = lock(&state.board).workspace_mut(id) {
        if ws.branch.is_empty() {
            ws.branch = head;
        }
        for r in &mut ws.repos {
            if r.base.is_empty() {
                r.base = draft.base.clone();
            }
        }
    }
    crate::state::publish(app);
    Ok(())
}

/// Measure an archived remote checkout and apply the local cleanup rules to it: archived, not the
/// clone itself, no uncommitted files, and a branch merged into the default branch (or a merged
/// PR, or a branch the workspace did not create). A missing checkout passes, so cleanup can
/// reconcile the board.
pub fn inspect(
    connection: &Connection,
    ws: &Workspace,
) -> Result<(u64, Result<(), String>), String> {
    prometeu_git::cleanup::hard(ws)?;
    let primary = ws.primary();
    let script = r#"cd "$1" 2>/dev/null || { echo missing; exit 0; }
size=$(du -sk . 2>/dev/null | cut -f1)
dirty=$(git status --porcelain | wc -l)
target=$(git symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null || echo origin/main)
merged=0
git rev-parse -q --verify "$target^{commit}" >/dev/null && git merge-base --is-ancestor HEAD "$target" && merged=1
echo "${size:-0} $dirty $merged""#;
    let out = connection
        .exec(
            &["sh", "-c", script, "prometeu-cleanup", &primary.worktree],
            None,
            GIT,
        )?
        .ok()?;
    let fields: Vec<u64> = out
        .split_whitespace()
        .filter_map(|f| f.parse().ok())
        .collect();
    let [size, dirty, merged] = fields[..] else {
        return Ok((0, Ok(())));
    };
    let verdict = if dirty > 0 {
        Err(prometeu_core::error::with_args(
            "err.cleanup.dirty",
            &[("n", dirty.to_string())],
        ))
    } else if merged == 0
        && !ws.preserve_branches.contains(&primary.path)
        && !primary.pr.as_ref().is_some_and(|pr| pr.merged())
    {
        Err(prometeu_core::error::with_args(
            "err.cleanup.unmerged",
            &[("branch", ws.branch.clone())],
        ))
    } else {
        Ok(())
    };
    Ok((size, verdict))
}

/// Remove a remote workspace's checkout and branch after the same checks as a local cleanup;
/// `force` is the person's explicit approval to lose uncommitted or unmerged work. Only worktrees
/// this app created are removed; a workspace that used the clone itself keeps it.
pub fn cleanup(connection: &Connection, ws: &Workspace, force: bool) -> Result<(), String> {
    let (_, verdict) = inspect(connection, ws)?;
    if !force {
        verdict?;
    }
    let primary = ws.primary();
    connection
        .exec(
            &[
                "git",
                "-C",
                &primary.path,
                "worktree",
                "remove",
                "--force",
                "--",
                &primary.worktree,
            ],
            None,
            GIT,
        )?
        .ok()
        .or_else(|error| {
            // An already missing checkout only needs Git's record pruned.
            let gone = connection
                .exec(&["test", "-e", &primary.worktree], None, GIT)?
                .code
                != Some(0);
            if !gone {
                return Err(error);
            }
            connection
                .exec(
                    &["git", "-C", &primary.path, "worktree", "prune"],
                    None,
                    GIT,
                )?
                .ok()
        })?;
    if !ws.preserve_branches.contains(&primary.path) {
        let _ = connection.exec(
            &["git", "-C", &primary.path, "branch", "-D", "--", &ws.branch],
            None,
            GIT,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_one_remote_normalizes_alike() {
        for url in [
            "git@github.com:Owner/Repo.git",
            "https://github.com/owner/repo",
            "https://user:token@github.com/owner/repo.git/",
            "ssh://git@github.com:22/owner/repo.git",
        ] {
            assert_eq!(
                normalize_origin(url).as_deref(),
                Some("github.com/owner/repo"),
                "{url}"
            );
        }
        assert_eq!(normalize_origin("/srv/git/repo.git"), None);
        assert_eq!(normalize_origin("./relative"), None);
        assert_eq!(normalize_origin(""), None);
    }

    #[test]
    fn remote_projects_never_collide_with_local_paths() {
        assert_eq!(project_id("vps", "/home/me/app"), "remote:vps:/home/me/app");
        assert_eq!(basename("/home/me/app/"), "app");
    }

    #[test]
    fn only_absolute_machine_paths_are_accepted() {
        assert!(absolute("relative").is_err());
        assert_eq!(absolute("/srv/app/").unwrap(), "/srv/app");
        assert_eq!(absolute("/").unwrap(), "/");
    }
}
