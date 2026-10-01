//! System programs and values that differ between macOS and Linux.

use std::process::Command;

pub fn opener() -> Command {
    Command::new(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    })
}

/// $SHELL, then the account's shell: launchers may omit $SHELL, and /bin/sh skips the startup files
/// that set PATH.
pub fn shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .or_else(account_shell)
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".into()
            } else {
                "/bin/sh".into()
            }
        })
}

fn account_shell() -> Option<String> {
    // SAFETY: the entry is checked for null and copied out before any other getpwuid call.
    let shell = unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() || (*entry).pw_shell.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*entry).pw_shell)
            .to_string_lossy()
            .into_owned()
    };
    (!shell.is_empty() && std::path::Path::new(&shell).exists()).then_some(shell)
}

/// Human-readable machine name shown to companion devices.
pub fn device_name() -> String {
    #[cfg(target_os = "macos")]
    let (name, fallback) = (
        Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok()),
        "Mac",
    );
    #[cfg(not(target_os = "macos"))]
    let (name, fallback) = (
        std::fs::read_to_string("/proc/sys/kernel/hostname").ok(),
        "Linux",
    );
    name.map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fallback.into())
}

/// Find an executable on PATH without running it. On macOS the process adopts the login-shell PATH
/// at startup, so this sees the same CLIs as a terminal.
pub fn has(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            std::fs::metadata(dir.join(program))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_shell_is_an_existing_program() {
        let shell = account_shell().expect("the test user has a shell");
        assert!(std::path::Path::new(&shell).exists(), "{shell}");
    }

    #[test]
    fn device_name_is_never_empty() {
        assert!(!device_name().is_empty());
    }

    #[test]
    fn finds_programs_on_path_only() {
        assert!(has("sh"));
        assert!(!has("prometeu-test-no-such-program"));
    }
}
