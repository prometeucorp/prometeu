//! The person's standalone to-do list. Tasks > To-dos and the built-in MCP reach it through
//! [`call`] only. The list stays on this computer, or on one remote machine that the person selects
//! explicitly: then its host keeps the list, its agents use the same list through the host MCP,
//! and this desktop never falls back to a local copy (`docs/contracts/todos.md`).
use crate::{mcp_access::Client, paths};
use prometeu_core::todos::{Status, Todo, TodoFile, TodoStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT: &str = "todos";

pub struct LocalFile(pub PathBuf);

impl TodoFile for LocalFile {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(&self.0) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&self, bytes: &[u8]) -> Result<(), String> {
        paths::write_private_bytes(&self.0, bytes)
    }
}

fn local() -> &'static TodoStore<LocalFile> {
    static STORE: OnceLock<TodoStore<LocalFile>> = OnceLock::new();
    STORE.get_or_init(|| TodoStore::new(LocalFile(paths::root().join("todos.json"))))
}

/// `<root>/todos-location.json`. A missing file means this computer.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Location {
    #[serde(default)]
    pub machine: Option<String>,
}

fn location_path() -> PathBuf {
    paths::root().join("todos-location.json")
}

fn location() -> Result<Location, String> {
    match std::fs::read(location_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "todos_location_unreadable".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Location::default()),
        Err(e) => Err(e.to_string()),
    }
}

pub enum Op<'a> {
    List,
    Create(&'a str),
    SetStatus(&'a str, Status),
}

/// The host of a remote machine, reached through its current attachment. Errors are stable codes:
/// `machine_unknown`, `machine_offline`, `host_update_required` or the code of the host.
pub trait Machines {
    fn request(&self, machine: &str, method: &str, params: Value) -> Result<Value, String>;
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run one operation on the list that `location` names. A remote list is never replaced by the
/// local one: when its machine is not reachable, the operation fails.
pub fn run<F: TodoFile>(
    location: &Location,
    local: &TodoStore<F>,
    machines: &dyn Machines,
    op: Op,
) -> Result<Value, String> {
    if let Op::Create(title) = op {
        // Refuse a bad title before it goes over the wire; the holder validates again.
        prometeu_core::todos::title(title)?;
    }
    let Some(machine) = &location.machine else {
        return match op {
            Op::List => Ok(json!({"items": local.read()?.items})),
            Op::Create(title) => Ok(json!(local.update(|list| list.create(title, now()))?)),
            Op::SetStatus(id, status) => Ok(json!(local.update(|list| list.set_status(
                id,
                status,
                now()
            ))?)),
        };
    };
    let (method, params) = match op {
        Op::List => ("todos_list", json!({})),
        Op::Create(title) => ("todos_create", json!({"title": title})),
        Op::SetStatus(id, status) => ("todos_set_status", json!({"id": id, "status": status})),
    };
    machines.request(machine, method, params)
}

struct Attached<'a>(&'a AppHandle);

impl Machines for Attached<'_> {
    fn request(&self, machine: &str, method: &str, params: Value) -> Result<Value, String> {
        let connection = self
            .0
            .state::<crate::remote::Service>()
            .get(machine)
            .ok_or("machine_unknown")?;
        if !connection.connected() {
            return Err("machine_offline".into());
        }
        connection
            .request(method, params)
            .map_err(|error| remote_error(method, error))
    }
}

fn remote_error(method: &str, error: String) -> String {
    if error == crate::i18n::t(crate::remote::connection::OFFLINE) {
        "machine_offline".into()
    } else if error == format!("unknown method {method}") {
        "host_update_required".into()
    } else {
        error
    }
}

static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// The host of `machine` reported a change of its list.
pub fn remote_changed(machine: &str) {
    if let Some(app) = APP.get() {
        if location().is_ok_and(|l| l.machine.as_deref() == Some(machine)) {
            let _ = app.emit(EVENT, ());
        }
    }
}

fn call(app: &AppHandle, op: Op) -> Result<Value, String> {
    let write = !matches!(op, Op::List);
    let result = run(&location()?, local(), &Attached(app), op)?;
    if write {
        let _ = app.emit(EVENT, ());
    }
    Ok(result)
}

/// Whether an MCP client may use the list. A conversation got the built-in MCP because the person
/// selected it for that conversation; an external client needs the explicit `--todos` grant.
pub fn allowed(client: &Client) -> bool {
    client.conversation.is_some() || client.todos
}

pub const TOOLS: &[&str] = &["list_todos", "create_todo", "set_todo_status"];

pub fn mcp(app: &AppHandle, client: &Client, name: &str, args: &Value) -> Result<Value, String> {
    if !allowed(client) {
        return Err("todos_not_allowed".into());
    }
    let text = |key: &str| args[key].as_str().ok_or("invalid_arguments");
    let op = match name {
        "list_todos" => Op::List,
        "create_todo" => Op::Create(text("title")?),
        "set_todo_status" => Op::SetStatus(text("id")?, Status::parse(text("status")?)?),
        _ => return Err("invalid_tool".into()),
    };
    call(app, op)
}

