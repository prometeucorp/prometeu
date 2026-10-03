use super::*;
use std::io::{Read, Write};
use std::sync::mpsc;

fn open(script: &str) -> StartedTerminal {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", script]);
    UnixTerminalFactory
        .open(command, TerminalSize { cols: 80, rows: 24 })
        .unwrap()
}
fn drain(mut reader: Box<dyn Read + Send>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut all = vec![];
        let mut buf = [0; 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }
        let _ = tx.send(all);
    });
    rx
}
#[test]
fn terminal_output_exit_code_and_repeated_wait_are_retained() {
    let mut terminal = open("printf service-output; exit 7");
    let output = drain(terminal.output)
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(terminal.waiter.wait().unwrap(), 7);
    assert_eq!(terminal.waiter.wait().unwrap(), 7);
    assert!(!terminal.control.running());
    assert_eq!(output, b"service-output");
    terminal.control.close();
}
#[test]
fn terminal_input_and_resizing_reach_the_child() {
    let mut terminal = open("read value; printf 'answer=%s size=' \"$value\"; stty size");
    terminal
        .control
        .resize(TerminalSize { cols: 93, rows: 37 })
        .unwrap();
    terminal.input.write_all(b"hello\n").unwrap();
    terminal.input.flush().unwrap();
    let output = drain(terminal.output)
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(String::from_utf8_lossy(&output).contains("answer=hello size=37 93"));
    assert_eq!(terminal.waiter.wait().unwrap(), 0);
}
#[test]
fn terminal_eof_does_not_disable_shutdown_of_a_live_child() {
    let mut terminal =
        open("trap '' HUP TERM; printf ready; exec </dev/null >/dev/null 2>&1; exec sleep 30");
    let output = drain(terminal.output)
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(output, b"ready");
    assert!(terminal.control.running());
    terminal.control.close();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(terminal.waiter.wait());
    });
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_ok());
    assert!(!terminal.control.running());
}
fn running(pid: u32) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
    !stat.is_empty() && !stat.starts_with('Z')
}
#[test]
fn closing_a_terminal_stops_its_group_and_reaps_its_leader() {
    let mut terminal =
        open("trap '' HUP; nohup sleep 30 >/dev/null 2>&1 & echo CHILD=$!; exec sleep 30");
    let mut reader = std::io::BufReader::new(terminal.output);
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
    let child: u32 = line.trim().strip_prefix("CHILD=").unwrap().parse().unwrap();
    let pid = terminal.control.system_id();
    let drained = drain(Box::new(reader));
    terminal.control.close();
    drained.recv_timeout(Duration::from_secs(5)).unwrap();
    terminal.waiter.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while running(child) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running(pid));
    assert!(!running(child));
}
#[test]
fn abandoning_the_terminal_waiter_kills_and_reaps() {
    let terminal = open("exec sleep 30");
    let pid = terminal.control.system_id();
    let drained = drain(terminal.output);
    drop(terminal.waiter);
    drained.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!terminal.control.running());
    assert!(!running(pid));
}
