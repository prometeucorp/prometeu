//! Linux persistence and directory adapters for the portable workspace catalog.
use prometeu_core::{
    board::Project,
    workspaces::{Catalog, CatalogStore, WorkspaceFolders},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

/// The primary conversation's Store holds the root lease before this adapter is used.
pub struct FileCatalog(pub PathBuf);
impl CatalogStore for FileCatalog {
    fn load(&self) -> Result<Option<Catalog>, String> {
        let path = self.0.join("workspaces.json");
        match fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
            Ok(meta) if meta.len() > 8 * 1024 * 1024 => {
                return Err("workspace_catalog_invalid".into())
            }
            Ok(_) => {}
        }
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map(Some)
            .map_err(|_| "workspace_catalog_invalid".into())
    }
    fn save(&self, catalog: &Catalog) -> Result<(), String> {
        let bytes = serde_json::to_vec(catalog).map_err(|e| e.to_string())?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("workspace_catalog_invalid".into());
        }
        let temporary = self.0.join("workspaces.json.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(temporary, self.0.join("workspaces.json")).map_err(|e| e.to_string())?;
        fs::File::open(&self.0)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
}
pub struct LinuxFolders;
impl WorkspaceFolders for LinuxFolders {
    fn inspect(&self, path: &str) -> Result<Project, String> {
        let source = PathBuf::from(path);
        if !source.is_absolute() {
            return Err("workspace_folder_invalid".into());
        }
        let path = source
            .canonicalize()
            .map_err(|_| "workspace_folder_invalid")?;
        if !path.is_dir() {
            return Err("workspace_folder_invalid".into());
        }
        let text = path.to_str().ok_or("workspace_folder_invalid")?.to_owned();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&text)
            .to_owned();
        Ok(Project {
            id: text.clone(),
            path: text,
            name,
        })
    }
}
