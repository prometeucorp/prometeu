//! The standalone to-do list: a title and one of three states, with no link to a project,
//! workspace, issue or provider. The desktop keeps one list for this computer and each remote
//! host keeps one for its machine; the caller supplies the file through [`TodoFile`].
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub const VERSION: u32 = 1;
/// Characters, after trimming.
pub const MAX_TITLE: usize = 200;
pub const MAX_ITEMS: usize = 1000;
/// The largest serialized `items` array that a new to-do can make. A status change can add at
/// most [`STATUS_GROWTH`] bytes for each item, so a full list stays below [`RESPONSE_LIMIT`].
pub const MAX_LIST_BYTES: usize = 512 * 1024;
/// `"completed"` is five bytes longer than `"todo"`.
pub const STATUS_GROWTH: usize = 5;
/// The smallest response limit of the MCP servers that return the list: the desktop bridge.
pub const RESPONSE_LIMIT: usize = 768 * 1024;
// A full list after every possible status change, with room for the envelope of a response.
const _: () = assert!(MAX_LIST_BYTES + MAX_ITEMS * STATUS_GROWTH + 1024 < RESPONSE_LIMIT);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Todo,
    Doing,
    Completed,
}

impl Status {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "todo" => Ok(Self::Todo),
            "doing" => Ok(Self::Doing),
            "completed" => Ok(Self::Completed),
            _ => Err("invalid_status".into()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Todo {
    /// A UUID that never changes and is never reused.
    pub id: String,
    pub title: String,
    pub status: Status,
    /// Unix seconds.
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct TodoList {
    #[serde(default)]
    pub version: u32,
    /// Only a remote host reads it: agents on the machine reach the list only while a desktop
    /// selected this machine to keep it. The desktop's own list ignores it.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub items: Vec<Todo>,
}

/// One line of text, trimmed, with 1 to [`MAX_TITLE`] characters.
pub fn title(raw: &str) -> Result<String, String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err("title_required".into());
    }
    if title.chars().any(char::is_control) {
        return Err("invalid_title".into());
    }
    if title.chars().count() > MAX_TITLE {
        return Err("title_too_long".into());
    }
    Ok(title.to_string())
}

impl TodoList {
    pub fn create(&mut self, raw: &str, now: u64) -> Result<Todo, String> {
        let title = title(raw)?;
        if self.items.len() >= MAX_ITEMS {
            return Err("todo_list_full".into());
        }
        let todo = Todo {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            status: Status::Todo,
            created_at: now,
            updated_at: now,
        };
        self.items.push(todo.clone());
        // Titles of multibyte characters fill the bytes before the count of items.
        if serde_json::to_vec(&self.items).map_or(true, |bytes| bytes.len() > MAX_LIST_BYTES) {
            self.items.pop();
            return Err("todo_list_full".into());
        }
        Ok(todo)
    }

