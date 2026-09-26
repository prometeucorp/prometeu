//! Safely expose workspace files to the tree and viewer.

use super::cwd_of;
use crate::{i18n, AppState};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(serde::Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
}

fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(i18n::io)?;
    let real = root.join(rel).canonicalize().map_err(i18n::io)?;
    match real.starts_with(&root) {
        true => Ok(real),
        false => Err(i18n::t("err.session.outside")),
    }
}

#[tauri::command]
pub fn list_dir(state: State<AppState>, id: String, rel: String) -> Vec<Entry> {
    let Some(root) = cwd_of(&state, &id) else {
        return Vec::new();
    };
    let Ok(dir) = inside(&root, &rel) else {
        return Vec::new();
    };

    let mut out: Vec<Entry> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            let path = match rel.is_empty() {
                true => name.clone(),
                false => format!("{rel}/{name}"),
            };
            Some(Entry { name, path, dir })
        })
        .collect();

    out.sort_by_key(|entry| (!entry.dir, entry.name.to_lowercase()));
    out
}

/// Resolve files inside the worktree and enforce a size limit because the complete contents cross
/// IPC.
fn open(state: &State<AppState>, id: &str, rel: &str, limit: u64) -> Result<PathBuf, String> {
    let root = cwd_of(state, id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let file = inside(&root, rel)?;
    let meta = std::fs::metadata(&file).map_err(i18n::io)?;
    if meta.len() > limit {
        return Err(i18n::ta(
            "err.session.tooBig",
            &[("kb", (meta.len() / 1024).to_string())],
        ));
    }
    Ok(file)
}

#[tauri::command]
pub fn read_file(state: State<AppState>, id: String, rel: String) -> Result<String, String> {
    let file = open(&state, &id, &rel, 2 * 1024 * 1024)?;
    let bytes = std::fs::read(&file).map_err(i18n::io)?;
    String::from_utf8(bytes).map_err(|_| i18n::t("err.session.binary"))
}

/// Return raw bytes for non-text viewers such as PDF and CSV, with a larger limit than editable
/// code files.
#[tauri::command]
pub fn read_bytes(
    state: State<AppState>,
    id: String,
    rel: String,
) -> Result<tauri::ipc::Response, String> {
    let file = open(&state, &id, &rel, 100 * 1024 * 1024)?;
    let bytes = std::fs::read(&file).map_err(i18n::io)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Use modification time and size to avoid rereading large files on every board event.
#[tauri::command]
pub fn file_stamp(state: State<AppState>, id: String, rel: String) -> Result<String, String> {
    let file = open(&state, &id, &rel, u64::MAX)?;
    let meta = std::fs::metadata(&file).map_err(i18n::io)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .unwrap_or_default();
    Ok(format!(
        "{}.{}-{}",
        mtime.as_secs(),
        mtime.subsec_nanos(),
        meta.len()
    ))
}

/// Save only if disk contents still match the text originally opened. Reject conflicting agent
/// edits instead of overwriting their work.
pub fn save(file: &Path, text: &str, was: &str) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(i18n::io)?;
    let now = String::from_utf8(bytes).map_err(|_| i18n::t("err.session.binary"))?;
    if now != was {
        return Err(i18n::t("err.session.changed"));
    }
    std::fs::write(file, text).map_err(i18n::io)
}

#[tauri::command]
pub fn write_file(
    state: State<AppState>,
    id: String,
    rel: String,
    text: String,
    was: String,
) -> Result<(), String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let file = inside(&root, &rel)?;
    let result = save(&file, &text, &was);
    super::git::invalidate_path(&file);
    result
}

/// A new or renamed entry's name: one plain path component, never Git's own folder.
fn valid_name(name: &str) -> Result<&str, String> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || name.eq_ignore_ascii_case(".git");
    match bad {
        true => Err(i18n::t("err.files.name")),
        false => Ok(name),
    }
}

/// Git's own folder, at any depth: the tree never creates, renames or trashes inside repository
/// metadata, whether it is named in `rel` or reached through a symlinked parent.
fn git_metadata(path: &Path) -> bool {
    path.components().any(|part| {
        part.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".git")
    })
}

/// Resolve `rel` without following its last component, so a symlink is renamed or trashed itself
/// rather than its target. The parent must still resolve inside the root, and no component may be
/// `.git`, before or after resolving the parent's symlinks.
fn entry(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let refused = || i18n::t("err.files.name");
    if rel
        .split(['/', '\\'])
        .any(|part| part.eq_ignore_ascii_case(".git"))
    {
        return Err(refused());
    }
    let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let parent = inside(root, parent)?;
    let base = root.canonicalize().map_err(i18n::io)?;
    if git_metadata(parent.strip_prefix(&base).unwrap_or(&parent)) {
        return Err(refused());
    }
    Ok(parent.join(valid_name(name)?))
}

