//! Bounded native WSL queries and package transfer, selected only by Windows composition.
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
pub trait WslCommands: Send + Sync {
    fn run(&self, args: &[&str], input: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, String>;
}
pub struct NativeWslCommands;
impl NativeWslCommands {
    pub fn command(args: &[&str]) -> Command {
        let mut command = Command::new("wsl.exe");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command.args(args);
        command
    }
}
impl WslCommands for NativeWslCommands {
    fn run(&self, args: &[&str], input: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, String> {
        run_process(Self::command(args), input, timeout)
    }
}
fn run_process(mut command: Command, input: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let (send, receive) = mpsc::channel();
    let mut stdin = child.stdin.take().unwrap();
    let writer = send.clone();
    std::thread::spawn(move || {
        let _ = writer.send((0, stdin.write_all(&input).map(|_| vec![])));
    });
    for (kind, stream) in [
        (
            1,
            Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        ),
        (
            2,
            Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>,
        ),
    ] {
        let send = send.clone();
        std::thread::spawn(move || {
            let mut bytes = vec![];
            let result = stream.take(65_537).read_to_end(&mut bytes).map(|_| bytes);
            let _ = send.send((kind, result));
        });
    }
    drop(send);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "WSL command timed out".into()));
            }
        }
    };
    let (mut output, mut error, mut write_error) = (vec![], vec![], None);
    for _ in 0..3 {
        let (kind, bytes) = receive
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "WSL output timed out")?;
        if kind == 0 {
            write_error = bytes.err();
            continue;
        }
        let bytes = bytes.map_err(|e| e.to_string())?;
        if bytes.len() > 65_536 {
            return Err("WSL output limit".into());
        }
        match kind {
            1 => output = bytes,
            _ => error = bytes,
        }
    }
    if !status.success() {
        return Err(String::from_utf8_lossy(&error).trim().to_owned());
    }
    if let Some(error) = write_error {
        return Err(error.to_string());
    }
    Ok(output)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn bounded_process_transfers_input_and_rejects_oversized_output() {
        let mut echo = Command::new("/bin/sh");
        echo.args(["-c", "cat"]);
        let bytes = "literal ' text ação".as_bytes().to_vec();
        assert_eq!(
            run_process(echo, bytes.clone(), Duration::from_secs(2)).unwrap(),
            bytes
        );
        let mut excess = Command::new("/bin/sh");
        excess.args(["-c", "head -c 100000 /dev/zero"]);
        assert!(run_process(excess, vec![], Duration::from_secs(2)).is_err());
    }
    #[test]
    fn blocked_input_does_not_extend_the_child_deadline() {
        let mut sleeper = Command::new("/bin/sh");
        sleeper.args(["-c", "exec sleep 10"]);
        let started = Instant::now();
        assert!(
            run_process(sleeper, vec![0; 1024 * 1024], Duration::from_millis(100))
                .unwrap_err()
                .contains("timed out")
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