    /// Setting the current state again changes nothing, so a retried call is harmless.
    pub fn set_status(&mut self, id: &str, status: Status, now: u64) -> Result<Todo, String> {
        let todo = self
            .items
            .iter_mut()
            .find(|todo| todo.id == id)
            .ok_or("todo_not_found")?;
        if todo.status != status {
            todo.status = status;
            todo.updated_at = now;
        }
        Ok(todo.clone())
    }
}

/// The storage port. `save` must replace the file atomically.
pub trait TodoFile: Send + Sync {
    /// `None` when the file does not exist yet.
    fn load(&self) -> Result<Option<Vec<u8>>, String>;
    fn save(&self, bytes: &[u8]) -> Result<(), String>;
}

/// Serializes every read-modify-write of one list inside its process. Only one process owns each
/// file: the desktop app for its list and the host daemon for the machine's list.
pub struct TodoStore<F: TodoFile> {
    file: F,
    lock: Mutex<()>,
}

impl<F: TodoFile> TodoStore<F> {
    pub fn new(file: F) -> Self {
        Self {
            file,
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<TodoList, String> {
        let Some(bytes) = self.file.load()? else {
            return Ok(TodoList::default());
        };
        let list: TodoList =
            serde_json::from_slice(&bytes).map_err(|e| format!("todos_unreadable: {e}"))?;
        // A newer format may hold fields this version would drop on its next write.
        if list.version > VERSION {
            return Err("todos_newer_version".into());
        }
        Ok(list)
    }

    pub fn read(&self) -> Result<TodoList, String> {
        let _guard = crate::lock::lock(&self.lock);
        self.load()
    }

    /// Apply `change` and persist the result; nothing is written when it fails.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut TodoList) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = crate::lock::lock(&self.lock);
        let mut list = self.load()?;
        let result = change(&mut list)?;
        list.version = VERSION;
        let bytes = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
        self.file.save(&bytes)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Memory(Mutex<Option<Vec<u8>>>);

    impl TodoFile for Arc<Memory> {
        fn load(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(crate::lock::lock(&self.0).clone())
        }
        fn save(&self, bytes: &[u8]) -> Result<(), String> {
            *crate::lock::lock(&self.0) = Some(bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn titles_are_one_trimmed_line_of_bounded_length() {
        assert_eq!(title("  Ship it  ").unwrap(), "Ship it");
        assert_eq!(title("   ").unwrap_err(), "title_required");
        assert_eq!(title("").unwrap_err(), "title_required");
        assert_eq!(title("one\ntwo").unwrap_err(), "invalid_title");
        assert_eq!(title("tab\there").unwrap_err(), "invalid_title");
        assert!(title(&"é".repeat(MAX_TITLE)).is_ok());
        assert_eq!(
            title(&"x".repeat(MAX_TITLE + 1)).unwrap_err(),
            "title_too_long"
        );
        assert_eq!(Status::parse("done").unwrap_err(), "invalid_status");
        assert_eq!(Status::parse("doing").unwrap(), Status::Doing);
    }

    #[test]
    fn items_start_as_todo_keep_their_id_and_survive_a_reload() {
        let memory = Arc::new(Memory::default());
        let store = TodoStore::new(memory.clone());
        let created = store.update(|list| list.create("Write docs", 10)).unwrap();
        assert_eq!(created.status, Status::Todo);
        let doing = store
            .update(|list| list.set_status(&created.id, Status::Doing, 20))
            .unwrap();
        assert_eq!(
            (doing.id.as_str(), doing.updated_at),
            (created.id.as_str(), 20)
        );
        let same = store
            .update(|list| list.set_status(&created.id, Status::Doing, 30))
            .unwrap();
        assert_eq!(same.updated_at, 20);
        assert_eq!(
            store
                .update(|list| list.set_status("missing", Status::Completed, 40))
                .unwrap_err(),
            "todo_not_found"
        );
        let reloaded = TodoStore::new(memory).read().unwrap();
        assert_eq!(reloaded.version, VERSION);
        assert_eq!(reloaded.items, vec![doing]);
        let saved: serde_json::Value = serde_json::to_value(&reloaded.items[0]).unwrap();
        assert_eq!(saved["status"], "doing");
    }

    #[test]
    fn failed_changes_write_nothing_and_newer_files_stay_untouched() {
        let memory = Arc::new(Memory::default());
        let store = TodoStore::new(memory.clone());
        assert!(store.update(|list| list.create(" ", 1)).is_err());
        assert!(crate::lock::lock(&memory.0).is_none());
        let newer = br#"{"version":2,"items":[],"future":true}"#.to_vec();
        *crate::lock::lock(&memory.0) = Some(newer.clone());
        assert_eq!(store.read().unwrap_err(), "todos_newer_version");
        assert!(store.update(|list| list.create("x", 1)).is_err());
        assert_eq!(crate::lock::lock(&memory.0).clone().unwrap(), newer);
        *crate::lock::lock(&memory.0) = Some(b"{broken".to_vec());
        assert!(store.read().unwrap_err().starts_with("todos_unreadable"));
    }

    #[test]
    fn older_files_without_optional_fields_still_load() {
        let list: TodoList = serde_json::from_str(
            r#"{"items":[{"id":"a","title":"t","status":"completed","created_at":1,"updated_at":2,"future":1}]}"#,
        )
        .unwrap();
        assert!(!list.enabled);
        assert_eq!(list.items[0].status, Status::Completed);
    }

    #[test]
    fn the_list_is_bounded() {
        let mut list = TodoList::default();
        for n in 0..MAX_ITEMS {
            list.create(&format!("item {n}"), 1).unwrap();
        }
        assert_eq!(list.create("one more", 1).unwrap_err(), "todo_list_full");
    }

    #[test]
    fn every_list_that_create_accepts_fits_one_response() {
        // The largest titles in bytes: 200 four-byte characters each.
        let wide = "😀".repeat(MAX_TITLE);
        let mut list = TodoList::default();
        while list.create(&wide, u64::MAX).is_ok() {}
        assert!(list.items.len() < MAX_ITEMS);
        for todo in &mut list.items {
            todo.status = Status::Completed;
        }
        let response = serde_json::to_vec(&serde_json::json!({"items": list.items})).unwrap();
        assert!(response.len() <= MAX_LIST_BYTES + list.items.len() * STATUS_GROWTH + 16);
        assert!(response.len() < RESPONSE_LIMIT);
        assert_eq!(list.create("x", 1).unwrap_err(), "todo_list_full");
    }

    #[test]
    fn concurrent_writers_lose_no_update() {
        let store = Arc::new(TodoStore::new(Arc::new(Memory::default())));
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for m in 0..25 {
                        store
                            .update(|list| list.create(&format!("{n}-{m}"), 1))
                            .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let list = store.read().unwrap();
        assert_eq!(list.items.len(), 200);
        let ids: std::collections::HashSet<_> = list.items.iter().map(|t| &t.id).collect();
        assert_eq!(ids.len(), 200);
    }
}
