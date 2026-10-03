//! Safely expose workspace files to the tree and viewer.

use super::cwd_of;
use crate::{i18n, AppState};
use std::path::Path;
use tauri::State;

pub use prometeu_core::files::Entry;
use prometeu_core::files::ProjectFiles;
pub use prometeu_files::save;
use prometeu_files::{inside, NativeFiles};

#[tauri::command]
pub fn list_dir(state: State<AppState>, id: String, rel: String) -> Vec<Entry> {
    cwd_of(&state, &id).map_or_else(Vec::new, |root| NativeFiles.list(&root, &rel))
}
#[tauri::command]
pub fn read_file(state: State<AppState>, id: String, rel: String) -> Result<String, String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    NativeFiles.read(&root, &rel)
}

/// Return raw bytes for non-text viewers such as PDF and CSV, with a larger limit than editable
/// code files.
#[tauri::command]
pub fn read_bytes(
    state: State<AppState>,
    id: String,
    rel: String,
) -> Result<tauri::ipc::Response, String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let bytes = NativeFiles.bytes(&root, &rel)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Use modification time and size to avoid rereading large files on every board event.
#[tauri::command]
pub fn file_stamp(state: State<AppState>, id: String, rel: String) -> Result<String, String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    NativeFiles.stamp(&root, &rel)
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
    state.repository_git.invalidate(&file);
    result
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
    let result = state.project_entries.create(&root, &rel, dir);
    state.repository_git.invalidate(&root);
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
    let result = state.project_entries.rename(&root, &from, &to);
    state.repository_git.invalidate(&root);
    result
}

/// Move to the system trash instead of deleting, so untracked work can still be recovered. Async
/// because the platform trash can be slow, notably through Finder on macOS.
#[tauri::command(async)]
pub fn trash_path(state: State<AppState>, id: String, rel: String) -> Result<(), String> {
    let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let result = state.project_entries.trash(&root, &rel);
    state.repository_git.invalidate(&root);
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
    use super::{inside, reveal_args};
    fn tmp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("prometeu-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
}
