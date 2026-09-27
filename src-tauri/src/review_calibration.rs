//! Opt-in, content-free review evidence. This store is separate from credentials and telemetry.
//! A persisted consent generation prevents a late launcher completion from undoing Clear/disable.

use crate::{i18n, paths};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static STORAGE: Mutex<()> = Mutex::new(());
type Result<T> = std::result::Result<T, String>;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    id: String,
    outcome: String,
    confidence: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    En,
    Pt,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    BusinessRule,
    ExpectedBehavior,
    Reproduction,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Answered,
    HandedToAgent,
    Dismissed,
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    v: u8,
    at: u64,
    model: Option<String>,
    language: Language,
    answers: Vec<Answer>,
    suggested: Vec<Topic>,
    action: Action,
    created: bool,
    latency_ms: u64,
}

fn valid_model(model: &str) -> bool {
    let Some((name, version)) = model.split_once('-') else {
        return false;
    };
    let parts: Vec<_> = version.split('.').collect();
    name == "jev"
        && parts.len() == 3
        && parts
            .iter()
            .all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
}

impl Record {
    fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        let topics: HashSet<_> = self.suggested.iter().collect();
        self.v == 1
            && self.at <= MAX_SAFE_INTEGER
            && self.latency_ms <= MAX_SAFE_INTEGER
            && self.model.as_deref().is_none_or(valid_model)
            && self.suggested.len() <= 2
            && topics.len() == self.suggested.len()
            && self.answers.len() <= 8
            && self.answers.iter().all(|a| {
                let outcomes: &[&str] = match a.id.as_str() {
                    "task_kind" => &["bug_fix", "feature", "investigation", "other"],
                    "business_rule" | "expected_behavior" | "reproduction" => &[
                        "present",
                        "ambiguous",
                        "absent",
                        "uninspected",
                        "not_applicable",
                    ],
                    "business_rule_resolver"
                    | "expected_behavior_resolver"
                    | "reproduction_resolver" => &["person", "agent", "unclear"],
                    "business_rule_kind" => &[
                        "existing_records",
                        "permissions",
                        "failure_handling",
                        "scope",
                        "other",
                    ],
                    _ => return false,
                };
                ids.insert(&a.id)
                    && outcomes.contains(&a.outcome.as_str())
                    && a.confidence.is_finite()
                    && (0.0..=1.0).contains(&a.confidence)
            })
    }
}

#[derive(Default, Deserialize, Serialize)]
struct Consent {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    generation: u64,
}

#[derive(Default, Serialize)]
pub struct Actions {
    answered: usize,
    handed_to_agent: usize,
    dismissed: usize,
    none: usize,
}

#[derive(Serialize)]
pub struct Status {
    enabled: bool,
    generation: u64,
    records: usize,
    created: usize,
    actions: Actions,
}

struct Store {
    root: PathBuf,
}

fn storage_error() -> String {
    i18n::t("err.calibration.storage")
}

impl Store {
    fn consent_path(&self) -> PathBuf {
        self.root.join("review-calibration.json")
    }
    fn records_path(&self) -> PathBuf {
        self.root.join("review-calibration.jsonl")
    }

