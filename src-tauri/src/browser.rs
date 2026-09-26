/// Display workspace Run in a native child webview over the main window's content area. The
/// frontend measures its logical bounds because native views do not follow CSS. Resolve the initial
/// port from backend state, and retain one webview per workspace so hiding and showing it preserves
/// navigation.
use crate::dock::ensure_port;
use crate::{i18n, AppState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, State, Url, WebviewBuilder,
    WebviewUrl,
};

/// Sanitize the webview label to Tauri's supported characters even though workspace IDs already
/// follow this format.
fn label(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-/:_".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("run-{safe}")
}

fn allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

#[cfg(target_os = "macos")]
mod media {
    use block2::DynBlock;
    use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
    use objc2::{sel, MainThreadMarker};
    use objc2_web_kit::{
        WKFrameInfo, WKMediaCaptureType, WKPermissionDecision, WKSecurityOrigin, WKWebView,
    };
    use std::sync::mpsc;
    use std::time::Duration;

    const CLASS: &std::ffi::CStr = c"PrometeuRunWebViewUIDelegate";

    extern "C-unwind" fn deny(
        _delegate: &AnyObject,
        _selector: Sel,
        _webview: &WKWebView,
        _origin: &WKSecurityOrigin,
        _frame: &WKFrameInfo,
        _capture_type: WKMediaCaptureType,
        decision: &DynBlock<dyn Fn(WKPermissionDecision)>,
    ) {
        decision.call((WKPermissionDecision::Deny,));
    }

    fn restricted_class(superclass: &AnyClass) -> Option<&'static AnyClass> {
        if let Some(class) = AnyClass::get(CLASS) {
            return (class.superclass() == Some(superclass)).then_some(class);
        }
        let mut class = ClassBuilder::new(CLASS, superclass)?;
        unsafe {
            class.add_method(
                sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:),
                deny as extern "C-unwind" fn(_, _, _, _, _, _, _),
            );
        }
        Some(class.register())
    }

    fn install(delegate: &AnyObject) -> bool {
        let current = delegate.class();
        if current.name() == CLASS {
            return true;
        }
        restricted_class(current)
            .is_some_and(|class| unsafe { AnyObject::set_class(delegate, class) == current })
    }

    /// Wry grants media capture by default. Change only this child view's delegate class, retaining
    /// inherited upload and new-window behavior while overriding microphone and camera requests.
    pub fn restrict(view: &tauri::Webview) -> bool {
        let (send, receive) = mpsc::sync_channel(1);
        if view
            .with_webview(move |platform| unsafe {
                let webview: &WKWebView = &*platform.inner().cast();
                let installed = MainThreadMarker::new()
                    .and_then(|_| webview.UIDelegate())
                    .and_then(|delegate| {
                        let object: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
                        install(object).then_some(())
                    })
                    .is_some();
                let _ = send.try_send(installed);
            })
            .is_err()
        {
            return false;
        }
        receive.recv_timeout(Duration::from_secs(3)) == Ok(true)
    }

    #[test]
    fn restricted_delegate_overrides_media_capture() {
        use objc2::runtime::NSObject;
        use objc2::ClassType;

        let delegate = NSObject::new();
        assert!(install(&delegate));
        assert_eq!(delegate.class().superclass(), Some(NSObject::class()));
        assert!(delegate
            .class()
            .instance_method(sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))
            .is_some());
        assert!(install(&delegate));
    }
}

/// Open only HTTP or HTTPS URLs in the system browser. Other schemes could open local files or
/// arbitrary applications.
pub(crate) fn browse(url: &Url) -> Result<(), String> {
    if !allowed(url) {
        return Err(i18n::ta("err.browser.badUrl", &[("url", url.to_string())]));
    }
    let ok = crate::platform::opener()
        .arg(url.as_str())
        .status()
        .map_err(i18n::io)?
        .success();
    ok.then_some(())
        .ok_or_else(|| i18n::t("err.browser.noBrowser"))
}

/// Links from app content open in the system browser. The embedded view belongs to Run; navigation
/// within that view remains the person's choice.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    let parsed =
        Url::parse(&url).map_err(|_| i18n::ta("err.browser.badUrl", &[("url", url.clone())]))?;
    browse(&parsed)
}

