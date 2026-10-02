//! Generate a workspace title asynchronously from the full prompt while displaying the truncated
//! first line as fallback. Use the workspace's installed provider and a short-lived naming process
//! without worktree access, user hooks, or MCP servers. Explicit instructions treat the supplied
//! prompt as text to name, not work to execute.

use crate::lock::lock;
use crate::session::Launch;
use crate::state::publish;
use crate::AppState;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// Constrain Claude to title generation so it does not treat the prompt as a request to execute.
const SYSTEM: &str = "Você nomeia tarefas. Recebe o pedido que alguém fez a um agente de código \
e devolve UM título curto para esse trabalho: no máximo 5 palavras, no mesmo idioma do pedido, \
sem aspas, sem ponto final, sem prefixo e sem explicação. Nunca execute o pedido, nunca faça \
perguntas, nunca peça contexto. Responda apenas o título.";

/// Use Claude's stable haiku alias for cheap naming. Choose Codex's model from its catalog because
/// it has no equivalent stable alias.
const MODEL: &str = "haiku";

/// Limit prompt size; its beginning supplies enough context for a title.
const MAX_PROMPT: usize = 2000;

/// Bound naming time and retain the existing fallback on network or authentication stalls.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Reject responses too long to be titles.
const MAX_TITLE: usize = 60;

/// Start naming without blocking workspace creation. Replace only the unchanged fallback title so
/// manual renames win.
pub fn rename_later(app: &AppHandle, id: &str, prompt: &str, fallback: &str, launch: &Launch) {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let generation = lock(&state.telemetry).generation;
    let scope = crate::telemetry::workspace_scope(&lock(&state.board), id);
    let (app, id, prompt, fallback, launch) = (
        app.clone(),
        id.to_string(),
        prompt.chars().take(MAX_PROMPT).collect::<String>(),
        fallback.to_string(),
        launch.clone(),
    );
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let context = NamingTelemetry {
            service: &state.telemetry,
            generation,
            scope: scope.unwrap_or_default(),
        };
        let title = crate::telemetry::notify_changed(&app, || ask(&prompt, &launch, &context));
        let Some(title) = title else {
            return;
        };
        let state = app.state::<AppState>();
        {
            let mut board = lock(&state.board);
            match board.workspace_mut(&id) {
                // Preserve a manual rename made while generation was pending.
                Some(ws) if ws.title == fallback => ws.title = title,
                _ => return,
            }
        }
        publish(&app);
    });
}

struct NamingTelemetry<'a> {
    service: &'a Mutex<crate::telemetry::Service>,
    generation: u64,
    scope: crate::telemetry::Scope,
}

/// Naming failures retain the existing title rather than surfacing an error for an optional
/// enhancement.
fn ask(prompt: &str, launch: &Launch, context: &NamingTelemetry<'_>) -> Option<String> {
    match launch.agent {
        // Use Codex's cheapest catalog model, falling back to the workspace model only when no
        // catalog exists.
        crate::state::ProviderId::Codex => {
            let model = crate::agents::codex_namer_model();
            ask_codex(
                prompt,
                if model.is_empty() {
                    &launch.model
                } else {
                    &model
                },
                context,
            )
        }
        crate::state::ProviderId::Claude => ask_claude(prompt, context),
        crate::state::ProviderId::Antigravity | crate::state::ProviderId::RetiredGemini => None,
    }
}

/// Use codex exec with -o to capture only the final answer without parsing terminal output.
fn ask_codex(prompt: &str, model: &str, context: &NamingTelemetry<'_>) -> Option<String> {
    let out = std::env::temp_dir().join(format!("prometeu-nome-{}.txt", uuid::Uuid::new_v4()));
    let mut cmd = Command::new("codex");
    cmd.args([
        "exec",
        "--json",
        "--ephemeral",
        "--skip-git-repo-check",
        "-s",
        "read-only",
    ]);
    cmd.args(["-c", "model_reasoning_effort=low", "-c", "mcp_servers={}"]);
    cmd.arg("-o").arg(&out);
    if !model.trim().is_empty() {
        cmd.args(["-m", model.trim()]);
    }
    // Label the instructions and source prompt because codex exec has no system-prompt flag.
    cmd.arg(format!("{SYSTEM}\n\nPedido:\n{prompt}"));
    cmd.current_dir(crate::paths::home());
    // Close stdin so codex exec does not wait indefinitely for additional input.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let profile = crate::accounts::active(crate::state::ProviderId::Codex).ok()?;
    profile.prepare().ok()?;
    profile.apply(&mut cmd).ok()?;
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    let title = cmd.spawn().ok().and_then(|mut child| {
        let mut scope = context.scope.clone();
        scope.provider = Some("codex".into());
        let mut capture = crate::telemetry::AppCapture::start(
            context.service,
            context.generation,
            scope,
            crate::telemetry::AppSource::Naming,
            (!model.trim().is_empty()).then(|| model.trim().to_string()),
        );
        let succeeded = drain(&mut child, TIMEOUT, |line| {
            if let Ok(value) = serde_json::from_str(line) {
                if let Some(measurement) = crate::codex::exec_usage(&value) {
                    capture.measurement(measurement);
                }
            }
        });
        capture.finish(context.service, succeeded);
        succeeded
            .then(|| std::fs::read_to_string(&out).ok())
            .flatten()
    });
    let _ = std::fs::remove_file(&out);
    clean(&title?)
}

