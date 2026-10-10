//! Local delivery only. Conversation eligibility stays in the frontend's live-event tracker.
use crate::{i18n, lock::lock, AppState};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

const WINDOW: &str = "notification";
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 96.0;
static CURRENT: Mutex<Option<Notice>> = Mutex::new(None);
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Banner,
    Notch,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Soft,
    Digital,
    Bell,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notice {
    title: String,
    body: String,
    style: Style,
    sound: Option<Tone>,
    tab: Option<String>,
    open_label: String,
    close_label: String,
}

fn validate(notice: &Notice) -> Result<(), String> {
    if notice.title.is_empty()
        || notice.title.len() > 512
        || notice.body.len() > 2048
        || notice
            .tab
            .as_ref()
            .is_some_and(|tab| tab.is_empty() || tab.len() > 128)
        || notice.open_label.len() > 256
        || notice.close_label.len() > 256
    {
        return Err(i18n::t("err.notifications.invalid"));
    }
    Ok(())
}

fn check_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err(i18n::t("err.notifications.invalid"));
    }
    Ok(())
}

fn tab_exists(app: &tauri::AppHandle, tab: &str) -> bool {
    lock(&app.state::<AppState>().board)
        .workspaces
        .iter()
        .any(|workspace| {
            !workspace.archived
                && !workspace.cleaned
                && workspace.tabs.iter().any(|item| item.id == tab)
        })
}

pub fn open_tab(app: &tauri::AppHandle, tab: Option<&str>) -> Result<(), String> {
    if let Some(tab) = tab {
        if !tab_exists(app, tab) {
            return Ok(());
        }
        app.emit_to("main", "notification-open", tab)
            .map_err(i18n::io)?;
    }
    if let Some(main) = app.get_webview_window("main") {
        main.unminimize().map_err(i18n::io)?;
        main.show().map_err(i18n::io)?;
        main.set_focus().map_err(i18n::io)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn notification_permission(
    window: tauri::WebviewWindow,
    request: bool,
) -> Result<String, String> {
    check_main(&window)?;
    #[cfg(target_os = "macos")]
    return tauri::async_runtime::spawn_blocking(move || mac::permission(request))
        .await
        .map_err(i18n::io)?;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Ok(freedesktop::permission())
    }
}

#[tauri::command]
pub async fn notification_show(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    notice: Notice,
) -> Result<(), String> {
    check_main(&window)?;
    validate(&notice)?;
    if notice
        .tab
        .as_deref()
        .is_some_and(|tab| !tab_exists(&app, tab))
    {
        return Ok(());
    }
    let visual = match notice.style {
        Style::Banner => {
            #[cfg(target_os = "macos")]
            {
                let banner = notice.clone();
                tauri::async_runtime::spawn_blocking(move || mac::banner(&banner))
                    .await
                    .map_err(i18n::io)?
            }
            #[cfg(not(target_os = "macos"))]
            {
                let (handle, banner) = (app.clone(), notice.clone());
                tauri::async_runtime::spawn_blocking(move || freedesktop::banner(&handle, &banner))
                    .await
                    .map_err(i18n::io)?
            }
        }
        Style::Notch => {
            let handle = app.clone();
            let overlay_notice = notice.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let (tx, rx) = std::sync::mpsc::channel();
                let target = handle.clone();
                handle
                    .run_on_main_thread(move || {
                        let _ = tx.send(show_notch(&target, &overlay_notice));
                    })
                    .map_err(i18n::io)?;
                rx.recv().map_err(i18n::io)?
            })
            .await
            .map_err(i18n::io)?
        }
        Style::None => Ok(()),
    };
    // Denied banner permission must not disable the independently selected sound.
    let audio = if let Some(tone) = notice.sound {
        play(tone).await
    } else {
        Ok(())
    };
    visual.and(audio)
}

