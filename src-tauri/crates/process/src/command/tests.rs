use super::*;
fn shell(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    command
}
fn policy(timeout: Duration, limit: usize) -> CommandPolicy {
    CommandPolicy {
        timeout,
        stdout: OutputPolicy::Capture { limit },
        stderr: OutputPolicy::Capture { limit },
    }
}
#[test]
fn prepared_environment_directory_input_and_nonzero_exit_survive_the_boundary() {
    let mut command = shell(
        "read value; printf '%s:%s:' \"$BOUNDARY_VALUE\" \"$value\"; pwd; printf error >&2; exit 7",
    );
    command.current_dir("/").env("BOUNDARY_VALUE", "configured");
    let result = UnixCommandRunner
        .run(
            &mut command,
            b"input\n",
            policy(Duration::from_secs(3), 1024),
        )
        .unwrap();
    assert!(!result.success);
    assert_eq!(result.stdout, b"configured:input:/\n");
    assert_eq!(result.stderr, b"error");
}
#[test]
fn drains_both_outputs_while_sending_input_larger_than_a_pipe() {
    let mut command = shell("head -c 200000 /dev/zero; head -c 200000 /dev/zero >&2; cat");
    let input = vec![b'x'; 200000];
    let result = UnixCommandRunner
        .run(
            &mut command,
            &input,
            policy(Duration::from_secs(5), 1_000_000),
        )
        .unwrap();
    assert!(result.success);
    assert_eq!(&result.stdout[200000..], input);
    assert_eq!(result.stderr.len(), 200000);
}
#[test]
fn blocked_input_is_bounded_by_the_same_deadline() {
    let now = Instant::now();
    let result = UnixCommandRunner.run(
        &mut shell("exec sleep 30"),
        &vec![b'x'; 200000],
        policy(Duration::from_millis(80), 1024),
    );
    assert!(matches!(result, Err(CommandError::Timeout)));
    assert!(now.elapsed() < Duration::from_secs(3));
}
#[test]
fn output_limits_apply_to_each_stream_and_exact_limit_is_valid() {
    let result = UnixCommandRunner
        .run(
            &mut shell("printf abc; printf xyz >&2"),
            &[],
            policy(Duration::from_secs(2), 3),
        )
        .unwrap();
    assert_eq!(result.stdout, b"abc");
    assert_eq!(result.stderr, b"xyz");
    for script in ["printf abcd; sleep 30", "printf abcd >&2; sleep 30"] {
        assert!(matches!(
            UnixCommandRunner.run(&mut shell(script), &[], policy(Duration::from_secs(2), 3)),
            Err(CommandError::OutputLimit)
        ));
    }
}
#[test]
fn inherited_pipe_from_an_exited_leader_does_not_escape_timeout_cleanup() {
    let path = std::env::temp_dir().join(format!(
        "prometeu-pipe-child-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut command = shell("sleep 30 & echo $! > \"$PID_FILE\"; exit 0");
    command.env("PID_FILE", &path);
    let result = UnixCommandRunner.run(&mut command, &[], policy(Duration::from_millis(150), 1024));
    assert!(matches!(result, Err(CommandError::Timeout)));
    let pid = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", pid.trim()])
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&output.stdout);
        if status.trim().is_empty() || status.trim().starts_with('Z') {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pipe-holding descendant survived"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn discarded_output_is_not_returned_and_missing_executable_is_distinct() {
    let options = CommandPolicy {
        timeout: Duration::from_secs(2),
        stdout: OutputPolicy::Discard,
        stderr: OutputPolicy::Discard,
    };
    let result = UnixCommandRunner
        .run(
            &mut shell("printf secret; printf diagnostic >&2"),
            &[],
            options,
        )
        .unwrap();
    assert!(result.stdout.is_empty() && result.stderr.is_empty());
    assert!(matches!(
        UnixCommandRunner.run(
            &mut Command::new("/prometeu-test-missing-command"),
            &[],
            options
        ),
        Err(CommandError::Unavailable)
    ));
}
