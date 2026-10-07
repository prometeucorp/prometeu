//! Local delivery only. Conversation eligibility stays in the frontend's live-event tracker.
use crate::{i18n, island, lock::lock, AppState};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

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

pub(crate) fn check_main(window: &tauri::WebviewWindow) -> Result<(), String> {
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
        Style::Notch => show_notch(&app, &notice),
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

/// The island expands with the notice; a newer notice replaces it and its timer.
fn show_notch(app: &tauri::AppHandle, notice: &Notice) -> Result<(), String> {
    *lock(&CURRENT) = Some(notice.clone());
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    app.emit_to(island::WINDOW, "notification", notice)
        .map_err(i18n::io)?;
    island::schedule(app);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(8));
        // A previous notice cannot hide the one that replaced it.
        if GENERATION.load(std::sync::atomic::Ordering::SeqCst) == generation {
            let _ = hide(&app);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn notification_current() -> Option<Notice> {
    lock(&CURRENT).clone()
}

fn hide(app: &tauri::AppHandle) -> Result<(), String> {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *lock(&CURRENT) = None;
    app.emit_to(island::WINDOW, "notification", None::<Notice>)
        .map_err(i18n::io)?;
    island::schedule(app);
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
    #[cfg(not(target_os = "macos"))]
    let _ = app;
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
}
