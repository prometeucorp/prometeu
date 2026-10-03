use super::*;
use crate::state::Project;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use prometeu_tools::catalog::CatalogProject;

pub(super) fn state(cache: &Cache, registered: &[Project]) -> Vec<CatalogProject> {
    let local: Vec<_> = if cache.doc.projects.is_empty()
        && cache
            .organizations
            .iter()
            .all(|org| org.doc.projects.is_empty())
    {
        Vec::new()
    } else {
        project_remotes(registered)
    };
    std::iter::once((
        None,
        None,
        cache.revision,
        &cache.doc,
        cache.links.as_ref().unwrap(),
    ))
    .chain(cache.organizations.iter().map(|org| {
        (
            Some(&org.id),
            Some(&org.name),
            org.revision,
            &org.doc,
            &org.links,
        )
    }))
    .flat_map(|(organization, name, revision, doc, links)| {
        let local = &local;
        doc.projects.iter().map(move |item| CatalogProject {
            item: item.clone(),
            organization: organization.cloned(),
            organization_name: name.cloned(),
            revision,
            local_path: links
                .get(&key("projects", &item.id))
                .filter(|path| {
                    registered.iter().any(|p| &p.path == *path) && Path::new(path).is_dir()
                })
                .cloned()
                .or_else(|| {
                    local
                        .iter()
                        .find(|(_, source)| same_remote(&item.source, source))
                        .map(|(project, _)| project.path.clone())
                }),
        })
    })
    .collect()
}

pub(super) fn validate(item: &Portable) -> Result<(), String> {
    if item.id.is_empty()
        || item.id.len() > 128
        || !item.id.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !item
            .id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        || item.note.len() > 2000
        || item.note.contains('\0')
    {
        return Err(invalid());
    }
    remote(&item.source).map(|_| ())
}

/// Accept only portable Git transports. Never pass paths, credentials or Git options to clone.
fn remote(source: &str) -> Result<String, String> {
    if source.is_empty()
        || source.len() > 4096
        || source
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '%'))
    {
        return Err(invalid());
    }
    let source = if let Some(rest) = source.strip_prefix("git@") {
        let (host, path) = rest.split_once(':').ok_or_else(invalid)?;
        if !host.starts_with(|c: char| c.is_ascii_alphanumeric())
            || !host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
        {
            return Err(invalid());
        }
        format!("ssh://git@{host}/{path}")
    } else if !source.contains("://") {
        let short = source.strip_prefix("github.com/").unwrap_or(source);
        let parts: Vec<_> = short.split('/').collect();
        if parts.len() != 2
            || parts.iter().any(|p| {
                !p.starts_with(|c: char| c.is_ascii_alphanumeric())
                    || !p
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            })
        {
            return Err(invalid());
        }
        format!("https://github.com/{short}")
    } else {
        source.into()
    };
    // Check raw segments before Url can normalize a traversal away.
    if source.split('/').any(|part| part == "." || part == "..") {
        return Err(invalid());
    }
    let url = reqwest::Url::parse(&source).map_err(|_| invalid())?;
    let path = url.path().strip_prefix('/').ok_or_else(invalid)?;
    if !matches!(url.scheme(), "https" | "ssh")
        || url.host_str().is_none()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (url.scheme() == "ssh" && url.username() != "git")
        || (url.scheme() == "https" && !url.username().is_empty())
        || !path.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_./-".contains(&c))
    {
        return Err(invalid());
    }
    Ok(url.to_string())
}

fn git() -> Command {
    let mut command = Command::new("git");
    command
        .args(["-c", "core.hooksPath=/dev/null"])
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https:ssh");
    command
}

fn same_remote(first: &str, second: &str) -> bool {
    let (Ok(first), Ok(second)) = (remote(first), remote(second)) else {
        return false;
    };
    first.strip_suffix(".git").unwrap_or(&first) == second.strip_suffix(".git").unwrap_or(&second)
}

