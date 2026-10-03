//! Desktop composition of the shared local skill library and optional Cloud publication.
use crate::{paths, plugins};
pub use prometeu_tools::skills::{package_id, validate, Skill};
use prometeu_tools::skills::{SkillLibrary, SkillPackages, Skills};
use std::sync::Arc;
use tauri::AppHandle;

struct Packages;
impl SkillPackages for Packages {
    fn load(&self) -> Vec<plugins::Plugin> {
        plugins::load()
    }
    fn save(&self, plugin: plugins::Plugin) -> Result<(), String> {
        plugins::save_local(plugin).map(|_| ())
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        plugins::remove_hub(id).map(|_| ())
    }
}
fn library() -> SkillLibrary {
    SkillLibrary {
        root: paths::root(),
        files: Arc::new(plugins::PrivatePackageFiles),
        packages: Arc::new(Packages),
    }
}
pub fn load() -> Vec<Skill> {
    library().load()
}
pub fn save_local(skill: Skill) -> Result<Vec<Skill>, String> {
    library().save(skill)
}
pub fn remove_local(id: &str) -> Result<Vec<Skill>, String> {
    library().remove(id)
}

#[tauri::command(async)]
pub fn skill_hub() -> Vec<Skill> {
    let _sync = crate::catalog::guard();
    load()
}
#[tauri::command(async)]
pub fn skill_save(
    app: AppHandle,
    skill: Skill,
    revision: Option<u64>,
) -> Result<Vec<Skill>, String> {
    let _sync = crate::catalog::guard();
    library().check(&skill)?;
    crate::catalog::save_skill(&app, &skill, revision)?;
    save_local(skill)
}
#[tauri::command(async)]
pub fn skill_remove(_app: AppHandle, id: String) -> Result<Vec<Skill>, String> {
    let _sync = crate::catalog::guard();
    remove_local(&id)
}
