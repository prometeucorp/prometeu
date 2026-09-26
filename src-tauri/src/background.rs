//! Observation of native window visibility/focus and power source. Consumers choose their own
//! refresh budget; this state never gates running agents or durable work.

use crate::lock::lock;
use serde::Serialize;
use std::sync::Mutex;
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

    fn observe_window(&mut self, visible: bool, focused: bool) -> bool {
        self.observe(visible, focused, self.power)
    }

    fn observe_power(&mut self, power: Power) -> bool {
        self.observe(self.visible, self.focused, power)
    }
}

type Waker = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct State {
    context: Mutex<Context>,
    wakers: Mutex<Vec<Waker>>,
}

impl State {
    pub fn snapshot(&self) -> Context {
        *lock(&self.context)
    }

    /// Native consumers that sleep until their own deadline are woken on every observed change,
    /// so a return to the foreground never depends on a webview round trip.
    pub fn on_change(&self, wake: impl Fn() + Send + Sync + 'static) {
        lock(&self.wakers).push(Box::new(wake));
    }

    /// Each writer changes only the fields it observed while holding the lock, so a window event
    /// and a power poll cannot overwrite each other with an older snapshot.
    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Context) -> bool) {
        let next = {
            let mut current = lock(&self.context);
            change(&mut current).then_some(*current)
        };
        if let Some(next) = next {
            for wake in lock(&self.wakers).iter() {
                wake();
            }
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
    let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    let focused = visible && window.is_focused().unwrap_or(false);
    app.state::<State>()
        .update(app, |context| context.observe_window(visible, focused));
}

/// Window callbacks provide prompt changes. The slow fallback covers hide/minimize paths without
/// a dedicated Tauri event, as well as power changes on platforms without a notification hook.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut ticks = 0u8;
        loop {
            // Read the window on the event-loop thread, in order with its window events.
            let poll = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(window) = poll.get_window("main") {
                    refresh_window(&window);
                }
            });
            if ticks == 0 {
                let power = power_source();
                app.state::<State>()
                    .update(&app, |context| context.observe_power(power));
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
    linux_power(std::path::Path::new("/sys/class/power_supply"))
}

/// Peripheral batteries (`scope` `Device`, such as a wireless mouse) never describe the computer's
/// power. Any online external supply, including USB-C and wireless chargers, means AC even while a
/// battery briefly reports discharging.
#[cfg(any(target_os = "linux", test))]
fn linux_power(root: &std::path::Path) -> Power {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Power::Unknown;
    };
    let read = |path: &std::path::Path, name: &str| {
        std::fs::read_to_string(path.join(name))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let (mut battery, mut external) = (false, false);
    for entry in entries.flatten() {
        let path = entry.path();
        if read(&path, "scope") == "Device" {
            continue;
        }
        match read(&path, "type").as_str() {
            "Battery" => battery = true,
            "Mains" | "Wireless" | "BrickID" => external |= read(&path, "online") == "1",
            kind if kind.starts_with("USB") => external |= read(&path, "online") == "1",
            _ => {}
        }
    }
    if external {
        Power::Ac
    } else if battery {
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
    fn a_writer_keeps_the_fields_it_did_not_observe() {
        let mut state = Context::default();
        state.observe(true, true, Power::Ac);
        assert!(state.observe_power(Power::Battery));
        assert_eq!((state.visible, state.focused), (true, true));
        assert!(state.observe_window(true, false));
        assert_eq!(state.power, Power::Battery);
        assert!(!state.observe_power(Power::Battery));
    }

    /// A supply directory name and its sysfs attributes.
    type Supply<'a> = (&'a str, &'a [(&'a str, &'a str)]);

    fn supplies(entries: &[Supply]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("prometeu-power-{}", uuid::Uuid::new_v4()));
        for (name, files) in entries {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            for (file, value) in *files {
                std::fs::write(dir.join(file), format!("{value}\n")).unwrap();
            }
        }
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn linux_power_ignores_peripheral_batteries_and_accepts_any_external_supply() {
        let mouse: Supply = (
            "hidpp_battery_0",
            &[
                ("type", "Battery"),
                ("scope", "Device"),
                ("status", "Discharging"),
            ],
        );
        let cases: [(&[Supply], Power); 5] = [
            (
                &[
                    ("AC", &[("type", "Mains"), ("online", "1")]),
                    ("BAT0", &[("type", "Battery"), ("status", "Charging")]),
                    mouse,
                ],
                Power::Ac,
            ),
            (
                &[
                    (
                        "ucsi-source-psy-USBC000:001",
                        &[("type", "USB"), ("online", "1")],
                    ),
                    ("BAT0", &[("type", "Battery"), ("status", "Discharging")]),
                ],
                Power::Ac,
            ),
            (
                &[
                    ("AC", &[("type", "Mains"), ("online", "0")]),
                    ("BAT0", &[("type", "Battery"), ("status", "Discharging")]),
                    mouse,
                ],
                Power::Battery,
            ),
            (&[mouse], Power::Unknown),
            (&[], Power::Unknown),
        ];
        for (entries, expected) in cases {
            let root = supplies(entries);
            assert_eq!(linux_power(&root), expected, "{entries:?}");
            let _ = std::fs::remove_dir_all(root);
        }
        assert_eq!(
            linux_power(std::path::Path::new("/nonexistent-power-supply")),
            Power::Unknown
        );
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
