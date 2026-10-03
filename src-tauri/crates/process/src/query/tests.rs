use super::*;
fn open(script: &str, timeout: Duration, max_output: usize) -> Box<dyn QueryProcess> {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    UnixQueryLauncher
        .launch(
            &mut command,
            QueryPolicy {
                timeout,
                max_output,
            },
        )
        .unwrap()
}
#[test]
fn query_retains_delimiters_and_returns_nonzero_exit_after_input_closes() {
    let mut query = open(
        "cat; printf private >&2; exit 7",
        Duration::from_secs(2),
        100,
    );
    query.send(b"hello\r\n").unwrap();
    query.close_input();
    assert_eq!(query.next().unwrap().as_deref(), Some("hello\r\n"));
    assert_eq!(query.next().unwrap(), None);
    assert!(!query.finish().unwrap());
    assert!(!query.finish().unwrap());
}
#[test]
fn total_output_limit_and_invalid_utf8_are_distinct_native_failures() {
    let mut query = open("printf 'ab\ncd\n'; sleep 30", Duration::from_secs(2), 5);
    assert_eq!(query.next().unwrap().as_deref(), Some("ab\n"));
    assert_eq!(query.next().unwrap_err(), CommandError::OutputLimit);
    let mut query = open(r"printf '\377\n'", Duration::from_secs(2), 100);
    assert_eq!(query.next().unwrap_err(), CommandError::InvalidOutput);
}
#[test]
fn query_deadline_covers_blocked_writes_and_live_output_waits() {
    let mut query = open("exec sleep 30", Duration::from_millis(80), 100);
    assert_eq!(
        query.send(&vec![b'x'; 200000]).unwrap_err(),
        CommandError::Timeout
    );
    let mut query = open("exec sleep 30", Duration::from_millis(80), 100);
    assert_eq!(query.next().unwrap_err(), CommandError::Timeout);
}
#[test]
fn dropping_a_successfully_answering_query_still_kills_and_reaps_its_server() {
    let mut query = open("echo $$; read wait", Duration::from_secs(2), 100);
    let pid: i32 = query.next().unwrap().unwrap().trim().parse().unwrap();
    drop(query);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
        -1
    );
}