fn taken(path: &Path) -> Result<(), String> {
    match path.symlink_metadata().is_ok() {
        true => Err(i18n::ta(
            "err.files.exists",
            &[(
                "name",
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )],
        )),
        false => Ok(()),
    }
}

fn create(root: &Path, rel: &str, dir: bool) -> Result<(), String> {
    let path = entry(root, rel)?;
    taken(&path)?;
    match dir {
        true => std::fs::create_dir(&path),
        false => std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map(drop),
    }
    .map_err(i18n::io)
}

fn rename(root: &Path, from: &str, to: &str) -> Result<(), String> {
    let (source, target) = (entry(root, from)?, entry(root, to)?);
    source.symlink_metadata().map_err(i18n::io)?;
    if source == target {
        return Ok(());
    }
    let case_only = case_change(&source, &target);
    // On a case-insensitive disk (the macOS and Windows defaults) the new spelling already
    // "exists": it is the source itself. That is the only target allowed to exist.
    if !case_only || spelled(&target) {
        taken(&target)?;
    }
    if target.starts_with(&source) {
        return Err(i18n::t("err.files.name"));
    }
    if !case_only {
        return std::fs::rename(&source, &target).map_err(i18n::io);
    }
    // Some file systems ignore a rename that only changes case; a detour through a free name in the
    // same folder always lands on the new spelling, and is undone if the second step fails. A
    // rename replaces an existing target, so the detour must never be a name someone else uses:
    // each call draws its own random name, and case-only renames run one at a time so two detours
    // in this process cannot meet between the check and the rename.
    static DETOURS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = DETOURS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let detour = source.with_file_name(format!(".prometeu-rename-{}", uuid::Uuid::new_v4()));
    taken(&detour)?;
    std::fs::rename(&source, &detour).map_err(i18n::io)?;
    std::fs::rename(&detour, &target).map_err(|error| {
        let _ = std::fs::rename(&detour, &source);
        i18n::io(error)
    })
}

/// The same name in the same folder with only its case changed.
fn case_change(source: &Path, target: &Path) -> bool {
    source.parent() == target.parent()
        && source
            .file_name()
            .zip(target.file_name())
            .is_some_and(|(a, b)| {
                a != b && a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
            })
}

/// Whether the folder lists an entry spelled exactly like `path`'s name. Where a case-insensitive
/// lookup finds the source under the new spelling, the listing still shows only the old one, so
/// this tells a second, distinct entry (refused) from the source itself (allowed) on every system
/// and without following symlinks.
fn spelled(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| entry.file_name() == name)
}

fn trash_entry(root: &Path, rel: &str) -> Result<(), String> {
    let path = entry(root, rel)?;
    path.symlink_metadata().map_err(i18n::io)?;
    trash::delete(&path)
        .map_err(|error| i18n::ta("err.files.trash", &[("cause", error.to_string())]))
}

/// Tree actions accept a workspace or a project id, like `list_dir`, and paths relative to it.
#[tauri::command]
pub fn create_path(
    state: State<AppState>,
    id: String,
    rel: String,
    dir: bool,
) -> Result<(), String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let result = create(&root, &rel, dir);
    super::git::invalidate_path(&root);
    result
}

#[tauri::command]
pub fn rename_path(
    state: State<AppState>,
    id: String,
    from: String,
    to: String,
) -> Result<(), String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let result = rename(&root, &from, &to);
    super::git::invalidate_path(&root);
    result
}

/// Move to the system trash instead of deleting, so untracked work can still be recovered. Async
/// because the platform trash can be slow, notably through Finder on macOS.
#[tauri::command(async)]
pub fn trash_path(state: State<AppState>, id: String, rel: String) -> Result<(), String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let result = trash_entry(&root, &rel);
    super::git::invalidate_path(&root);
    result
}

/// Finder selects the entry with -R, so the person sees which file the tree meant. Systems served
/// by xdg-open have no selection flag; opening the file there would launch another application
/// over it, so open the folder holding it instead.
fn reveal_args(target: &Path, dir: bool, mac: bool) -> Vec<std::ffi::OsString> {
    let own = |path: &Path| path.as_os_str().to_os_string();
    if dir {
        return vec![own(target)];
    }
    match mac {
        true => vec![std::ffi::OsString::from("-R"), own(target)],
        false => vec![own(target.parent().unwrap_or(target))],
    }
}