/// Create the workspace webview on first use. The frontend supplies its bounds separately; return
/// the port for the address display.
#[tauri::command]
pub fn browser_open(app: AppHandle, state: State<AppState>, id: String) -> Result<u16, String> {
    let port = ensure_port(&app, &state, &id).ok_or_else(|| i18n::t("err.session.noPort"))?;
    let url = format!("http://localhost:{port}");
    let fail = || i18n::ta("err.session.openFailed", &[("path", url.clone())]);
    if let Some(view) = app.get_webview(&label(&id)) {
        #[cfg(target_os = "macos")]
        if !media::restrict(&view) {
            let _ = view.close();
            return Err(fail());
        }
        view.show().map_err(|_| fail())?;
        return Ok(port);
    }
    let window = app.get_window("main").ok_or_else(fail)?;
    let parsed = Url::parse(&url).map_err(|_| fail())?;
    #[cfg(target_os = "macos")]
    let initial = Url::parse("about:blank").expect("valid blank URL");
    #[cfg(not(target_os = "macos"))]
    let initial = parsed.clone();
    // on_navigation includes subframes without identifying the main frame, so permit navigation by
    // scheme rather than host. Redirecting off-site frames would open a browser tab for every ad or
    // login iframe. Track main-frame page loads, and poll browser_url for SPA history changes.
    let of = id.clone();
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    let view = window
        .add_child(
            WebviewBuilder::new(label(&id), WebviewUrl::External(initial))
                // Keep page uploads in WebKit. Only the main webview receives chat attachments.
                .disable_drag_drop_handler()
                .on_navigation(|url| allowed(url) || url.as_str() == "about:blank")
                .on_page_load(move |view, payload| {
                    if matches!(payload.event(), PageLoadEvent::Started) && allowed(payload.url()) {
                        let _ = view.emit("browser:url", (of.clone(), payload.url().to_string()));
                    }
                })
                // Open window.open and target=_blank requests in the system browser while
                // preserving the embedded page.
                .on_new_window(move |url, _| {
                    let _ = browse(&url);
                    NewWindowResponse::Deny
                }),
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(0.0, 0.0),
        )
        .map_err(|_| fail())?;
    #[cfg(target_os = "macos")]
    {
        if !media::restrict(&view) {
            let _ = view.close();
            return Err(fail());
        }
        if view.navigate(parsed).is_err() {
            let _ = view.close();
            return Err(fail());
        }
    }
    Ok(port)
}

/// Read the current webview URL to catch SPA navigation and history.pushState changes.
#[tauri::command]
pub fn browser_url(app: AppHandle, id: String) -> Option<String> {
    let view = app.get_webview(&label(&id))?;
    view.url().ok().map(|u| u.to_string())
}

/// Accept typed HTTP or HTTPS addresses only. Unlike the initial Run URL, this address
/// intentionally comes from the frontend; reject file and application schemes.
#[tauri::command]
pub fn browser_navigate(app: AppHandle, id: String, url: String) -> Result<(), String> {
    let bad = || i18n::ta("err.browser.badUrl", &[("url", url.clone())]);
    let parsed = Url::parse(&url).map_err(|_| bad())?;
    if !allowed(&parsed) {
        return Err(bad());
    }
    let view = app.get_webview(&label(&id)).ok_or_else(bad)?;
    view.navigate(parsed).map_err(|_| bad())
}

/// Use logical pixels relative to the window, matching getBoundingClientRect in the full-window
/// main webview.
#[tauri::command]
pub fn browser_bounds(app: AppHandle, id: String, x: f64, y: f64, w: f64, h: f64) {
    if let Some(view) = app.get_webview(&label(&id)) {
        let _ = view.set_bounds(Rect {
            position: LogicalPosition::new(x, y).into(),
            size: LogicalSize::new(w, h).into(),
        });
    }
}

/// Hide the webview without destroying its page when the center panel or workspace changes.
#[tauri::command]
pub fn browser_hide(app: AppHandle, id: String) {
    if let Some(view) = app.get_webview(&label(&id)) {
        let _ = view.hide();
    }
}

/// Use the page's native history for back and forward because Tauri does not expose dedicated
/// navigation methods.
#[tauri::command]
pub fn browser_back(app: AppHandle, id: String) {
    hop(&app, &id, "history.back()");
}

