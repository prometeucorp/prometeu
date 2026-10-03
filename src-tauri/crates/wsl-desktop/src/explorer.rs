//! Native file-manager effects. WSL validates and translates entries before this adapter is called.
use prometeu_bridge::application::FileManager;

pub struct Explorer;

#[cfg(windows)]
impl FileManager for Explorer {
    fn reveal(&self, path: &str, dir: bool) -> Result<(), String> {
        use windows_sys::Win32::{
            System::Com::{
                CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
            },
            UI::{
                Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteW},
                WindowsAndMessaging::SW_SHOWNORMAL,
            },
        };
        let failed = || "Windows Explorer could not reveal the entry".to_string();
        let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        // The blocking worker owns its COM initialization and every returned item list.
        // No command line, shell expansion or file association is used for file selection.
        unsafe {
            let initialized = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            if initialized < 0 {
                return Err(failed());
            }
            let result = match dir {
                true => {
                    let opened = ShellExecuteW(
                        std::ptr::null_mut(),
                        windows_sys::core::w!("explore"),
                        path.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOWNORMAL,
                    );
                    (opened as isize > 32).then_some(()).ok_or_else(failed)
                }
                false => {
                    let mut item = std::ptr::null_mut();
                    let parsed = SHParseDisplayName(
                        path.as_ptr(),
                        std::ptr::null_mut(),
                        &mut item,
                        0,
                        std::ptr::null_mut(),
                    );
                    let result = match parsed >= 0 && !item.is_null() {
                        true => (SHOpenFolderAndSelectItems(item, 0, std::ptr::null(), 0) >= 0)
                            .then_some(())
                            .ok_or_else(failed),
                        false => Err(failed()),
                    };
                    CoTaskMemFree(item.cast());
                    result
                }
            };
            CoUninitialize();
            result
        }
    }
}

#[cfg(not(windows))]
impl FileManager for Explorer {
    fn reveal(&self, _: &str, _: bool) -> Result<(), String> {
        Err("Windows Explorer is unavailable on this host".into())
    }
}
