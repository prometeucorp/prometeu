//! UserNotifications supplies authorization and system banners; AppKit is not replaced or swizzled.
use super::Notice;
use crate::i18n;
use block2::{DynBlock, RcBlock};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
    AnyThread, DefinedClass, MainThreadMarker,
};
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSProcessInfo, NSString};
use objc2_user_notifications::*;
use std::{cell::RefCell, ptr::NonNull, sync::mpsc, time::Duration};

define_class!(
    // The delegate owns only a thread-safe AppHandle; Cocoa may call it on its private queue.
    #[unsafe(super = NSObject)]
    #[ivars = tauri::AppHandle]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}
    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn present(
            &self,
            _center: &UNUserNotificationCenter,
            _notice: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn respond(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            if response.actionIdentifier().to_string()
                == "com.apple.UNNotificationDefaultActionIdentifier"
            {
                let id = response.notification().request().identifier().to_string();
                let tab = id.strip_prefix("tab:").map(str::to_owned);
                let app = self.ivars().clone();
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    let _ = super::open_tab(&handle, tab.as_deref());
                });
            }
            completion.call(());
        }
    }
);

thread_local! {
    // UNUserNotificationCenter holds its delegate weakly.
    static DELEGATE: RefCell<Option<Retained<NotificationDelegate>>> = const { RefCell::new(None) };
}

fn available() -> bool {
    let bundle = NSBundle::mainBundle();
    // UserNotifications can abort an unbundled `tauri dev` process, even with an embedded plist.
    bundle.bundlePath().to_string().ends_with(".app") && bundle.bundleIdentifier().is_some()
}

pub fn top_offset(monitor: &tauri::Monitor) -> f64 {
    let scale = monitor.scale_factor();
    let x = f64::from(monitor.position().x) / scale;
    let y = f64::from(monitor.position().y) / scale;
    let width = f64::from(monitor.size().width) / scale;
    let height = f64::from(monitor.size().height) / scale;
    let Some(mtm) = MainThreadMarker::new() else {
        return 0.0;
    };
    let screens = NSScreen::screens(mtm);
    let Some(reference_height) = screens.firstObject().map(|main| main.frame().size.height) else {
        return 0.0;
    };
    screens
        .iter()
        .find(|screen| {
            let frame = screen.frame();
            matches_frame(
                (
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                ),
                (x, y, width, height),
                reference_height,
            )
        })
        .map(|screen| {
            let frame = screen.frame();
            let visible = screen.visibleFrame();
            let menu_bar =
                (frame.origin.y + frame.size.height) - (visible.origin.y + visible.size.height);
            offset(notch_top(&screen), menu_bar)
        })
        .unwrap_or(0.0)
}

// AppKit frames are bottom-left anchored on the main display, so the top-anchored
// logical y converts through the reference height; including it keeps vertically
// stacked displays of identical size apart.
fn matches_frame(
    (frame_x, frame_y, frame_width, frame_height): (f64, f64, f64, f64),
    (x, y, width, height): (f64, f64, f64, f64),
    reference_height: f64,
) -> bool {
    (frame_x - x).abs() <= 0.5
        && (frame_y - (reference_height - y - height)).abs() <= 0.5
        && (frame_width - width).abs() <= 0.5
        && (frame_height - height).abs() <= 0.5
}

// safeAreaInsets needs macOS 12; older releases have no notch to hang from.
fn notch_top(screen: &NSScreen) -> f64 {
    if NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        >= 12
    {
        screen.safeAreaInsets().top
    } else {
        0.0
    }
}

// A notched display hangs the island from the top edge over the camera housing;
// anywhere else the island would cover the menu bar, so it starts below it.
fn offset(notch_top: f64, menu_bar: f64) -> f64 {
    if notch_top > 0.0 {
        0.0
    } else {
        menu_bar
    }
}

pub fn install(app: &tauri::AppHandle) {
    if !available() {
        return;
    }
    let delegate = NotificationDelegate::alloc().set_ivars(app.clone());
    let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(delegate), init] };
    UNUserNotificationCenter::currentNotificationCenter()
        .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with(|stored| *stored.borrow_mut() = Some(delegate));
}

pub fn permission(request: bool) -> Result<String, String> {
    if !available() {
        return Ok("unavailable".into());
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::channel();
    if request {
        let callback = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let result = if error.is_null() {
                Ok(if granted.as_bool() {
                    "granted"
                } else {
                    "denied"
                }
                .to_string())
            } else {
                Err(i18n::io(unsafe { &*error }.localizedDescription()))
            };
            let _ = tx.send(result);
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert,
            &callback,
        );
    } else {
        let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            let result = if status == UNAuthorizationStatus::Authorized
                || status == UNAuthorizationStatus::Provisional
            {
                "granted"
            } else if status == UNAuthorizationStatus::NotDetermined {
                "default"
            } else {
                "denied"
            };
            let _ = tx.send(Ok(result.to_string()));
        });
        center.getNotificationSettingsWithCompletionHandler(&callback);
    }
    rx.recv_timeout(Duration::from_secs(60)).map_err(i18n::io)?
}

pub fn banner(notice: &Notice) -> Result<(), String> {
    if permission(false)? != "granted" {
        return Err(i18n::t("err.notifications.permission"));
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notice.title));
    content.setBody(&NSString::from_str(&notice.body));
    // One entry per tab replaces stale banners; test notices cannot open a conversation.
    let id = notice
        .tab
        .as_ref()
        .map(|tab| format!("tab:{tab}"))
        .unwrap_or_else(|| "preview".into());
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&id),
        &content,
        None,
    );
    let (tx, rx) = mpsc::channel();
    let callback = RcBlock::new(move |error: *mut NSError| {
        let result = if error.is_null() {
            Ok(())
        } else {
            Err(i18n::io(unsafe { &*error }.localizedDescription()))
        };
        let _ = tx.send(result);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
    rx.recv_timeout(Duration::from_secs(10)).map_err(i18n::io)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notchless_displays_push_the_island_below_the_menu_bar() {
        assert_eq!(offset(37.0, 25.0), 0.0);
        assert_eq!(offset(0.0, 25.0), 25.0);
    }

    #[test]
    fn stacked_identical_displays_match_by_converted_y() {
        let reference = 2160.0;
        let top = ((0.0, 1080.0, 1920.0, 1080.0), (0.0, 0.0, 1920.0, 1080.0));
        let bottom = ((0.0, 0.0, 1920.0, 1080.0), (0.0, 1080.0, 1920.0, 1080.0));
        for (frame, logical) in [top, bottom] {
            assert!(matches_frame(frame, logical, reference));
            let swapped = (logical.0, 1080.0 - logical.1, logical.2, logical.3);
            assert!(!matches_frame(frame, swapped, reference));
        }
    }
}
