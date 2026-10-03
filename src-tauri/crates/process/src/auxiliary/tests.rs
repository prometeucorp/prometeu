use super::*;
fn open(script: &str) -> Box<dyn AuxiliaryProcess> {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    UnixAuxiliaryLauncher.launch(command).unwrap()
}
fn line(process: &mut dyn AuxiliaryProcess) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "private output timed out");
        match process.line(Duration::from_millis(50)).unwrap() {
            LinePoll::Line(line) => return Some(line),
            LinePoll::Pending => {}
            LinePoll::Closed => return None,
        }
    }
}
#[test]
fn private_input_output_and_exit_preserve_line_semantics() {
    let mut process = open("read value; printf '%s\\r\\n' \"$value\"; printf ignored >&2; exit 7");
    process.send(b"request\n").unwrap();
    assert_eq!(line(process.as_mut()).as_deref(), Some("request"));
    assert_eq!(line(process.as_mut()), None);
    assert_eq!(process.wait(Duration::from_secs(2)).unwrap(), Some(false));
    assert_eq!(process.wait(Duration::ZERO).unwrap(), Some(false));
}
#[test]
fn dropping_private_io_terminates_the_live_child() {
    let mut process = open("echo $$; exec sleep 30");
    let pid = line(process.as_mut()).unwrap();
    drop(process);
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
}
#[test]
fn oversized_private_lines_close_without_delivering_partial_protocol_frames() {
    let mut process = open("head -c 1048577 /dev/zero | tr '\\000' x; printf '\\n'");
    assert_eq!(line(process.as_mut()), None);
}