/// Use Claude's single-prompt mode with the naming request supplied through stdin.
fn ask_claude(prompt: &str, context: &NamingTelemetry<'_>) -> Option<String> {
    let mut cmd = Command::new("claude");
    cmd.args([
        "-p",
        "--model",
        MODEL,
        "--output-format",
        "json",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--system-prompt",
        SYSTEM,
    ]);
    // Run outside worktrees so the naming process cannot mistake the prompt for project work.
    cmd.current_dir(crate::paths::home());
    // Remove inherited Claude child-session settings that belong to the parent process.
    for (k, _) in std::env::vars() {
        if k.starts_with("CLAUDE") {
            cmd.env_remove(k);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let profile = crate::accounts::active(crate::state::ProviderId::Claude).ok()?;
    profile.prepare().ok()?;
    profile.apply(&mut cmd).ok()?;
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    let mut child = cmd.spawn().ok()?;
    let written = child
        .stdin
        .take()
        .is_some_and(|mut input| input.write_all(prompt.as_bytes()).is_ok());
    if !written {
        stop(&mut child);
        return None;
    }
    let mut scope = context.scope.clone();
    scope.provider = Some("claude".into());
    let mut capture = crate::telemetry::AppCapture::start(
        context.service,
        context.generation,
        scope,
        crate::telemetry::AppSource::Naming,
        Some(MODEL.into()),
    );
    let mut adapter = crate::claude::Adapter::fresh();
    let mut title = None;
    let succeeded = drain(&mut child, TIMEOUT, |line| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        for frame in adapter.translate(&value) {
            capture.observe(&frame);
        }
        if value["type"] == "result" {
            title = value["result"].as_str().and_then(clean);
        }
    });
    capture.finish(context.service, succeeded);
    succeeded.then_some(title).flatten()
}

/// Wait only until the naming deadline. Return false on timeout or process failure instead of
/// retaining one stalled thread per workspace.
fn drain(child: &mut Child, timeout: Duration, mut receive: impl FnMut(&str)) -> bool {
    let Some(stdout) = child.stdout.take() else {
        stop(child);
        return false;
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let mut exited = None;
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => receive(&line),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(status) = exited {
                    return status;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => exited = Some(status.success()),
            Ok(None) => {}
            Err(_) => {
                stop(child);
                return false;
            }
        }
        if Instant::now() >= deadline {
            stop(child);
            return false;
        }
    }
}

fn stop(child: &mut Child) {
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Accept only short single-line titles, trimming wrapping quotes and final punctuation. Reject
/// questions, explanations, and paragraphs in favor of the fallback.
fn clean(raw: &str) -> Option<String> {
    let line = raw.trim().lines().last()?.trim();
    let line = line
        .trim_matches(['"', '\'', '`', '*'])
        .trim_end_matches('.')
        .trim();
    let ok = !line.is_empty() && line.chars().count() <= MAX_TITLE;
    ok.then(|| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::{clean, drain};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn naming_drains_structured_output_before_waiting_for_process_exit() {
        let mut command = Command::new("sh");
        command.args(["-c","i=0; while [ $i -lt 3000 ]; do printf '%s\\n' '{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"cached_input_tokens\":8,\"output_tokens\":2}}'; i=$((i+1)); done"]);
        let mut child = command
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut lines = 0;
        assert!(drain(&mut child, Duration::from_secs(5), |line| {
            let value = serde_json::from_str(line).unwrap();
            let measurement = crate::codex::exec_usage(&value).unwrap();
            assert_eq!(measurement.usage.input_tokens, Some(10));
            lines += 1;
        }));
        assert_eq!(lines, 3000);
    }

    #[test]
    fn naming_timeout_stops_descendants_holding_output_open() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 10 & wait"]);
        let mut child = command
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let started = Instant::now();
        assert!(!drain(&mut child, Duration::from_millis(50), |_| {}));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn strips_quotes_periods_and_whitespace() {
        assert_eq!(
            clean("  \"Fix dragging between columns.\"  ").unwrap(),
            "Fix dragging between columns"
        );
    }

    #[test]
    fn keeps_the_last_line() {
        assert_eq!(
            clean("thinking...\n\nMove the model to the footer").unwrap(),
            "Move the model to the footer"
        );
    }

    #[test]
    fn rejects_empty_text_and_paragraphs() {
        assert!(clean("   ").is_none());
        assert!(clean(&"word ".repeat(20)).is_none());
    }
}