    fn consent(&self) -> Result<Consent> {
        match std::fs::read(self.consent_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| storage_error()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Consent::default()),
            Err(_) => Err(storage_error()),
        }
    }

    fn records(&self) -> Result<Vec<Record>> {
        let file = match std::fs::File::open(self.records_path()) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => return Err(storage_error()),
        };
        std::io::BufReader::new(file)
            .lines()
            .map(|line| {
                let record: Record = serde_json::from_str(&line.map_err(|_| storage_error())?)
                    .map_err(|_| storage_error())?;
                if record.valid() {
                    Ok(record)
                } else {
                    Err(storage_error())
                }
            })
            .collect()
    }

    fn status(&self) -> Result<Status> {
        let consent = self.consent()?;
        let records = self.records()?;
        let mut actions = Actions::default();
        for record in &records {
            match record.action {
                Action::Answered => actions.answered += 1,
                Action::HandedToAgent => actions.handed_to_agent += 1,
                Action::Dismissed => actions.dismissed += 1,
                Action::None => actions.none += 1,
            }
        }
        Ok(Status {
            enabled: consent.enabled,
            generation: consent.generation,
            records: records.len(),
            created: records.iter().filter(|r| r.created).count(),
            actions,
        })
    }

    fn change_consent(&self, enabled: bool) -> Result<()> {
        let generation = self
            .consent()?
            .generation
            .checked_add(1)
            .filter(|n| *n <= MAX_SAFE_INTEGER)
            .ok_or_else(storage_error)?;
        let body = serde_json::to_string(&Consent {
            enabled,
            generation,
        })
        .map_err(|_| storage_error())?;
        paths::write_private(&self.consent_path(), &body).map_err(|_| storage_error())
    }

    fn append(&self, generation: u64, record: &Record) -> Result<()> {
        let consent = self.consent()?;
        if !consent.enabled || generation != consent.generation {
            return Ok(());
        }
        if !record.valid() {
            return Err(i18n::t("err.calibration.invalid"));
        }
        // Refuse to append to damaged history; Clear is the explicit recovery action.
        self.records()?;
        let mut body = serde_json::to_vec(record).map_err(|_| storage_error())?;
        body.push(b'\n');
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(self.records_path())
            .map_err(|_| storage_error())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| storage_error())?;
        }
        file.write_all(&body)
            .and_then(|()| file.sync_all())
            .map_err(|_| storage_error())
    }

    fn clear(&self) -> Result<()> {
        self.change_consent(self.consent()?.enabled)?;
        paths::write_private(&self.records_path(), "").map_err(|_| storage_error())
    }

    fn csv(&self) -> Result<String> {
        let mut csv =
            String::from("v,at,model,language,answers,suggested,action,created,latency_ms\r\n");
        for record in self.records()? {
            let value = serde_json::to_value(record).map_err(|_| storage_error())?;
            let fields = [
                "v",
                "at",
                "model",
                "language",
                "answers",
                "suggested",
                "action",
                "created",
                "latency_ms",
            ];
            let row: Vec<_> = fields
                .iter()
                .map(|field| {
                    let value = &value[field];
                    let text = if value.is_null() {
                        String::new()
                    } else {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    };
                    format!("\"{}\"", text.replace('"', "\"\""))
                })
                .collect();
            csv.push_str(&row.join(","));
            csv.push_str("\r\n");
        }
        Ok(csv)
    }
}

fn store() -> Store {
    Store {
        root: paths::root(),
    }
}

#[tauri::command(async)]
pub fn review_calibration_status() -> Result<Status> {
    let _guard = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    store().status()
}

#[tauri::command(async)]
pub fn review_calibration_set_enabled(enabled: bool) -> Result<Status> {
    let _guard = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let store = store();
    store.change_consent(enabled)?;
    store.status()
}

#[tauri::command(async)]
pub fn review_calibration_append(generation: u64, record: Record) -> Result<()> {
    let _guard = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    store().append(generation, &record)
}

#[tauri::command(async)]
pub fn review_calibration_clear() -> Result<Status> {
    let _guard = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let store = store();
    store.clear()?;
    store.status()
}

#[tauri::command(async)]
pub fn review_calibration_export(path: String) -> Result<()> {
    let _guard = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let csv = store().csv()?;
    write_export(Path::new(&path), &csv).map_err(|_| i18n::t("err.calibration.export"))
}