#[tauri::command]
pub fn browser_forward(app: AppHandle, id: String) {
    hop(&app, &id, "history.forward()");
}

fn hop(app: &AppHandle, id: &str, js: &str) {
    if let Some(view) = app.get_webview(&label(id)) {
        let _ = view.eval(js);
    }
}

#[tauri::command]
pub fn browser_reload(app: AppHandle, id: String) {
    if let Some(view) = app.get_webview(&label(&id)) {
        let _ = view.reload();
    }
}

/// Destroy closed tabs' webviews to avoid retaining hidden pages and their resource usage.
#[tauri::command]
pub fn browser_close(app: AppHandle, id: String) {
    if let Some(view) = app.get_webview(&label(&id)) {
        let _ = view.close();
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl BrowserRect {
    fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 100_000.0)
            && self.width > 0.0
            && self.height > 0.0
    }

    #[cfg(any(target_os = "macos", test))]
    fn clipped(&self, width: f64, height: f64) -> Option<Self> {
        if !self.valid() || !width.is_finite() || !height.is_finite() {
            return None;
        }
        let x = self.x.max(0.0);
        let y = self.y.max(0.0);
        let rect = Self {
            x,
            y,
            width: (self.x + self.width).min(width) - x,
            height: (self.y + self.height).min(height) - y,
        };
        rect.valid().then_some(rect)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewport {
    width: f64,
    height: f64,
}

/// Page content stays untrusted text. It never supplies a file path or JavaScript to execute.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSelection {
    url: String,
    selector: String,
    tag: String,
    text: String,
    html: String,
    styles: BTreeMap<String, String>,
    rect: BrowserRect,
    viewport: BrowserViewport,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserInspection {
    active: bool,
    selection: Option<BrowserSelection>,
}

fn parse_inspection(json: &str) -> Result<BrowserInspection, String> {
    let fail = || i18n::t("err.browser.inspectFailed");
    if json.len() > 256 * 1024 {
        return Err(fail());
    }
    let inspection: BrowserInspection = serde_json::from_str(json).map_err(|_| fail())?;
    if let Some(value) = &inspection.selection {
        if value.url.len() > 16_384
            || !Url::parse(&value.url).is_ok_and(|url| allowed(&url))
            || value.selector.is_empty()
            || value.selector.len() > 4096
            || value.tag.is_empty()
            || value.tag.len() > 128
            || !value
                .tag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || value.text.len() > 8192
            || value.html.len() > 49_152
            || value.styles.len() > 64
            || value
                .styles
                .iter()
                .any(|(key, value)| key.is_empty() || key.len() > 128 || value.len() > 4096)
            || !value.rect.valid()
            || !(BrowserRect {
                x: 0.0,
                y: 0.0,
                width: value.viewport.width,
                height: value.viewport.height,
            })
            .valid()
        {
            return Err(fail());
        }
    }
    Ok(inspection)
}

/// The script is fixed by the app bundle. Remote pages receive no additional Tauri capability.
async fn evaluate(view: tauri::Webview, script: String) -> Result<String, String> {
    let (send, receive) = mpsc::sync_channel(1);
    view.eval_with_callback(script, move |json| {
        let _ = send.try_send(json);
    })
    .map_err(|_| i18n::t("err.browser.inspectFailed"))?;
    tauri::async_runtime::spawn_blocking(move || receive.recv_timeout(Duration::from_secs(3)))
        .await
        .map_err(|_| i18n::t("err.browser.inspectFailed"))?
        .map_err(|_| i18n::t("err.browser.inspectFailed"))
}

#[tauri::command]
pub async fn browser_inspect(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    let view = app
        .get_webview(&label(&id))
        .ok_or_else(|| i18n::t("err.browser.inspectFailed"))?;
    let script = if enabled {
        format!(
            "(() => {{ try {{ {}\n; window.__prometeuInspector.setEnabled(true); return true; }} catch {{ return false; }} }})()",
            include_str!("../../src/browser-inspector.js")
        )
    } else {
        // Hiding a preview never installs listeners into a page that was not inspected.
        "(() => { try { window.__prometeuInspector?.setEnabled(false); return true; } catch { return false; } })()".into()
    };
    let result = evaluate(view, script).await?;
    if result == "true" {
        Ok(())
    } else {
        Err(i18n::t("err.browser.inspectFailed"))
    }
}

#[tauri::command]
pub async fn browser_selection(app: AppHandle, id: String) -> Result<BrowserInspection, String> {
    let view = app
        .get_webview(&label(&id))
        .ok_or_else(|| i18n::t("err.browser.inspectFailed"))?;
    let json = evaluate(
        view,
        // Wry's native serializer assumes a JSON-compatible value. Return only a string even if
        // the page replaced the inspector API or JSON.stringify with hostile code.
        r#"(() => { try {
            const json = JSON.stringify({
                active: window.__prometeuInspector?.enabled ?? false,
                selection: window.__prometeuInspector?.takeSelection() ?? null
            });
            return typeof json === 'string' && json.length <= 262144 ? json : '';
        } catch { return ''; } })()"#
            .into(),
    )
    .await?;
    if json.len() > 512 * 1024 {
        return Err(i18n::t("err.browser.inspectFailed"));
    }
    let json: String =
        serde_json::from_str(&json).map_err(|_| i18n::t("err.browser.inspectFailed"))?;
    parse_inspection(&json)
}