fn show_notch(app: &tauri::AppHandle, notice: &Notice) -> Result<(), String> {
    let overlay = if let Some(window) = app.get_webview_window(WINDOW) {
        window
    } else {
        tauri::WebviewWindowBuilder::new(
            app,
            WINDOW,
            tauri::WebviewUrl::App("notification.html".into()),
        )
        .title("Prometeu")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .resizable(false)
        .focused(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .visible(false)
        .shadow(false)
        .background_color(tauri::window::Color(12, 11, 10, 255))
        .build()
        .map_err(i18n::io)?
    };
    place(app, &overlay)?;
    *lock(&CURRENT) = Some(notice.clone());
    overlay.emit("notification", notice).map_err(i18n::io)?;
    #[cfg(target_os = "macos")]
    {
        // Tauri's show() makes the window key. Order it without activating the app instead.
        // show_notch always runs on the main thread and the Tauri window owns this pointer.
        let native = unsafe {
            &*overlay
                .ns_window()
                .map_err(i18n::io)?
                .cast::<objc2_app_kit::NSWindow>()
        };
        native.setLevel(objc2_app_kit::NSStatusWindowLevel);
        // Clip the native content too: CSS rounding alone leaves opaque square window corners.
        native.setOpaque(false);
        native.setBackgroundColor(Some(&objc2_app_kit::NSColor::clearColor()));
        if let Some(view) = native.contentView() {
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                use objc2_quartz_core::CACornerMask;
                layer.setCornerRadius(18.0);
                layer.setMaskedCorners(if layer.isGeometryFlipped() {
                    CACornerMask::LayerMinXMaxYCorner | CACornerMask::LayerMaxXMaxYCorner
                } else {
                    CACornerMask::LayerMinXMinYCorner | CACornerMask::LayerMaxXMinYCorner
                });
                layer.setMasksToBounds(true);
            }
        }
        native.orderFrontRegardless();
    }
    #[cfg(not(target_os = "macos"))]
    overlay.show().map_err(i18n::io)?;
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(8));
        // A previous notice cannot hide the one that replaced it.
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if GENERATION.load(std::sync::atomic::Ordering::SeqCst) == generation {
                let _ = hide(&handle);
            }
        });
    });
    Ok(())
}

fn place(app: &tauri::AppHandle, overlay: &tauri::WebviewWindow) -> Result<(), String> {
    let monitor = select_monitor(app, overlay)?;
    let scale = monitor.scale_factor();
    #[cfg(target_os = "macos")]
    let top = mac::top_offset(&monitor);
    #[cfg(not(target_os = "macos"))]
    let top = 0.0;
    let (x, y) = island_point(
        monitor.position().x,
        monitor.size().width,
        monitor.position().y,
        scale,
        top,
    );
    overlay
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(i18n::io)
}

// The island follows the display in use: the pointer's, then the main window's, then the primary.
fn select_monitor(
    app: &tauri::AppHandle,
    overlay: &tauri::WebviewWindow,
) -> Result<tauri::Monitor, String> {
    cursor_monitor(app)
        .or_else(|| {
            app.get_webview_window("main")
                .and_then(|main| main.current_monitor().ok().flatten())
        })
        .or_else(|| overlay.primary_monitor().ok().flatten())
        .ok_or_else(|| i18n::t("err.notifications.unavailable"))
}

// tao reports the cursor in physical points of the primary display while monitor frames
// mix per-display scales, so the lookup compares everything in logical space.
fn cursor_monitor(app: &tauri::AppHandle) -> Option<tauri::Monitor> {
    let point = app.cursor_position().ok()?;
    let scale = app.primary_monitor().ok().flatten()?.scale_factor();
    let (x, y) = (point.x / scale, point.y / scale);
    app.available_monitors().ok()?.into_iter().find(|monitor| {
        let scale = monitor.scale_factor();
        contains_logical(
            f64::from(monitor.position().x) / scale,
            f64::from(monitor.position().y) / scale,
            f64::from(monitor.size().width) / scale,
            f64::from(monitor.size().height) / scale,
            x,
            y,
        )
    })
}

fn contains_logical(mx: f64, my: f64, mw: f64, mh: f64, x: f64, y: f64) -> bool {
    x >= mx && x < mx + mw && y >= my && y < my + mh
}

