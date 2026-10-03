//! Generate a workspace title asynchronously from the full prompt while displaying the truncated
//! first line as fallback. Use the workspace's installed provider and a short-lived naming process
//! without worktree access, user hooks, or MCP servers. Explicit instructions treat the supplied
//! prompt as text to name, not work to execute.

use crate::lock::lock;
use crate::session::Launch;
use crate::state::publish;
use crate::AppState;
use prometeu_core::command::{CommandPolicy, CommandRunner, OutputPolicy};
use std::process::Command;
use std::time::Duration;
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
    let (app, id, prompt, fallback, launch) = (
        app.clone(),
        id.to_string(),
        prompt.chars().take(MAX_PROMPT).collect::<String>(),
        fallback.to_string(),
        launch.clone(),
    );
    std::thread::spawn(move || {
        let runner = app.state::<AppState>().command_runner.clone();
        let Some(title) = ask(runner.as_ref(), &prompt, &launch) else {
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

/// Naming failures retain the existing title rather than surfacing an error for an optional
/// enhancement.
fn ask(runner: &dyn CommandRunner<Command>, prompt: &str, launch: &Launch) -> Option<String> {
    match launch.agent {
        // Use Codex's cheapest catalog model, falling back to the workspace model only when no
        // catalog exists.
        crate::state::ProviderId::Codex => {
            let model = crate::agents::codex_namer_model();
            ask_codex(
                runner,
                prompt,
                if model.is_empty() {
                    &launch.model
                } else {
                    &model
                },
            )
        }
        crate::state::ProviderId::Claude => ask_claude(runner, prompt),
        crate::state::ProviderId::Antigravity | crate::state::ProviderId::RetiredGemini => None,
    }
}

/// Use codex exec with -o to capture only the final answer without parsing terminal output.
fn ask_codex(runner: &dyn CommandRunner<Command>, prompt: &str, model: &str) -> Option<String> {
    let out = std::env::temp_dir().join(format!("prometeu-nome-{}.txt", uuid::Uuid::new_v4()));
    let mut cmd = Command::new("codex");
    cmd.args([
        "exec",
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
    let profile = crate::accounts::active(crate::state::ProviderId::Codex).ok()?;
    crate::accounts::prepare_profile(&profile).ok()?;
    crate::accounts::apply_profile(&profile, &mut cmd).ok()?;
    let title = runner
        .run(
            &mut cmd,
            &[],
            CommandPolicy {
                timeout: TIMEOUT,
                stdout: OutputPolicy::Discard,
                stderr: OutputPolicy::Discard,
            },
        )
        .ok()
        .filter(|output| output.success)
        .and_then(|_| std::fs::read_to_string(&out).ok());
    let _ = std::fs::remove_file(&out);
    clean(&title?)
}

/// Use Claude's single-prompt mode with the naming request supplied through stdin.
fn ask_claude(runner: &dyn CommandRunner<Command>, prompt: &str) -> Option<String> {
    let mut cmd = Command::new("claude");
    cmd.args([
        "-p",
        "--model",
        MODEL,
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
    let profile = crate::accounts::active(crate::state::ProviderId::Claude).ok()?;
    crate::accounts::prepare_profile(&profile).ok()?;
    crate::accounts::apply_profile(&profile, &mut cmd).ok()?;
    title_from_command(runner, &mut cmd, prompt)
}

fn title_from_command(
    runner: &dyn CommandRunner<Command>,
    command: &mut Command,
    prompt: &str,
) -> Option<String> {
    let output = runner
        .run(
            command,
            prompt.as_bytes(),
            CommandPolicy {
                timeout: TIMEOUT,
                stdout: OutputPolicy::Capture { limit: 1_048_576 },
                stderr: OutputPolicy::Discard,
            },
        )
        .ok()?;
    output
        .success
        .then(|| clean(&String::from_utf8_lossy(&output.stdout)))?
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
    use super::clean;

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

#[cfg(test)]
mod command_port_tests {
    use super::*;
    use prometeu_core::command::{CommandError, CommandOutput};
    struct Runner(bool);
    impl CommandRunner<Command> for Runner {
        fn run(
            &self,
            _: &mut Command,
            input: &[u8],
            policy: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            assert_eq!(input, b"name this task");
            assert_eq!(policy.timeout, TIMEOUT);
            assert_eq!(policy.stdout, OutputPolicy::Capture { limit: 1_048_576 });
            assert_eq!(policy.stderr, OutputPolicy::Discard);
            match self.0 {
                true => Ok(CommandOutput {
                    success: true,
                    stdout: b"\"Fix terminal output.\"\n".to_vec(),
                    stderr: vec![],
                }),
                false => Err(CommandError::Timeout),
            }
        }
    }
    #[test]
    fn injected_naming_results_keep_title_cleaning_and_timeout_fallback() {
        assert_eq!(
            title_from_command(&Runner(true), &mut Command::new("unused"), "name this task")
                .as_deref(),
            Some("Fix terminal output")
        );
        assert_eq!(
            title_from_command(
                &Runner(false),
                &mut Command::new("unused"),
                "name this task"
            ),
            None
        );
    }
}
