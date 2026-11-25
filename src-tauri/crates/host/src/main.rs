//! `prometeu-host serve|attach|hook|version [--root DIR]`.
//!
//! - `serve` runs the resident host on `<root>/host-v<protocol>.sock`, one per root.
//! - `attach` is what the desktop runs over SSH: it starts the host when needed and relays its
//!   stdin/stdout to the socket, so the desktop's connection can drop without touching execution.
//! - `hook` is the command native CLI hooks run. It forwards the hook payload to the host and
//!   prints whatever decision the desktop returns.
//! - `agents setup [--socket PATH] [--rotate] [--client-uid ID]` enables agent clients: it writes
//!   the agent socket's path and the token they must present, and prints an MCP configuration
//!   (ADR 0103).
//! - `mcp [--socket PATH]` is a stdio MCP server over the agent socket, for bot platforms.
mod agents;
mod client;
mod mcp;
mod registry;
mod server;

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let mut root = None;
    let mut socket = None;
    let mut rotate = false;
    let mut client_uid: Option<String> = None;
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(std::path::PathBuf::from),
            "--socket" => socket = args.next().map(std::path::PathBuf::from),
            "--rotate" => rotate = true,
            "--client-uid" => client_uid = args.next().filter(|v| v.parse::<u32>().is_ok()),
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
                    let mut env = serde_json::json!({
                        "PROMETEU_AGENT_TOKEN": token,
                        "PROMETEU_AGENT_NAME": "<bot name>",
                    });
                    // The client checks the listener's user ID as its own user namespace sees it,
                    // which only the person deploying it knows for a container.
                    if let Some(uid) = &client_uid {
                        env["PROMETEU_AGENT_HOST_UID"] = serde_json::json!(uid);
                    }
                    let config = serde_json::json!({
                        "command": exe,
                        "args": ["mcp", "--socket", socket.display().to_string()],
                        "env": env,
                    });
                    // SAFETY: getuid has no preconditions.
                    let uid = unsafe { libc::getuid() };
                    println!("Agent socket: {}", socket.display());
                    println!("Token: {token}");
                    println!(
                        "Anyone with this token can run commands as this user through Claude."
                    );
                    println!("Only this user may be able to write to the socket's directory.");
                    if client_uid.is_none() {
                        println!("To make the client check who serves the socket, run again with");
                        println!(
                            "--client-uid <ID>: this user's ID as the client sees it. On this"
                        );
                        println!(
                            "machine it is {uid}; inside a rootless container it is usually 65534."
                        );
                    }
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