fn project_remote(path: &str) -> Option<String> {
    let output = git()
        .arg("-C")
        .arg(path)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn project_remotes(projects: &[Project]) -> Vec<(Project, String)> {
    projects
        .iter()
        .filter_map(|project| project_remote(&project.path).map(|source| (project.clone(), source)))
        .collect()
}

fn matching_project(
    item: &Portable,
    remotes: &[(Project, String)],
    registered: &[Project],
) -> Option<Project> {
    remotes
        .iter()
        .filter(|(_, source)| same_remote(&item.source, source))
        .find_map(|(project, _)| {
            registered
                .iter()
                .find(|current| current.id == project.id && current.path == project.path)
                .cloned()
        })
}

/// Show Git diagnostics locally, without URL credentials, query strings or authorization headers.
fn diagnostic(cause: &str) -> String {
    cause
        .lines()
        .map(|line| {
            if line.to_ascii_lowercase().contains("authorization:") {
                return "[redacted]".to_string();
            }
            line.split_inclusive(char::is_whitespace)
                .map(|word| {
                    let value = word.trim_matches(|c: char| {
                        c.is_whitespace() || matches!(c, '\'' | '"' | '(' | ')')
                    });
                    if value.contains("://") {
                        if let Ok(mut url) = reqwest::Url::parse(value) {
                            let _ = url.set_username("");
                            let _ = url.set_password(None);
                            url.set_query(None);
                            url.set_fragment(None);
                            return word.replace(value, url.as_str());
                        }
                        return word.replace(value, "[redacted URL]");
                    }
                    word.to_string()
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(4096)
        .collect::<String>()
        .trim()
        .to_string()
}

fn clone_error(cause: &str) -> String {
    let lower = cause.to_ascii_lowercase();
    let code = if [
        "permission denied (publickey",
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "returned error: 401",
        "returned error: 403",
    ]
    .iter()
    .any(|message| lower.contains(message))
    {
        "err.project.access"
    } else if lower.contains("repository not found") || lower.contains("repository does not exist")
    {
        "err.project.unavailable"
    } else {
        "err.project.cloneDetail"
    };
    i18n::ta(code, &[("cause", diagnostic(cause))])
}

fn directory_error(error: std::io::Error) -> String {
    i18n::ta(
        "err.project.directoryDetail",
        &[("cause", diagnostic(&error.to_string()))],
    )
}

fn checkout(item: &Portable, directory: &Path, existing: bool) -> Result<PathBuf, String> {
    validate(item)?;
    let source = remote(&item.source)?;
    let directory = directory.canonicalize().map_err(directory_error)?;
    if !directory.is_dir() {
        return Err(i18n::t("err.project.directory"));
    }
    let destination = if existing {
        directory
    } else {
        directory.join(&item.id)
    };
    if destination.try_exists().map_err(directory_error)? {
        // A retry may reuse this repository, but must never overwrite another folder.
        let root = git()
            .arg("-C")
            .arg(&destination)
            .args(["rev-parse", "--show-toplevel"])
            .output();
        let origin = git()
            .arg("-C")
            .arg(&destination)
            .args(["config", "--get", "remote.origin.url"])
            .output();
        if let (Ok(root), Ok(origin)) = (root, origin) {
            if root.status.success() && origin.status.success() {
                let actual = PathBuf::from(String::from_utf8_lossy(&root.stdout).trim())
                    .canonicalize()
                    .ok();
                let expected = destination.canonicalize().ok();
                if actual.is_some()
                    && actual == expected
                    && same_remote(&source, String::from_utf8_lossy(&origin.stdout).trim())
                {
                    return Ok(expected.unwrap());
                }
            }
        }
        return Err(i18n::t("err.project.exists"));
    }
    // Reserve the destination and publish only a complete clone. A failed checkout cannot later
    // be mistaken for an existing repository and registered by a retry.
    std::fs::create_dir(&destination).map_err(directory_error)?;
    let staging = destination.with_file_name(format!(".prometeu-clone-{}", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::create_dir(&staging) {
        let _ = std::fs::remove_dir(&destination);
        return Err(directory_error(error));
    }
    let cloned = git()
        .args(["clone", "--"])
        .arg(&source)
        .arg(&staging)
        .output()
        .map_err(|error| clone_error(&format!("git: {error}")))
        .and_then(|output| {
            if output.status.success() {
                return Ok(());
            }
            let cause = String::from_utf8_lossy(&output.stderr);
            Err(if cause.trim().is_empty() {
                clone_error(&output.status.to_string())
            } else {
                clone_error(&cause)
            })
        })
        .and_then(|()| {
            std::fs::rename(&staging, &destination).map_err(|error| clone_error(&error.to_string()))
        });
    if let Err(error) = cloned {
        // Only this operation's temporary clone is removed. A nonempty destination is preserved.
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir(&destination);
        return Err(error);
    }
    Ok(destination)
}

#[tauri::command(async)]
pub fn catalog_install_project(
    app: AppHandle,
    organization: Option<String>,
    id: String,
    revision: Option<u64>,
    directory: String,
    existing: bool,
) -> Result<Project, String> {
    // Probe a snapshot before taking the catalog lock; slow paths must not block shared state.
    let (remotes, _sync) = loop {
        let registered = lock(&app.state::<AppState>().board).projects.clone();
        let remotes = project_remotes(&registered);
        let sync = guard();
        // Another installation may have registered a project while we probed or waited.
        if lock(&app.state::<AppState>().board)
            .projects
            .iter()
            .map(|p| (&p.id, &p.path))
            .eq(registered.iter().map(|p| (&p.id, &p.path)))
        {
            break (remotes, sync);
        }
    };
    let mut cache = load_cache();
    let (route, cached) = match organization.as_deref() {
        Some(id) => {
            let org = cache
                .organizations
                .iter()
                .find(|org| org.id == id)
                .ok_or_else(invalid)?;
            (format!("/api/organizations/{id}/catalog"), &org.doc)
        }
        None => ("/api/catalog".into(), &cache.doc),
    };
    // Membership, revision and source are checked again before cloning or linking.
    let (code, value) = cloud::api(Method::GET, &route, None, Duration::from_secs(12))?
        .ok_or_else(|| i18n::t("err.catalog.disconnected"))?;
    if code != 200 {
        return Err(i18n::t("err.cloud.network"));
    }
    let (current_revision, doc) = parse(&value)?;
    if revision != current_revision || cached != &doc {
        pull_locked(&app)?;
        return Err(conflict());
    }
    let item = doc
        .projects
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(invalid)?;
    let project = matching_project(
        item,
        &remotes,
        &lock(&app.state::<AppState>().board).projects,
    );
    if let Some(project) = project {
        let links = match organization {
            Some(id) => {
                &mut cache
                    .organizations
                    .iter_mut()
                    .find(|org| org.id == id)
                    .unwrap()
                    .links
            }
            None => cache.links.as_mut().unwrap(),
        };
        links.insert(key("projects", &id), project.path.clone());
        save_cache(&cache)?;
        let _ = app.emit("catalog", ());
        return Ok(project);
    }
    let path = checkout(item, Path::new(&directory), existing)?;
    let project =
        crate::session::register_project(&mut lock(&app.state::<AppState>().board), &path);
    publish(&app);
    crate::state::persist_now(&app).map_err(|_| i18n::t("err.cloud.storage"))?;
    let links = match organization {
        Some(id) => {
            &mut cache
                .organizations
                .iter_mut()
                .find(|org| org.id == id)
                .unwrap()
                .links
        }
        None => cache.links.as_mut().unwrap(),
    };
    links.insert(key("projects", &id), project.path.clone());
    save_cache(&cache)?;
    let _ = app.emit("catalog", ());
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Board;

    #[test]
    fn clone_errors_explain_access_without_hiding_git_diagnostics_or_exposing_url_secrets() {
        for (cause, code) in [
            (
                "git@example.test: Permission denied (publickey).",
                "err.project.access",
            ),
            ("fatal: Authentication failed", "err.project.access"),
            (
                "fatal: could not read Username: terminal prompts disabled",
                "err.project.access",
            ),
            (
                "fatal: The requested URL returned error: 403",
                "err.project.access",
            ),
            ("remote: Repository not found.", "err.project.unavailable"),
            (
                "fatal: Could not resolve host: example.test",
                "err.project.cloneDetail",
            ),
            (
                "fatal: could not create directory: Permission denied",
                "err.project.cloneDetail",
            ),
        ] {
            let error = clone_error(cause);
            let payload: Value =
                serde_json::from_str(error.strip_prefix("i18n:").unwrap()).unwrap();
            assert_eq!(payload["code"], code);
            assert_eq!(payload["args"]["cause"], cause);
        }
        let safe = diagnostic("fatal: Authentication failed for 'https://synthetic-user:synthetic-pass@example.test/app?secretquery=private#secretfragment'\nAuthorization: Bearer synthetic-header\n");
        assert!(safe.contains("Authentication failed"));
        assert!(safe.contains("https://example.test/app"));
        assert!(
            !diagnostic("fatal: url=https://user:synthetic-pass@example.test/app")
                .contains("synthetic-pass")
        );
        for secret in [
            "synthetic-user",
            "synthetic-pass",
            "secretquery",
            "secretfragment",
            "synthetic-header",
        ] {
            assert!(!safe.contains(secret));
        }
        assert_eq!(diagnostic(&"é".repeat(5000)).chars().count(), 4096);
        assert_eq!(
            diagnostic("fatal:\0 bad\r\nnetwork failure"),
            "fatal: bad\nnetwork failure"
        );
    }

    #[test]
    fn sources_match_cloud_contract_and_names_cannot_escape_destination() {
        let fixtures: Value =
            serde_json::from_str(include_str!("../../../fixtures/cloud-api.json")).unwrap();
        for source in fixtures["project_sources"]["valid"].as_array().unwrap() {
            assert!(remote(source.as_str().unwrap()).is_ok(), "{source}");
        }
        for source in fixtures["project_sources"]["invalid"].as_array().unwrap() {
            assert!(remote(source.as_str().unwrap()).is_err(), "{source}");
        }
        for id in ["../escape", "app/path", "-option", ".git", "", "a\0b"] {
            assert!(validate(&Portable {
                id: id.into(),
                source: "team/app".into(),
                note: String::new()
            })
            .is_err());
        }
    }

    #[test]
    fn clone_register_retry_and_link_preserve_files_without_running_setup() {
        // Isolate Git environment from parallel tests and all of the person's Git configuration.
        if std::env::var_os("PROMETEU_PROJECT_CLONE_TEST").is_none() {
            let root = std::env::temp_dir()
                .join(format!("prometeu-project-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let result = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "catalog::projects::tests::clone_register_retry_and_link_preserve_files_without_running_setup", "--nocapture"])
                .env("PROMETEU_PROJECT_CLONE_TEST", &root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_SSH_COMMAND", format!("sh '{}'", root.join("ssh-test").display()))
                .env("GIT_SSH_VARIANT", "ssh").output().unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let root = PathBuf::from(std::env::var_os("PROMETEU_PROJECT_CLONE_TEST").unwrap());
        let source = root.join("source");
        let git_ok = |args: &[&str]| {
            let result = Command::new("git").args(args).output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        };
        git_ok(&["init", source.to_str().unwrap()]);
        std::fs::write(source.join("README.md"), "fixture").unwrap();
        std::fs::create_dir(source.join(".prometeu")).unwrap();
        std::fs::write(
            source.join(".prometeu/settings.toml"),
            "[scripts]\nsetup = \"touch SETUP_RAN\"\n",
        )
        .unwrap();
        git_ok(&["-C", source.to_str().unwrap(), "add", "."]);
        git_ok(&[
            "-C",
            source.to_str().unwrap(),
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            // The fixture identity has no signing key; a contributor who signs every commit by
            // default would otherwise fail here.
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "fixture",
        ]);
        std::fs::write(
            root.join("ssh-test"),
            format!("exec git-upload-pack '{}'\n", source.display()),
        )
        .unwrap();
        let item = Portable {
            id: "app".into(),
            source: "git@example.test:team/app.git".into(),
            note: String::new(),
        };
        let path = checkout(&item, &root, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("README.md")).unwrap(),
            "fixture"
        );
        assert!(!path.join("SETUP_RAN").exists());
        let mut board = Board::default();
        crate::session::register_project(&mut board, &path);
        std::fs::write(path.join("README.md"), "local changes").unwrap();
        let retry = checkout(&item, &root, false).unwrap();
        crate::session::register_project(&mut board, &retry);
        assert_eq!(board.projects.len(), 1);
        assert_eq!(
            matching_project(&item, &project_remotes(&board.projects), &board.projects)
                .unwrap()
                .path,
            retry.display().to_string()
        );
        // A project removed during the probe must not be returned from the old snapshot.
        let snapshot = board.projects.clone();
        let remotes = project_remotes(&snapshot);
        assert!(matching_project(&item, &remotes, &[]).is_none());
        board.projects[0].name = "Renamed during probe".into();
        assert_eq!(
            matching_project(&item, &remotes, &board.projects)
                .unwrap()
                .name,
            "Renamed during probe"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("README.md")).unwrap(),
            "local changes"
        );
        assert_eq!(checkout(&item, &path, true).unwrap(), retry);
        std::fs::create_dir(path.join("subdir")).unwrap();
        assert!(checkout(&item, &path.join("subdir"), true).is_err());
        let different = Portable {
            source: "team/another".into(),
            ..item.clone()
        };
        assert!(checkout(&different, &root, false).is_err());
        let different = Portable {
            source: "git@example.test:team/app.git.git".into(),
            ..item.clone()
        };
        assert!(checkout(&different, &root, false).is_err());
        let mut cache = Cache {
            doc: Doc {
                projects: vec![item.clone()],
                ..Doc::default()
            },
            ..Cache::default()
        };
        assert_eq!(
            state(&cache, &board.projects)[0].local_path.as_deref(),
            retry.to_str()
        );
        cache
            .links
            .as_mut()
            .unwrap()
            .insert(key("projects", "app"), retry.display().to_string());
        assert!(state(&cache, &board.projects)[0].local_path.is_some());
        bind(
            &mut cache,
            &Doc::default(),
            [BTreeSet::new(), BTreeSet::new(), BTreeSet::new()],
        );
        assert!(cache.links.unwrap().is_empty());
        assert_eq!(board.projects.len(), 1);
        std::fs::write(
            root.join("ssh-test"),
            "echo 'Permission denied (publickey).' >&2\nexit 1\n",
        )
        .unwrap();
        let failed = Portable {
            id: "failed".into(),
            ..item
        };
        let error = checkout(&failed, &root, false).unwrap_err();
        assert!(error.contains("err.project.access"));
        assert!(error.contains("Permission denied (publickey)."));
        assert!(!root.join("failed").exists());
        assert_eq!(board.projects.len(), 1);
        assert_eq!(
            std::fs::read_to_string(path.join("README.md")).unwrap(),
            "local changes"
        );
    }
}
