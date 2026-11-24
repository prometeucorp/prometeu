//! `prometeu-host serve|attach|hook|version [--root DIR]`.
//!
//! - `serve` runs the resident host on `<root>/host-v<protocol>.sock`, one per root.
//! - `attach` is what the desktop runs over SSH: it starts the host when needed and relays its
//!   stdin/stdout to the socket, so the desktop's connection can drop without touching execution.
//! - `hook` is the command native CLI hooks run. It forwards the hook payload to the host and
//!   prints whatever decision the desktop returns.
//! - `agents setup [--socket PATH] [--rotate]` enables agent clients: it writes the agent socket's
//!   path and the token they must present, and prints an MCP configuration (ADR 0103).
//! - `mcp [--socket PATH]` is a stdio MCP server over the agent socket, for bot platforms.
#[cfg(unix)]
mod agents;
#[cfg(unix)]
mod client;
#[cfg(unix)]
mod mcp;
#[cfg(unix)]
mod registry;
#[cfg(unix)]
mod server;

#[cfg(unix)]
fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let mut root = None;
    let mut socket = None;
    let mut rotate = false;
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(std::path::PathBuf::from),
            "--socket" => socket = args.next().map(std::path::PathBuf::from),
            "--rotate" => rotate = true,
            _ => rest.push(arg),
        }
    }
    let root = root.unwrap_or_else(client::default_root);
    let code = match command.as_str() {
        "serve" => match server::serve(&root) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("prometeu-host: {error}");
                1
            }
        },
        "attach" => match client::attach(&root) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("prometeu-host: {error}");
                1
            }
        },
        // A hook failure must never change the native CLI's behaviour: exit cleanly with no output.
        "hook" => {
            let _ = client::hook(&root);
            0
        }
        "agents" if rest.first().map(String::as_str) == Some("setup") => {
            match agents::setup(&root, socket.as_deref(), rotate) {
                Ok((token, socket)) => {
                    let exe = std::env::current_exe()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| "prometeu-host".into());
                    let config = serde_json::json!({
                        "command": exe,
                        "args": ["mcp", "--socket", socket.display().to_string()],
                        "env": {"PROMETEU_AGENT_TOKEN": token, "PROMETEU_AGENT_NAME": "<bot name>"},
                    });
                    println!("Agent socket: {}", socket.display());
                    println!("Token: {token}");
                    println!(
                        "Anyone with this token can run commands as this user through Claude."
                    );
                    println!("MCP server configuration (adjust paths as seen from the client):");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&config).unwrap_or_default()
                    );
                    0
                }
                Err(error) => {
                    eprintln!("prometeu-host: {error}");
                    1
                }
            }
        }
        "mcp" => {
            let socket = socket.unwrap_or_else(|| agents::socket_path(&root));
            match mcp::run(socket) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("prometeu-host: {error}");
                    1
                }
            }
        }
        "version" => {
            println!(
                "{}",
                serde_json::json!({"protocol": prometeu_host::PROTOCOL, "version": env!("CARGO_PKG_VERSION")})
            );
            0
        }
        _ => {
            eprintln!("usage: prometeu-host serve|attach|hook|version|mcp|agents setup [--root DIR] [--socket PATH] [--rotate]");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(not(unix))]
fn main() {
    eprintln!("prometeu-host runs only on Unix hosts");
    std::process::exit(2);
}