/// Exports are private files in a user-selected directory; never chmod that directory.
fn write_export(path: &Path, csv: &str) -> std::io::Result<()> {
    let invalid = || std::io::Error::other("invalid export destination");
    if path.extension().and_then(|value| value.to_str()) != Some("csv") {
        return Err(invalid());
    }
    let validate = || match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(invalid()),
    };
    validate()?;
    let temporary = path.with_file_name(format!(
        ".prometeu-calibration-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(csv.as_bytes())?;
        file.sync_all()?;
        validate()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Record {
        serde_json::from_value(serde_json::json!({"v":1,"at":1790000000000_u64,"model":"jev-1.13.0","language":"pt","answers":[{"id":"task_kind","outcome":"feature","confidence":0.91}],"suggested":["business_rule"],"action":"answered","created":true,"latency_ms":125})).unwrap()
    }
    fn temp() -> Store {
        Store {
            root: std::env::temp_dir()
                .join(format!("prometeu-calibration-{}", uuid::Uuid::new_v4())),
        }
    }

    #[test]
    fn recording_requires_separate_opt_in_and_private_files() {
        let store = temp();
        assert!(!store.status().unwrap().enabled);
        store.append(0, &fixture()).unwrap();
        assert!(!store.root.exists());
        store.change_consent(true).unwrap();
        store.append(1, &fixture()).unwrap();
        let summary = store.status().unwrap();
        assert_eq!(
            (summary.records, summary.created, summary.actions.answered),
            (1, 1, 1)
        );
        assert!(!store.root.join("typesafe.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for file in [store.consent_path(), store.records_path()] {
                assert_eq!(
                    std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            assert_eq!(
                std::fs::metadata(&store.root).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        std::fs::remove_dir_all(store.root).unwrap();
    }

    #[test]
    fn clear_and_disable_reject_late_records_even_after_reenable() {
        let store = temp();
        store.change_consent(true).unwrap();
        store.append(1, &fixture()).unwrap();
        store.clear().unwrap();
        store.append(1, &fixture()).unwrap();
        assert_eq!(store.status().unwrap().records, 0);
        store.change_consent(false).unwrap();
        store.change_consent(true).unwrap();
        store.append(2, &fixture()).unwrap();
        assert_eq!(store.status().unwrap().records, 0);
        store.append(4, &fixture()).unwrap();
        assert_eq!(store.status().unwrap().records, 1);
        std::fs::remove_dir_all(store.root).unwrap();
    }

    #[test]
    fn persisted_schema_cannot_carry_task_content() {
        let original = serde_json::to_value(fixture()).unwrap();
        for field in [
            "draft",
            "issue",
            "project",
            "repository",
            "key",
            "attachments",
            "path",
        ] {
            let mut value = original.clone();
            value[field] = serde_json::json!("private content");
            assert!(serde_json::from_value::<Record>(value).is_err());
        }
        for (field, value) in [("id", "private_project"), ("outcome", "secret text")] {
            let mut invalid = fixture();
            if field == "id" {
                invalid.answers[0].id = value.into();
            } else {
                invalid.answers[0].outcome = value.into();
            }
            assert!(!invalid.valid());
        }
        let mut invalid = fixture();
        invalid.model = Some("ts_live_secret_key_123".into());
        assert!(!invalid.valid());
        invalid.model = Some("confidentialclientname-1.2.3".into());
        assert!(!invalid.valid());
        invalid.model = Some("jev-123456789.1.0".into());
        assert!(!invalid.valid());
        let mut extra = original;
        extra["answers"][0]["text"] = serde_json::json!("private text");
        assert!(serde_json::from_value::<Record>(extra).is_err());
    }

    #[test]
    fn csv_contains_all_records_and_clear_recovers_damaged_history() {
        let store = temp();
        store.change_consent(true).unwrap();
        store.append(1, &fixture()).unwrap();
        let mut second = fixture();
        second.model = None;
        second.created = false;
        second.action = Action::None;
        store.append(1, &second).unwrap();
        let csv = store.csv().unwrap();
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.contains("\"\"id\"\":\"\"task_kind\"\""));
        assert!(csv.contains("\"false\",\"125\""));
        std::fs::write(store.records_path(), "corrupted").unwrap();
        assert!(store.append(1, &fixture()).is_err());
        assert!(store.status().is_err());
        store.clear().unwrap();
        assert_eq!(store.status().unwrap().records, 0);
        std::fs::remove_dir_all(store.root).unwrap();
    }

    #[test]
    fn export_preserves_the_parent_permissions_and_rejects_non_csv_destinations() {
        let store = temp();
        std::fs::create_dir_all(&store.root).unwrap();
        let destination = store.root.join("export.csv");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&store.root, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        write_export(&destination, "v,at\r\n1,12\r\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "v,at\r\n1,12\r\n"
        );
        assert!(write_export(&store.root.join("typesafe.json"), "private").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            assert_eq!(
                std::fs::metadata(&store.root).unwrap().permissions().mode() & 0o777,
                0o755
            );
            assert_eq!(
                std::fs::metadata(&destination)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            let link = store.root.join("link.csv");
            symlink(&destination, &link).unwrap();
            assert!(write_export(&link, "replaced").is_err());
            assert_eq!(
                std::fs::read_to_string(&destination).unwrap(),
                "v,at\r\n1,12\r\n"
            );
        }
        std::fs::remove_dir_all(store.root).unwrap();
    }
}
