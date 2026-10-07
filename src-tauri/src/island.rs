//! Persistent notch island for the `notch` notification style. The main window owns the content
//! and pushes a bounded snapshot; this module relays it, keeps the overlay around the camera
//! housing and expands it while the pointer is over it or during a notice.
use crate::{i18n, lock::lock, notifications};
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use tauri::{Emitter, Manager};

pub const WINDOW: &str = "notification";
const EXPANDED_WIDTH: f64 = 560.0;
const MAX_HEIGHT: f64 = 640.0;
/// Room beside the camera housing for the flame and the counter.
const WING: f64 = 64.0;
const MAX_SNAPSHOT: usize = 512 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(false);
static SNAPSHOT: Mutex<Option<Value>> = Mutex::new(None);
static HOVER: Mutex<Hover> = Mutex::new(Hover {
    inside: false,
    latched: false,
});
static LAYOUT: Mutex<Layout> = Mutex::new(Layout {
    expanded: false,
    top: 32.0,
    height: 0.0,
});
static WATCH: Once = Once::new();

struct Hover {
    inside: bool,
    /// Set after opening a conversation so the panel stays closed, even during a notice, until
    /// the pointer leaves.
    latched: bool,
}

#[derive(Clone, Copy, Serialize)]
pub struct Layout {
    expanded: bool,
    /// Height of the camera housing or menu bar; expanded content starts below it.
    top: f64,
    /// Rendered height last reported by the overlay.
    #[serde(skip)]
    height: f64,
}

#[derive(Serialize)]
pub struct Current {
    snapshot: Option<Value>,
    layout: Layout,
    notice: Option<notifications::Notice>,
}

#[tauri::command]
pub fn island_enable(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<(), String> {
    notifications::check_main(&window)?;
    ENABLED.store(enabled, Ordering::SeqCst);
    if enabled {
        let handle = app.clone();
        WATCH.call_once(move || watch(handle));
    }
    schedule(&app);
    Ok(())
}

#[tauri::command]
pub fn island_update(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    snapshot: Value,
) -> Result<(), String> {
    notifications::check_main(&window)?;
    if serde_json::to_vec(&snapshot).map_err(i18n::io)?.len() > MAX_SNAPSHOT {
        return Err(i18n::t("err.notifications.invalid"));
    }
    app.emit_to(WINDOW, "island", &snapshot).map_err(i18n::io)?;
    *lock(&SNAPSHOT) = Some(snapshot);
    Ok(())
}

#[tauri::command]
pub fn island_current() -> Current {
    Current {
        snapshot: lock(&SNAPSHOT).clone(),
        layout: *lock(&LAYOUT),
        notice: notifications::notification_current(),
    }
}

/// The overlay reports its rendered height so the expanded panel fits its content.
#[tauri::command]
pub fn island_resize(app: tauri::AppHandle, height: f64) {
    if !height.is_finite() {
        return;
    }
    lock(&LAYOUT).height = height.clamp(0.0, MAX_HEIGHT);
    schedule(&app);
}

#[tauri::command]
pub fn island_open(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    if tab.is_empty() || tab.len() > 128 {
        return Err(i18n::t("err.notifications.invalid"));
    }
    lock(&HOVER).latched = true;
    schedule(&app);
    notifications::open_tab(&app, Some(&tab))
}

/// Recompute visibility, size and expansion on the main thread, where AppKit geometry lives.
pub fn schedule(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(error) = refresh(&handle) {
            eprintln!("island: {error}");
        }
    });
}

fn refresh(app: &tauri::AppHandle) -> Result<(), String> {
    let notice = notifications::notification_current().is_some();
    if !ENABLED.load(Ordering::SeqCst) && !notice {
        lock(&LAYOUT).expanded = false;
        if let Some(window) = app.get_webview_window(WINDOW) {
            window.hide().map_err(i18n::io)?;
        }
        return Ok(());
    }
    let overlay = overlay(app)?;
    let expanded = {
        let hover = lock(&HOVER);
        !hover.latched && (hover.inside || notice)
    };
    let height = lock(&LAYOUT).height;
    let top = arrange(app, &overlay, expanded, height)?;
    let (layout, changed) = {
        let mut layout = lock(&LAYOUT);
        let changed = layout.expanded != expanded || layout.top != top;
        layout.expanded = expanded;
        layout.top = top;
        (*layout, changed)
    };
    if changed {
        overlay.emit("island-layout", layout).map_err(i18n::io)?;
    }
    Ok(())
}

