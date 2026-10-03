use super::*;
use prometeu_core::process::ShutdownPolicy;
use std::io::Write;
use std::sync::mpsc;

fn shell(script: &str) -> StartedProcess {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    UnixProcessLauncher.launch(command).unwrap()
}

fn running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&output.stdout);
    let status = status.trim();
    !status.is_empty() && !status.starts_with('Z')
}

fn stopped(pid: u32) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    !running(pid)
}

#[test]
fn launch_preserves_arguments_environment_removals_and_working_directory() {
    let root = std::env::temp_dir().join(format!("prometeu process 日本語 {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "printf '%s\\n%s\\n%s\\n' \"$1\" \"$PROMETEU_TEST_VALUE\" \"${PROMETEU_REMOVED-unset}\"; pwd; printf diagnostic >&2; exit 7", "fixture", "argument with spaces 日本語"]);
    command
        .env_clear()
        .env("PROMETEU_TEST_VALUE", "literal $() value")
        .env("PROMETEU_REMOVED", "remove me")
        .env_remove("PROMETEU_REMOVED")
        .current_dir(&root);
    let StartedProcess {
        input,
        stdout,
        stderr,
        handle,
        mut waiter,
    } = UnixProcessLauncher.launch(command).unwrap();
    drop(input);
    let output: Vec<_> = stdout.collect();
    assert_eq!(
        &output[..3],
        ["argument with spaces 日本語", "literal $() value", "unset"]
    );
    assert_eq!(
        std::fs::canonicalize(&output[3]).unwrap(),
        std::fs::canonicalize(&root).unwrap()
    );
    assert_eq!(stderr.collect::<Vec<_>>(), ["diagnostic"]);
    assert_eq!(waiter.wait().unwrap().code, Some(7));
    assert!(!handle.control.running());
    assert_eq!(waiter.wait().unwrap().code, Some(7));
    handle.control.interrupt();
    handle.control.terminate();
    handle.control.kill();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_spawn_failure_returns_without_a_live_process() {
    assert!(matches!(
        UnixProcessLauncher.launch(Command::new("/prometeu-missing-executable")),
        Err(LaunchError::Spawn(_))
    ));
}

#[test]
fn closing_input_allows_a_graceful_exit_with_its_final_output() {
    let StartedProcess {
        input,
        stdout,
        stderr,
        mut waiter,
        ..
    } = shell("while IFS= read -r line; do :; done; printf 'input closed\\n'; exit 12");
    drop(input);
    assert_eq!(stdout.collect::<Vec<_>>(), ["input closed"]);
    assert_eq!(stderr.count(), 0);
    assert_eq!(waiter.wait().unwrap().code, Some(12));
}

#[test]
fn interrupt_reaches_the_process_group() {
    let StartedProcess {
        input,
        mut stdout,
        handle,
        mut waiter,
        ..
    } = shell("trap 'exit 23' INT; echo ready; while :; do sleep 1; done");
    assert_eq!(stdout.next().as_deref(), Some("ready"));
    let (finished, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = finished.send(waiter.wait());
    });
    handle.control.interrupt();
    let result = result.recv_timeout(Duration::from_secs(5));
    if result.is_err() {
        handle.control.kill();
    }
    assert_eq!(result.unwrap().unwrap().code, Some(23));
    drop(input);
}

#[test]
fn closing_stdout_does_not_disable_shutdown_of_a_live_child() {
    let StartedProcess {
        input,
        mut stdout,
        stderr,
        handle,
        mut waiter,
    } = shell("exec 1>&-; exec sleep 30");
    drop(input);
    assert_eq!(stdout.next(), None);
    assert!(handle.control.running());
    let (finished, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = finished.send(waiter.wait());
    });
    ShutdownPolicy {
        input_grace: Duration::from_millis(10),
        terminate_grace: Duration::from_millis(10),
    }
    .shutdown(handle.control.as_ref());
    result
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert!(!handle.control.running());
    assert_eq!(stderr.count(), 0);
}

#[test]
fn escalation_reaches_a_child_and_descendant_that_ignore_termination() {
    let StartedProcess {
        input,
        mut stdout,
        stderr,
        handle,
        mut waiter,
    } = shell("trap '' TERM; (trap '' TERM; exec sleep 30) & echo $!; wait");
    let descendant: u32 = stdout.next().unwrap().parse().unwrap();
    assert!(running(descendant));
    drop(input);
    let (finished, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = stdout.count();
        let _ = stderr.count();
        let _ = finished.send(waiter.wait());
    });
    ShutdownPolicy {
        input_grace: Duration::from_millis(10),
        terminate_grace: Duration::from_millis(10),
    }
    .shutdown(handle.control.as_ref());
    result
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert!(stopped(handle.control.system_id()));
    assert!(stopped(descendant));
}

#[test]
fn abandoning_the_waiter_kills_and_reaps_the_owned_child() {
    let started = shell("exec sleep 30");
    let handle = started.handle.clone();
    assert!(running(handle.control.system_id()));
    drop(started);
    assert!(!handle.control.running());
    assert!(stopped(handle.control.system_id()));
}

#[test]
fn both_output_pipes_drain_while_a_large_input_waits_for_the_child() {
    let StartedProcess { mut input, stdout, stderr, handle, mut waiter } = shell(
        "head -c 262144 /dev/zero | tr '\\000' x; printf '\\n'; head -c 262144 /dev/zero | tr '\\000' y >&2; printf '\\n' >&2; IFS= read -r line; printf '%s\\n' \"${#line}\""
    );
    let (written, received) = mpsc::channel();
    std::thread::spawn(move || {
        let result = input.write_all(("p".repeat(1024 * 1024) + "\n").as_bytes());
        let _ = written.send(result);
    });
    let result = received.recv_timeout(Duration::from_secs(10));
    if result.is_err() {
        handle.control.kill();
    }
    result.unwrap().unwrap();
    let output: Vec<_> = stdout.collect();
    assert_eq!(output, ["x".repeat(262144), "1048576".into()]);
    assert_eq!(stderr.collect::<Vec<_>>(), ["y".repeat(262144)]);
    assert_eq!(waiter.wait().unwrap().code, Some(0));
}
