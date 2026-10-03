//! Windows owns browser consent and its loopback socket; credentials remain in WSL.
use prometeu_bridge::oauth_error;
use prometeu_oauth::{self as oauth, Consent, ConsentRequest};
use std::{
    net::TcpListener,
    time::{Duration, Instant},
};
pub struct BrowserConsent;
impl Consent for BrowserConsent {
    fn authorize(&self, request: &ConsentRequest) -> Result<String, String> {
        let listener = TcpListener::bind(("127.0.0.1", oauth::MCP_PORT))
            .map_err(|e| oauth_error::port(e.to_string()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| oauth_error::port(e.to_string()))?;
        open(&request.authorize)?;
        let language = request.language.clone();
        oauth::wait_for_code(
            &listener,
            oauth::MCP_ROUTE,
            &request.state,
            Instant::now() + Duration::from_secs(300),
            &move |ok, why| oauth::mcp_page(&language, ok, why),
        )
        .map_err(oauth_error::denied)
    }
}
#[cfg(windows)]
fn open(url: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    // ShellExecute receives a literal web URL, never a command line or local file association.
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.contains('\0') {
        return Err(oauth_error::browser());
    }
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        // The native blocking worker owns its COM apartment for the shell extension call.
        if CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        ) < 0
        {
            return Err(oauth_error::browser());
        }
        let result = windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            windows_sys::core::w!("open"),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
        CoUninitialize();
        result
    };
    (result as isize > 32)
        .then_some(())
        .ok_or_else(oauth_error::browser)
}
#[cfg(not(windows))]
fn open(_: &str) -> Result<(), String> {
    Err(oauth_error::browser())
}