#[cfg(target_os = "macos")]
async fn require_current_selection(view: &tauri::Webview) -> Result<(), String> {
    let result = evaluate(
        view.clone(),
        "(() => { try { return window.__prometeuInspector?.selectionCurrent?.() === true; } catch { return false; } })()".into(),
    )
    .await;
    if result.as_deref() == Ok("true") {
        Ok(())
    } else {
        Err(i18n::t("err.browser.captureFailed"))
    }
}

/// Snapshot only the embedded page, without screen-recording permission or a caller-supplied path.
#[tauri::command]
pub async fn browser_capture(
    app: AppHandle,
    id: String,
    rect: Option<BrowserRect>,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        use block2::RcBlock;
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
        use objc2_foundation::{NSDictionary, NSError, NSPoint, NSRect, NSSize};
        use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

        if rect.is_some_and(|rect| !rect.valid()) {
            return Err(i18n::t("err.browser.captureFailed"));
        }
        let view = app
            .get_webview(&label(&id))
            .ok_or_else(|| i18n::t("err.browser.captureFailed"))?;
        if rect.is_some() {
            require_current_selection(&view).await?;
        }
        let (send, receive) = mpsc::sync_channel(1);
        view.with_webview(move |platform| unsafe {
            // Tauri supplies a live WKWebView and runs this closure on the AppKit thread.
            let webview: &WKWebView = &*platform.inner().cast();
            let Some(main) = MainThreadMarker::new() else {
                let _ = send.try_send(None);
                return;
            };
            let bounds = webview.bounds();
            let target = rect.unwrap_or(BrowserRect {
                x: 0.0,
                y: 0.0,
                width: bounds.size.width,
                height: bounds.size.height,
            });
            let Some(target) = target.clipped(bounds.size.width, bounds.size.height) else {
                let _ = send.try_send(None);
                return;
            };
            let configuration = WKSnapshotConfiguration::new(main);
            configuration.setRect(NSRect::new(
                NSPoint::new(target.x, target.y),
                NSSize::new(target.width, target.height),
            ));
            let completed = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let png = (|| {
                    if !error.is_null() {
                        return None;
                    }
                    let tiff = image.as_ref()?.TIFFRepresentation()?;
                    let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
                    let png = bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )?;
                    (png.len() <= 20 * 1024 * 1024).then(|| png.to_vec())
                })();
                let _ = send.try_send(png);
            });
            webview
                .takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &completed);
        })
        .map_err(|_| i18n::t("err.browser.captureFailed"))?;
        let png = tauri::async_runtime::spawn_blocking(move || {
            receive
                .recv_timeout(Duration::from_secs(5))
                .ok()
                .flatten()
                .ok_or_else(|| i18n::t("err.browser.captureFailed"))
        })
        .await
        .map_err(|_| i18n::t("err.browser.captureFailed"))??;
        // A page can scroll or reflow while WebKit renders. Keep the original text, but never
        // persist an image of a different region under that selection.
        if rect.is_some() {
            require_current_selection(&view).await?;
        }
        tauri::async_runtime::spawn_blocking(move || {
            let file = crate::paths::root()
                .join("attachments")
                .join(uuid::Uuid::new_v4().to_string())
                .join("browser.png");
            crate::paths::write_private_bytes(&file, &png)
                .map_err(|_| i18n::t("err.browser.captureFailed"))?;
            Ok(file.to_string_lossy().into_owned())
        })
        .await
        .map_err(|_| i18n::t("err.browser.captureFailed"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, id, rect);
        Err(i18n::t("err.browser.captureFailed"))
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_inspection, BrowserRect};

    fn inspection() -> serde_json::Value {
        serde_json::json!({
            "active": false,
            "selection": {
                "url": "http://localhost:3000/design",
                "selector": "main > button",
                "tag": "button",
                "text": "Create workspace",
                "html": "<button>Create workspace</button>",
                "styles": { "padding": "12px", "color": "rgb(0, 0, 0)" },
                "rect": { "x": -20.0, "y": 12.0, "width": 120.0, "height": 40.0 },
                "viewport": { "width": 800.0, "height": 600.0 }
            }
        })
    }

    #[test]
    fn inspection_preserves_text_and_state_without_executing_html() {
        let payload = inspection();
        let parsed = parse_inspection(&payload.to_string()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), payload);
        let empty = parse_inspection(r#"{"active":true,"selection":null}"#).unwrap();
        assert!(empty.active);
        assert!(empty.selection.is_none());
    }

    #[test]
    fn inspection_rejects_invalid_or_oversized_external_payloads() {
        for (field, value) in [
            ("url", serde_json::json!("file:///etc/passwd")),
            ("url", serde_json::json!("javascript:alert(1)")),
            ("tag", serde_json::json!("button onclick=x")),
            ("selector", serde_json::json!("")),
            ("text", serde_json::json!("é".repeat(4097))),
            ("html", serde_json::json!("a".repeat(49_153))),
            ("styles", serde_json::json!({ "color": "a".repeat(4097) })),
            (
                "rect",
                serde_json::json!({"x": 0, "y": 0, "width": -1, "height": 1}),
            ),
            ("viewport", serde_json::json!({"width": 0, "height": 800})),
        ] {
            let mut payload = inspection();
            payload["selection"][field] = value;
            assert!(parse_inspection(&payload.to_string()).is_err(), "{field}");
        }
        let mut payload = inspection();
        payload["selection"]["path"] = serde_json::json!("/etc/passwd");
        assert!(parse_inspection(&payload.to_string()).is_err());
        assert!(parse_inspection(&" ".repeat(256 * 1024 + 1)).is_err());
        assert!(parse_inspection(r#"{"active":"true","selection":null}"#).is_err());
    }

    #[test]
    fn capture_clips_elements_to_the_viewport_and_rejects_invalid_geometry() {
        let rect = BrowserRect {
            x: -20.0,
            y: 580.0,
            width: 120.0,
            height: 40.0,
        };
        let clipped = rect.clipped(800.0, 600.0).unwrap();
        assert_eq!(
            (clipped.x, clipped.y, clipped.width, clipped.height),
            (0.0, 580.0, 100.0, 20.0)
        );
        assert!(rect.clipped(800.0, 500.0).is_none());
        assert!(rect.clipped(f64::NAN, 600.0).is_none());
        assert!(BrowserRect {
            width: f64::INFINITY,
            ..rect
        }
        .clipped(800.0, 600.0)
        .is_none());
        assert!(BrowserRect {
            height: 0.0,
            ..rect
        }
        .clipped(800.0, 600.0)
        .is_none());
    }

    #[test]
    fn labels_use_only_tauri_supported_characters() {
        assert_eq!(super::label("dock-1130"), "run-dock-1130");
        assert_eq!(super::label("porta 17.a"), "run-porta-17-a");
    }

    #[test]
    fn navigation_accepts_only_http_urls() {
        assert!(super::allowed(
            &tauri::Url::parse("http://localhost:3000").unwrap()
        ));
        assert!(super::allowed(
            &tauri::Url::parse("https://example.com/login").unwrap()
        ));
        assert!(!super::allowed(
            &tauri::Url::parse("file:///etc/passwd").unwrap()
        ));
        assert!(!super::allowed(
            &tauri::Url::parse("javascript:alert(1)").unwrap()
        ));
    }
}