/// Stable codes become catalog messages for the interface.
fn localized(code: String) -> String {
    let key = code.split(':').next().unwrap_or_default();
    match key {
        "title_required"
        | "title_too_long"
        | "invalid_title"
        | "invalid_status"
        | "todo_not_found"
        | "todo_list_full"
        | "todos_not_here"
        | "todos_newer_version"
        | "todos_unreadable"
        | "todos_location_unreadable"
        | "machine_unknown"
        | "machine_offline"
        | "host_update_required"
        | "todos_leave_offline" => crate::i18n::t(&format!("todos.err.{key}")),
        _ => crate::i18n::io(code),
    }
}

#[derive(Serialize)]
pub struct TodosView {
    /// The machine that keeps the list, or null for this computer.
    pub machine: Option<String>,
    /// Null when the list could not be read; `error` tells why.
    pub items: Option<Vec<Todo>>,
    pub error: Option<String>,
}

fn view(app: &AppHandle) -> TodosView {
    let machine = location().ok().and_then(|l| l.machine);
    match call(app, Op::List).and_then(|value| {
        serde_json::from_value::<Vec<Todo>>(value["items"].clone()).map_err(|e| e.to_string())
    }) {
        Ok(items) => TodosView {
            machine,
            items: Some(items),
            error: None,
        },
        Err(error) => TodosView {
            machine,
            items: None,
            error: Some(localized(error)),
        },
    }
}

#[tauri::command(async)]
pub fn todos(app: AppHandle) -> TodosView {
    view(&app)
}

#[tauri::command(async)]
pub fn todo_create(app: AppHandle, title: String) -> Result<Todo, String> {
    let value = call(&app, Op::Create(&title)).map_err(localized)?;
    serde_json::from_value(value).map_err(crate::i18n::io)
}

#[tauri::command(async)]
pub fn todo_set_status(app: AppHandle, id: String, status: String) -> Result<Todo, String> {
    let status = Status::parse(&status).map_err(localized)?;
    let value = call(&app, Op::SetStatus(&id, status)).map_err(localized)?;
    serde_json::from_value(value).map_err(crate::i18n::io)
}

/// Select where the list lives. The selected host starts to serve its agents, and the host that
/// kept the list before stops. A machine that is not connected cannot be selected or left, so
/// two hosts never serve the list for this desktop at the same time.
#[tauri::command(async)]
pub fn todos_locate(app: AppHandle, machine: Option<String>) -> Result<TodosView, String> {
    locate(&Attached(&app), machine).map_err(localized)?;
    let _ = app.emit(EVENT, ());
    Ok(view(&app))
}

fn locate(machines: &dyn Machines, machine: Option<String>) -> Result<(), String> {
    static LOCATING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = crate::lock::lock(&LOCATING);
    let current = location()?;
    let next = Location { machine };
    switch(&current, &next, machines)?;
    paths::write_private(
        &location_path(),
        &serde_json::to_string(&next).map_err(|e| e.to_string())?,
    )
}

