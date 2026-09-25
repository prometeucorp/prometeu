//! Use macOS caffeinate to prevent sleep: -i blocks idle sleep, -d keeps the display on, and -s
//! covers other sleep attempts on AC power. The -w app PID ties its lifetime to ours, including
//! abrupt app termination. On Linux, systemd-inhibit holds the locks while `tail --pid` waits on
//! our PID. The UI chooses when to enable it based on user preference and active agents.

use crate::lock::lock;
use serde::Deserialize;
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Off,
    System,
    Display,
}

fn running() -> &'static Mutex<Option<(Mode, Child)>> {
    static RUNNING: OnceLock<Mutex<Option<(Mode, Child)>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(None))
}

/// Repeated requests for the same state are no-ops because the UI updates this setting with every
/// board change.
#[tauri::command]
pub fn set_awake(mode: Mode) -> Result<(), String> {
    // Not an error: the UI repeats this call on every board change.
    if !supported() {
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    if mode == Mode::System {
        return Err("system-only sleep inhibition is unavailable on this platform".into());
    }
    let mut child = lock(running());
    // Restart an unexpectedly exited inhibitor process instead of treating its stale handle as
    // active.
    let mut alive = child.take();
    if let Some((_, caffeinate)) = &mut alive {
        // Only Ok(None) proves the process is still running; a status or error does not.
        if !matches!(caffeinate.try_wait(), Ok(None)) {
            alive = None;
        }
    }
    if let Some((current, alive)) = alive {
        if current == mode {
            *child = Some((current, alive));
            return Ok(());
        }
        let mut alive = alive;
        stop(&alive);
        let _ = alive.kill();
        let _ = alive.wait();
    }
    if mode != Mode::Off {
        let spawned = inhibitor(mode).spawn().map_err(|error| error.to_string())?;
        *child = Some((mode, spawned));
    }
    Ok(())
}

pub fn shutdown() {
    let _ = set_awake(Mode::Off);
}

#[cfg(target_os = "macos")]
fn inhibitor(mode: Mode) -> Command {
    let mut cmd = Command::new("/usr/bin/caffeinate");
    if mode == Mode::Display {
        cmd.arg("-d");
    }
    cmd.args(["-i", "-s", "-w", &std::process::id().to_string()]);
    cmd
}

#[cfg(target_os = "macos")]
fn supported() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
fn supported() -> bool {
    crate::platform::has("systemd-inhibit") && std::path::Path::new("/run/systemd/system").exists()
}

#[cfg(target_os = "macos")]
fn stop(_: &Child) {}

/// systemd-inhibit does not forward signals to tail, so kill its whole group.
#[cfg(not(target_os = "macos"))]
fn stop(child: &Child) {
    // SAFETY: only sends a signal to the group created in inhibitor().
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
}

#[cfg(not(target_os = "macos"))]
fn inhibitor(_: Mode) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new("systemd-inhibit");
    cmd.process_group(0);
    cmd.args([
        "--what=idle:sleep",
        "--who=Prometeu",
        "--why=Agents are running",
        "--mode=block",
        "tail",
        "--pid",
        &std::process::id().to_string(),
        "-f",
        "/dev/null",
    ]);
    cmd
}

#[cfg(all(test, target_os = "macos"))]
const EXPECTED: &str = "caffeinate -d -i -s -w";
#[cfg(all(test, not(target_os = "macos")))]
const EXPECTED: &str = "systemd-inhibit --what=idle:sleep";

#[cfg(test)]
mod tests {
    use super::*;

    /// Start and stop the real inhibitor. The test PID it waits on ensures failures cannot leave
    /// the machine permanently awake.
    #[test]
    fn enables_display_and_system_awake_once_then_disables_them() {
        if !supported() {
            eprintln!("skipped: no systemd session to inhibit");
            return;
        }
        set_awake(Mode::Display).expect("inhibitor did not start");
        let first = lock(running()).as_ref().map(|(_, child)| child.id());
        assert!(first.is_some());

        // Verify the flags as well as the child: caffeinate with only -i allowed the display to
        // sleep.
        let command = Command::new("/bin/ps")
            .args(["-p", &first.unwrap().to_string(), "-o", "command="])
            .output()
            .expect("could not read inhibitor");
        let command = String::from_utf8_lossy(&command.stdout);
        assert!(command.contains(EXPECTED), "{command}");

        set_awake(Mode::Display).expect("second request");
        assert_eq!(lock(running()).as_ref().map(|(_, child)| child.id()), first);

        #[cfg(target_os = "macos")]
        {
            set_awake(Mode::System).expect("system-only inhibitor");
            let second = lock(running()).as_ref().map(|(_, child)| child.id());
            assert_ne!(second, first);
            let output = Command::new("/bin/ps")
                .args(["-p", &second.unwrap().to_string(), "-o", "command="])
                .output()
                .unwrap();
            let command = String::from_utf8_lossy(&output.stdout);
            assert!(command.contains("caffeinate -i -s -w"), "{command}");
            assert!(!command.contains(" -d "), "{command}");
        }
        set_awake(Mode::Off).expect("disable");
        assert!(lock(running()).is_none());
        // Disabling an already disabled assertion is a no-op.
        set_awake(Mode::Off).expect("disable again");
        assert!(lock(running()).is_none());
        set_awake(Mode::Display).expect("restart before normal exit");
        shutdown();
        assert!(lock(running()).is_none());
        assert_eq!(
            serde_json::from_str::<Mode>("\"system\"").unwrap(),
            Mode::System
        );
    }
}
