//! Observation of native window visibility/focus and power source. Consumers choose their own
//! refresh budget; this state never gates running agents or durable work.

use crate::lock::lock;
use serde::Serialize;
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Window};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Power {
    Ac,
    Battery,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Context {
    pub visible: bool,
    pub focused: bool,
    pub power: Power,
    pub revision: u64,
}

impl Context {
    fn observe(&mut self, visible: bool, focused: bool, power: Power) -> bool {
        if (self.visible, self.focused, self.power) == (visible, focused, power) {
            return false;
        }
        self.visible = visible;
        self.focused = focused;
        self.power = power;
        self.revision += 1;
        true
    }
}

#[derive(Default)]
pub struct State {
    context: Mutex<Context>,
    pub changed: Condvar,
}

impl State {
    pub fn snapshot(&self) -> Context {
        *lock(&self.context)
    }

    fn update(&self, app: &AppHandle, visible: bool, focused: bool, power: Power) {
        let next = {
            let mut current = lock(&self.context);
            current.observe(visible, focused, power).then_some(*current)
        };
        if let Some(next) = next {
            self.changed.notify_all();
            let _ = app.emit("background-context", next);
        }
    }
}

#[tauri::command]
pub fn background_context(state: tauri::State<State>) -> Context {
    state.snapshot()
}

pub fn refresh_window(window: &Window) {
    if window.label() != "main" {
        return;
    }
    let app = window.app_handle();
    let state = app.state::<State>();
    let previous = state.snapshot();
    let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    let focused = visible && window.is_focused().unwrap_or(false);
    state.update(app, visible, focused, previous.power);
}

/// Window callbacks provide prompt changes. The slow fallback covers hide/minimize paths without
/// a dedicated Tauri event, as well as power changes on platforms without a notification hook.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut ticks = 0u8;
        loop {
            if let Some(window) = app.get_window("main") {
                refresh_window(&window);
            }
            if ticks == 0 {
                let state = app.state::<State>();
                let old = state.snapshot();
                state.update(&app, old.visible, old.focused, power_source());
            }
            ticks = (ticks + 1) % 12;
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

#[cfg(target_os = "macos")]
fn power_source() -> Power {
    use std::ffi::{c_char, c_void, CStr};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        fn IOPSGetProvidingPowerSourceType(snapshot: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(
            value: *const c_void,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(value: *const c_void);
    }
    // SAFETY: IOKit returns a retained snapshot; the returned string is borrowed from it. Copy
    // its ASCII contents before releasing the snapshot. Null and conversion failure mean unknown.
    unsafe {
        let snapshot = IOPSCopyPowerSourcesInfo();
        if snapshot.is_null() {
            return Power::Unknown;
        }
        let source = IOPSGetProvidingPowerSourceType(snapshot);
        let mut buffer = [0 as c_char; 32];
        let result = if !source.is_null()
            && CFStringGetCString(source, buffer.as_mut_ptr(), buffer.len() as isize, 0x0600) != 0
        {
            match CStr::from_ptr(buffer.as_ptr()).to_bytes() {
                b"AC Power" => Power::Ac,
                b"Battery Power" => Power::Battery,
                _ => Power::Unknown,
            }
        } else {
            Power::Unknown
        };
        CFRelease(snapshot);
        result
    }
}

#[cfg(target_os = "linux")]
fn power_source() -> Power {
    let root = std::path::Path::new("/sys/class/power_supply");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Power::Unknown;
    };
    let mut found_battery = false;
    let mut found_ac = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() == "Battery" {
            found_battery = true;
            if std::fs::read_to_string(path.join("status"))
                .unwrap_or_default()
                .trim()
                == "Discharging"
            {
                return Power::Battery;
            }
        } else if kind.trim() == "Mains"
            && std::fs::read_to_string(path.join("online"))
                .unwrap_or_default()
                .trim()
                == "1"
        {
            found_ac = true;
        }
    }
    if found_ac {
        Power::Ac
    } else if found_battery {
        Power::Battery
    } else {
        Power::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_only_increment_revision_on_observed_change() {
        let mut state = Context::default();
        assert!(state.observe(true, true, Power::Ac));
        assert_eq!(state.revision, 1);
        assert!(!state.observe(true, true, Power::Ac));
        assert_eq!(state.revision, 1);
        assert!(state.observe(false, true, Power::Battery));
        assert!(!state.visible);
        assert_eq!(state.revision, 2);
    }

    #[test]
    fn unknown_power_uses_conservative_budget() {
        assert_eq!(serde_json::to_value(Power::Unknown).unwrap(), "unknown");
    }

    #[test]
    fn native_event_payload_matches_typed_frontend_context() {
        let mut state = Context::default();
        state.observe(true, true, Power::Battery);
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({
                "visible": true,
                "focused": true,
                "power": "battery",
                "revision": 1
            })
        );
    }
}