fn overlay(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(WINDOW) {
        return Ok(window);
    }
    let window = tauri::WebviewWindowBuilder::new(
        app,
        WINDOW,
        tauri::WebviewUrl::App("notification.html".into()),
    )
    .title("Prometeu")
    .inner_size(2.0 * WING + 48.0, 32.0)
    .decorations(false)
    .resizable(false)
    .focused(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible_on_all_workspaces(true)
    .visible(false)
    .shadow(false)
    // A click on the inactive overlay must reach its buttons instead of only activating the app.
    .accept_first_mouse(true)
    .background_color(tauri::window::Color(12, 11, 10, 255))
    .build()
    .map_err(i18n::io)?;
    #[cfg(target_os = "macos")]
    {
        let native = ns_window(&window)?;
        native.setLevel(objc2_app_kit::NSStatusWindowLevel);
        native.setOpaque(false);
        native.setBackgroundColor(Some(&objc2_app_kit::NSColor::clearColor()));
    }
    Ok(window)
}

#[cfg(target_os = "macos")]
fn ns_window(window: &tauri::WebviewWindow) -> Result<&objc2_app_kit::NSWindow, String> {
    // Called on the main thread; the Tauri window owns this pointer for the borrow's lifetime.
    Ok(unsafe {
        &*window
            .ns_window()
            .map_err(i18n::io)?
            .cast::<objc2_app_kit::NSWindow>()
    })
}

/// Size and order the overlay at the top center of the notched display, or of the main window's
/// display when none has a notch. Returns the housing height. Tauri's show() would make the
/// window key, so it is ordered without activating the app.
#[cfg(target_os = "macos")]
fn arrange(
    app: &tauri::AppHandle,
    overlay: &tauri::WebviewWindow,
    expanded: bool,
    content: f64,
) -> Result<f64, String> {
    use objc2::{runtime::NSObjectProtocol, sel, MainThreadMarker};
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let mtm = MainThreadMarker::new().ok_or_else(|| i18n::t("err.notifications.unavailable"))?;
    let native = ns_window(overlay)?;
    // safeAreaInsets and the auxiliary areas exist from macOS 12.
    let notched = |screen: &NSScreen| {
        screen.respondsToSelector(sel!(safeAreaInsets)) && screen.safeAreaInsets().top > 0.0
    };
    let screen = NSScreen::screens(mtm)
        .iter()
        .find(|screen| notched(screen))
        .or_else(|| {
            app.get_webview_window("main")
                .and_then(|main| ns_window(&main).ok().and_then(|main| main.screen()))
        })
        .or_else(|| NSScreen::mainScreen(mtm))
        .ok_or_else(|| i18n::t("err.notifications.unavailable"))?;
    let frame = screen.frame();
    let (top, camera) = if notched(&screen) {
        let sides =
            screen.auxiliaryTopLeftArea().size.width + screen.auxiliaryTopRightArea().size.width;
        (screen.safeAreaInsets().top, frame.size.width - sides)
    } else {
        let visible = screen.visibleFrame();
        let bar = frame.origin.y + frame.size.height - (visible.origin.y + visible.size.height);
        (if bar > 0.0 { bar } else { 24.0 }, 48.0)
    };
    let (width, height) = if expanded {
        (EXPANDED_WIDTH, content.max(top + 56.0).min(MAX_HEIGHT))
    } else {
        (camera + 2.0 * WING, top)
    };
    native.setFrame_display(
        NSRect::new(
            NSPoint::new(
                frame.origin.x + (frame.size.width - width) / 2.0,
                frame.origin.y + frame.size.height - height,
            ),
            NSSize::new(width, height),
        ),
        true,
    );
    // Clip the native content too: CSS rounding alone leaves opaque square window corners.
    if let Some(view) = native.contentView() {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            use objc2_quartz_core::CACornerMask;
            layer.setCornerRadius(if expanded { 18.0 } else { 10.0 });
            layer.setMaskedCorners(if layer.isGeometryFlipped() {
                CACornerMask::LayerMinXMaxYCorner | CACornerMask::LayerMaxXMaxYCorner
            } else {
                CACornerMask::LayerMinXMinYCorner | CACornerMask::LayerMaxXMinYCorner
            });
            layer.setMasksToBounds(true);
        }
    }
    native.orderFrontRegardless();
    Ok(top)
}

/// Other desktops have no camera housing; the compositor decides stacking.
#[cfg(not(target_os = "macos"))]
fn arrange(
    app: &tauri::AppHandle,
    overlay: &tauri::WebviewWindow,
    expanded: bool,
    content: f64,
) -> Result<f64, String> {
    const TOP: f64 = 30.0;
    let (width, height) = if expanded {
        (EXPANDED_WIDTH, content.clamp(TOP + 56.0, MAX_HEIGHT))
    } else {
        (2.0 * WING + 48.0, TOP)
    };
    let monitor = app
        .get_webview_window("main")
        .and_then(|main| main.current_monitor().ok().flatten())
        .or_else(|| overlay.primary_monitor().ok().flatten())
        .ok_or_else(|| i18n::t("err.notifications.unavailable"))?;
    let scale = monitor.scale_factor();
    overlay
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(i18n::io)?;
    overlay
        .set_position(tauri::LogicalPosition::new(
            (f64::from(monitor.position().x) + f64::from(monitor.size().width) / 2.0) / scale
                - width / 2.0,
            f64::from(monitor.position().y) / scale,
        ))
        .map_err(i18n::io)?;
    if !overlay.is_visible().unwrap_or(false) {
        overlay.show().map_err(i18n::io)?;
    }
    Ok(TOP)
}

/// WKWebView tracks the pointer only in the key window, so hover is sampled natively.
// ponytail: 10 Hz polling while the island is on; move to an NSTrackingArea if energy reports flag it.
fn watch(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut away = 3u8;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !ENABLED.load(Ordering::SeqCst) && notifications::notification_current().is_none() {
                continue;
            }
            let Some(overlay) = app.get_webview_window(WINDOW) else {
                continue;
            };
            let (Ok(cursor), Ok(origin), Ok(size)) = (
                app.cursor_position(),
                overlay.outer_position(),
                overlay.outer_size(),
            ) else {
                continue;
            };
            let hit = cursor.x >= f64::from(origin.x)
                && cursor.y >= f64::from(origin.y)
                && cursor.x < f64::from(origin.x) + f64::from(size.width)
                && cursor.y < f64::from(origin.y) + f64::from(size.height);
            // A short grace period keeps the panel open while the pointer crosses its edge.
            away = if hit { 0 } else { away.saturating_add(1) };
            let inside = away < 3;
            let changed = {
                let mut hover = lock(&HOVER);
                if !inside {
                    hover.latched = false;
                }
                let changed = hover.inside != inside;
                hover.inside = inside;
                changed
            };
            if changed {
                schedule(&app);
            }
        }
    });
}