fn island_point(
    physical_x: i32,
    physical_width: u32,
    physical_y: i32,
    scale: f64,
    top_offset: f64,
) -> (f64, f64) {
    let x = (f64::from(physical_x) + f64::from(physical_width) / 2.0) / scale - WIDTH / 2.0;
    let y = f64::from(physical_y) / scale + top_offset;
    (x, y)
}

#[tauri::command]
pub fn notification_current() -> Option<Notice> {
    lock(&CURRENT).clone()
}

fn hide(app: &tauri::AppHandle) -> Result<(), String> {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *lock(&CURRENT) = None;
    if let Some(window) = app.get_webview_window(WINDOW) {
        window.hide().map_err(i18n::io)?;
    }
    Ok(())
}

#[tauri::command]
pub fn notification_dismiss(app: tauri::AppHandle) -> Result<(), String> {
    hide(&app)
}

#[tauri::command]
pub fn notification_open(app: tauri::AppHandle) -> Result<(), String> {
    let current = lock(&CURRENT).clone();
    hide(&app)?;
    if let Some(notice) = current {
        open_tab(&app, notice.tab.as_deref())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn notification_sound(window: tauri::WebviewWindow, tone: Tone) -> Result<(), String> {
    check_main(&window)?;
    play(tone).await
}

async fn play(tone: Tone) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return tauri::async_runtime::spawn_blocking(move || {
        let name = match tone {
            Tone::Soft => "Pop",
            Tone::Digital => "Glass",
            Tone::Bell => "Ping",
        };
        let status = std::process::Command::new("/usr/bin/afplay")
            .arg(format!("/System/Library/Sounds/{name}.aiff"))
            .status()
            .map_err(i18n::io)?;
        if status.success() {
            Ok(())
        } else {
            Err(i18n::t("err.notifications.sound"))
        }
    })
    .await
    .map_err(i18n::io)?;
    #[cfg(not(target_os = "macos"))]
    return tauri::async_runtime::spawn_blocking(move || freedesktop::sound(tone))
        .await
        .map_err(i18n::io)?;
}

#[cfg(not(target_os = "macos"))]
mod freedesktop;
#[cfg(target_os = "macos")]
mod mac;

pub fn shutdown() {
    #[cfg(not(target_os = "macos"))]
    freedesktop::shutdown();
}

pub fn install(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    mac::install(app);
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    main.on_window_event(move |event| {
        if let tauri::WindowEvent::Moved(_) = event {
            if let Some(overlay) = handle.get_webview_window(WINDOW) {
                if overlay.is_visible().unwrap_or(false) {
                    let _ = place(&handle, &overlay);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_payload_is_bounded_and_enums_are_closed() {
        let json = serde_json::json!({ "title":"Done", "body":"Workspace", "style":"notch",
            "sound":null, "tab":"tab-1", "openLabel":"Open", "closeLabel":"Dismiss" });
        let mut notice: Notice = serde_json::from_value(json.clone()).unwrap();
        assert!(validate(&notice).is_ok());
        notice.body = "x".repeat(2049);
        assert!(validate(&notice).is_err());
        let mut invalid = json;
        invalid["sound"] = serde_json::json!("/tmp/arbitrary.aiff");
        assert!(serde_json::from_value::<Notice>(invalid).is_err());
    }

    #[test]
    fn island_centers_on_the_monitor_and_applies_the_top_offset() {
        assert_eq!(island_point(0, 3024, 0, 2.0, 0.0), (576.0, 0.0));
        assert_eq!(island_point(0, 3024, 0, 2.0, 25.0), (576.0, 25.0));
        assert_eq!(island_point(3024, 3840, 0, 2.0, 0.0), (2292.0, 0.0));
    }

    #[test]
    fn cursor_lookup_uses_each_monitor_logical_frame() {
        assert!(contains_logical(0.0, 0.0, 1512.0, 982.0, 1511.0, 500.0));
        assert!(!contains_logical(0.0, 0.0, 1512.0, 982.0, 1512.0, 500.0));
        assert!(contains_logical(1512.0, 0.0, 1920.0, 1080.0, 1512.0, 500.0));
    }
}