/// Show a workspace or project entry in the system file manager. An empty relative path opens its
/// root; a file path selects or locates the entry.
#[tauri::command]
pub fn reveal_path(state: State<AppState>, id: String, rel: String) -> Result<(), String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let target = inside(&root, &rel)?;
    let ok = crate::platform::opener()
        .args(reveal_args(
            &target,
            target.is_dir(),
            cfg!(target_os = "macos"),
        ))
        .status()
        .map_err(i18n::io)?
        .success();
    ok.then_some(()).ok_or_else(|| {
        i18n::ta(
            "err.session.openFailed",
            &[("path", target.display().to_string())],
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{create, entry, inside, rename, reveal_args, save, spelled, trash_entry};

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("prometeu-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Save when disk contents still match the opened version.
    #[test]
    fn save_writes_when_disk_content_is_unchanged() {
        let dir = tmp("save");
        let file = dir.join("nota.md");
        std::fs::write(&file, "line one\nline two\n").unwrap();

        save(&file, "line one\n", "line one\nline two\n").unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "line one\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Preserve agent changes made while the person was editing by rejecting the stale save.
    #[test]
    fn save_rejects_concurrent_agent_writes() {
        let dir = tmp("race");
        let file = dir.join("nota.md");
        std::fs::write(&file, "o que o agente escreveu\n").unwrap();

        let err = save(&file, "o que eu escrevi\n", "o que eu abri\n").unwrap_err();

        assert_eq!(err, "i18n:{\"code\":\"err.session.changed\"}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "o que o agente escreveu\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Finder selects the file so the person sees which one the tree meant.
    #[test]
    fn reveal_selects_the_file_in_finder() {
        let file = std::path::Path::new("/wt/app/src/main.ts");

        assert_eq!(
            reveal_args(file, false, true),
            vec![
                std::ffi::OsString::from("-R"),
                std::ffi::OsString::from(file)
            ]
        );
    }

    /// xdg-open cannot select an entry, so open the folder holding the file instead of the file,
    /// which would launch another application over it.
    #[test]
    fn reveal_opens_the_holding_folder_where_selection_is_unavailable() {
        let file = std::path::Path::new("/wt/app/src/main.ts");

        assert_eq!(
            reveal_args(file, false, false),
            vec![std::ffi::OsString::from("/wt/app/src")]
        );
    }

    /// A folder is already the destination on either system.
    #[test]
    fn reveal_opens_a_folder_directly() {
        let dir = std::path::Path::new("/wt/app/src");

        assert_eq!(
            reveal_args(dir, true, true),
            vec![std::ffi::OsString::from(dir)]
        );
        assert_eq!(
            reveal_args(dir, true, false),
            vec![std::ffi::OsString::from(dir)]
        );
    }

    /// An empty relative path targets the workspace or project root, not a file to select.
    #[test]
    fn reveal_opens_root_for_empty_relative_path() {
        let root = tmp("reveal-root");
        let target = inside(&root, "").unwrap();
        let expected = root.canonicalize().unwrap();

        assert_eq!(target, expected);
        for mac in [true, false] {
            assert_eq!(
                reveal_args(&target, target.is_dir(), mac),
                vec![expected.as_os_str().to_os_string()]
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A symlink outside the worktree must not authorize writing there.
    #[test]
    fn inside_rejects_links_escaping_the_worktree() {
        let dir = tmp("outside");
        let (root, outside) = (dir.join("worktree"), dir.join("fora"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("atalho.txt")).unwrap();

        assert!(inside(&root, "atalho.txt").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_makes_files_and_folders_without_replacing_anything() {
        let dir = tmp("create");
        create(&dir, "notes.md", false).unwrap();
        create(&dir, "docs", true).unwrap();
        create(&dir, "docs/guide.md", false).unwrap();
        assert!(dir.join("notes.md").is_file() && dir.join("docs/guide.md").is_file());

        std::fs::write(dir.join("notes.md"), "kept").unwrap();
        assert!(create(&dir, "notes.md", false).is_err());
        assert!(create(&dir, "docs", true).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("notes.md")).unwrap(),
            "kept"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_stay_one_component_inside_the_root() {
        let dir = tmp("names");
        for bad in ["", ".", "..", "a/../../x", ".git", ".GIT", "../escape"] {
            assert!(create(&dir, bad, false).is_err(), "{bad:?}");
        }
        assert!(create(&dir, "missing/new.md", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Git metadata is off limits at any depth, named directly or through a symlinked folder.
    #[test]
    fn git_metadata_is_refused_at_any_depth() {
        let dir = tmp("git-meta");
        for repo in ["", "sub/"] {
            std::fs::create_dir_all(dir.join(format!("{repo}.git"))).unwrap();
            std::fs::write(dir.join(format!("{repo}.git/config")), "x").unwrap();
            std::fs::write(dir.join(format!("{repo}.git/HEAD")), "x").unwrap();
        }
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::os::unix::fs::symlink(dir.join(".git"), dir.join("meta")).unwrap();

        for bad in [
            ".git/config",
            "sub/.git/HEAD",
            "sub/.GIT/HEAD",
            "meta/config",
        ] {
            assert!(entry(&dir, bad).is_err(), "{bad:?}");
            assert!(
                create(&dir, &format!("{bad}.new"), false).is_err(),
                "{bad:?}"
            );
            assert!(rename(&dir, bad, "moved").is_err(), "{bad:?}");
            assert!(trash_entry(&dir, bad).is_err(), "{bad:?}");
        }
        assert!(rename(&dir, "a.md", ".git/x").is_err());
        assert!(rename(&dir, "a.md", "sub/.git/x").is_err());
        assert!(dir.join("a.md").is_file());
        assert!(dir.join(".git/config").is_file() && dir.join("sub/.git/HEAD").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Changing only the case keeps the entry, while a second entry spelled that way is refused.
    /// On a case-sensitive disk (Linux CI) `spelled` separates the two cases; on a case-insensitive
    /// one (macOS CI) the target lookup finds the source, so the test renames the spelling back.
    #[test]
    fn case_only_rename_changes_the_spelling_but_never_replaces_another_entry() {
        let dir = tmp("case");
        std::fs::write(dir.join("readme.md"), "r").unwrap();
        assert!(spelled(&dir.join("readme.md")));
        assert!(!spelled(&dir.join("README.md")));

        rename(&dir, "readme.md", "README.md").unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["README.md"]);
        assert_eq!(std::fs::read_to_string(dir.join("README.md")).unwrap(), "r");

        // Only a case-sensitive disk (Linux CI) can hold two entries that differ in case; on a
        // case-insensitive one (the macOS default) the old spelling still finds the renamed file,
        // so the same call is a case-only rename back.
        if dir.join("readme.md").exists() {
            rename(&dir, "README.md", "readme.md").unwrap();
            let names: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(names, ["readme.md"]);
            assert_eq!(std::fs::read_to_string(dir.join("readme.md")).unwrap(), "r");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        std::fs::write(dir.join("readme.md"), "other").unwrap();
        assert!(rename(&dir, "README.md", "readme.md").is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("readme.md")).unwrap(),
            "other"
        );
        std::os::unix::fs::symlink(dir.join("README.md"), dir.join("link.md")).unwrap();
        std::os::unix::fs::symlink(dir.join("README.md"), dir.join("LINK.md")).unwrap();
        assert!(rename(&dir, "link.md", "LINK.md").is_err());
        assert!(dir.join("link.md").symlink_metadata().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Case-only renames detour through a temporary name; concurrent ones in the same folder must
    /// never land on each other's detour and replace a file.
    #[test]
    fn concurrent_case_only_renames_keep_every_file() {
        let dir = tmp("case-race");
        let names: Vec<String> = (0..16).map(|n| format!("file{n}.md")).collect();
        for name in &names {
            std::fs::write(dir.join(name), name).unwrap();
        }
        std::thread::scope(|scope| {
            for name in &names {
                let dir = &dir;
                scope.spawn(move || rename(dir, name, &name.to_uppercase()).unwrap());
            }
        });
        let mut listed: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        listed.sort();
        let mut expected: Vec<_> = names.iter().map(|name| name.to_uppercase()).collect();
        expected.sort();
        assert_eq!(listed, expected);
        for name in &names {
            assert_eq!(
                std::fs::read_to_string(dir.join(name.to_uppercase())).unwrap(),
                *name
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_moves_entries_but_refuses_to_overwrite() {
        let dir = tmp("rename");
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::fs::write(dir.join("b.md"), "b").unwrap();
        std::fs::create_dir(dir.join("src")).unwrap();

        assert!(rename(&dir, "a.md", "b.md").is_err());
        assert!(rename(&dir, "src", "src/inner").is_err());
        rename(&dir, "a.md", "src/c.md").unwrap();
        rename(&dir, "src", "lib").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("lib/c.md")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(dir.join("b.md")).unwrap(), "b");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two links to one file stay distinct entries, so renaming one onto the other is refused.
    #[test]
    fn rename_keeps_a_second_link_to_the_same_file() {
        let dir = tmp("links");
        std::fs::write(dir.join("target.md"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("target.md"), dir.join("one.md")).unwrap();
        std::os::unix::fs::symlink(dir.join("target.md"), dir.join("two.md")).unwrap();
        assert!(rename(&dir, "one.md", "two.md").is_err());
        assert!(dir.join("one.md").symlink_metadata().is_ok());
        assert!(dir.join("two.md").symlink_metadata().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Acting on a link must touch the link, never the file it points to.
    #[test]
    fn entry_does_not_follow_the_last_symlink() {
        let dir = tmp("link");
        std::fs::write(dir.join("target.md"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("target.md"), dir.join("link.md")).unwrap();
        let root = dir.canonicalize().unwrap();
        assert_eq!(entry(&dir, "link.md").unwrap(), root.join("link.md"));
        rename(&dir, "link.md", "renamed.md").unwrap();
        assert!(dir
            .join("renamed.md")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(dir.join("target.md").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