/// Enable the next host before the current location is saved; disable the previous one first so
/// that a failure leaves at most the previous host serving.
pub fn switch(current: &Location, next: &Location, machines: &dyn Machines) -> Result<(), String> {
    let enable = |machine: &str, enabled: bool| {
        machines.request(machine, "todos_enable", json!({"enabled": enabled}))
    };
    if let Some(machine) = &next.machine {
        // Fail early, before the previous host stops serving.
        machines
            .request(machine, "todos_list", json!({}))
            .or_else(|e| {
                if e == "todos_not_here" {
                    Ok(json!({}))
                } else {
                    Err(e)
                }
            })?;
    }
    if let Some(previous) = current
        .machine
        .as_deref()
        .filter(|m| next.machine.as_deref() != Some(*m))
    {
        match enable(previous, false) {
            Ok(_) => {}
            // A removed machine keeps no list for this desktop.
            Err(e) if e == "machine_unknown" => {}
            Err(e) if e == "machine_offline" => return Err("todos_leave_offline".into()),
            Err(e) => return Err(e),
        }
    }
    if let Some(machine) = &next.machine {
        if let Err(error) = enable(machine, true) {
            if let Some(previous) = current.machine.as_deref().filter(|m| *m != machine) {
                let _ = enable(previous, true);
            }
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// One file that several stores can open, as two processes would.
    #[derive(Default, Clone)]
    struct Memory(Arc<Mutex<Option<Vec<u8>>>>);

    impl TodoFile for Memory {
        fn load(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, bytes: &[u8]) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(bytes.to_vec());
            Ok(())
        }
    }

    /// A machine whose host keeps its own list, as `prometeu-host` does, and that can go offline.
    #[derive(Default)]
    struct Fake {
        online: Mutex<bool>,
        enabled: Mutex<bool>,
        list: Memory,
        calls: Mutex<Vec<String>>,
    }

    impl Machines for Fake {
        fn request(&self, machine: &str, method: &str, params: Value) -> Result<Value, String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{machine} {method}"));
            if machine != "vm" {
                return Err("machine_unknown".into());
            }
            if !*self.online.lock().unwrap() {
                return Err("machine_offline".into());
            }
            if method == "todos_enable" {
                *self.enabled.lock().unwrap() = params["enabled"].as_bool().unwrap();
                return Ok(json!({}));
            }
            if !*self.enabled.lock().unwrap() {
                return Err("todos_not_here".into());
            }
            let store = TodoStore::new(self.list.clone());
            let op = match method {
                "todos_list" => Op::List,
                "todos_create" => Op::Create(params["title"].as_str().unwrap()),
                "todos_set_status" => Op::SetStatus(
                    params["id"].as_str().unwrap(),
                    Status::parse(params["status"].as_str().unwrap())?,
                ),
                _ => unreachable!(),
            };
            run(&Location::default(), &store, self, op)
        }
    }

    fn here() -> Location {
        Location::default()
    }

    fn vm() -> Location {
        Location {
            machine: Some("vm".into()),
        }
    }

    #[test]
    fn this_computer_keeps_its_list_without_any_machine() {
        let store = TodoStore::new(Memory::default());
        let machines = Fake::default();
        let todo = run(&here(), &store, &machines, Op::Create("Local")).unwrap();
        assert_eq!(todo["status"], "todo");
        let id = todo["id"].as_str().unwrap();
        run(&here(), &store, &machines, Op::SetStatus(id, Status::Doing)).unwrap();
        let list = run(&here(), &store, &machines, Op::List).unwrap();
        assert_eq!(list["items"][0]["status"], "doing");
        assert!(machines.calls.lock().unwrap().is_empty());
        assert_eq!(
            run(&here(), &store, &machines, Op::Create("  ")).unwrap_err(),
            "title_required"
        );
    }

    #[test]
    fn a_remote_list_is_the_host_list_and_never_falls_back_to_this_computer() {
        let local = TodoStore::new(Memory::default());
        let machines = Fake::default();
        *machines.online.lock().unwrap() = true;
        // An unselected host refuses, so nothing is written to a list nobody sees.
        assert_eq!(
            run(&vm(), &local, &machines, Op::Create("x")).unwrap_err(),
            "todos_not_here"
        );
        switch(&here(), &vm(), &machines).unwrap();
        // What an agent on the machine added through the host MCP.
        let host = TodoStore::new(machines.list.clone());
        let id = host
            .update(|list| list.create("From the VM", 1))
            .unwrap()
            .id;
        let list = run(&vm(), &local, &machines, Op::List).unwrap();
        assert_eq!(list["items"][0]["id"], id.as_str());
        run(
            &vm(),
            &local,
            &machines,
            Op::SetStatus(&id, Status::Completed),
        )
        .unwrap();
        assert_eq!(host.read().unwrap().items[0].status, Status::Completed);
        *machines.online.lock().unwrap() = false;
        for op in [
            Op::List,
            Op::Create("offline"),
            Op::SetStatus(&id, Status::Todo),
        ] {
            assert_eq!(
                run(&vm(), &local, &machines, op).unwrap_err(),
                "machine_offline"
            );
        }
        assert!(local.read().unwrap().items.is_empty());
        // An offline host cannot be left: it would go on serving its agents.
        assert_eq!(
            switch(&vm(), &here(), &machines).unwrap_err(),
            "todos_leave_offline"
        );
        *machines.online.lock().unwrap() = true;
        switch(&vm(), &here(), &machines).unwrap();
        assert!(!*machines.enabled.lock().unwrap());
        // A removed machine does not block the way back.
        let gone = Location {
            machine: Some("gone".into()),
        };
        switch(&gone, &here(), &machines).unwrap();
        assert!(switch(&here(), &gone, &machines).is_err());
    }

    #[test]
    fn conversations_may_use_the_list_and_external_clients_need_the_grant() {
        let conversation = Client::conversation("tab");
        assert!(allowed(&conversation));
        let mut external = Client {
            id: "client:x".into(),
            conversation: None,
            projects: vec!["/repo".into()],
            todos: false,
        };
        assert!(!allowed(&external));
        external.todos = true;
        assert!(allowed(&external));
    }

    #[test]
    fn interface_errors_are_catalog_codes() {
        assert!(localized("machine_offline".into()).contains("todos.err.machine_offline"));
        assert!(localized("todos_unreadable: bad".into()).contains("todos.err.todos_unreadable"));
        assert!(localized("disk full".into()).contains("err.io"));
    }
}
