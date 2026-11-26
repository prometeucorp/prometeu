//! `prometeu-host serve|attach|hook|version [--root DIR]`.
//!
//! - `serve` runs the resident host on `<root>/host-v<protocol>.sock`, one per root.
//! - `attach` is what the desktop runs over SSH: it starts the host when needed and relays its
//!   stdin/stdout to the socket, so the desktop's connection can drop without touching execution.
//! - `hook` is the command native CLI hooks run. It forwards the hook payload to the host and
//!   prints whatever decision the desktop returns.
//! - `agents setup [--socket PATH] [--rotate] [--client-uid ID] [--json]` enables
//!   agent clients:
//!   it prepares the agent socket's directory and client executable, writes the token they must
//!   present, and prints a local stdio MCP configuration (ADR 0103). `agents status` and
//!   `agents disable` report and undo it.
//! - `mcp [--socket PATH]` is a stdio MCP server over the agent socket, for any MCP client.
//! - `codex-relay --socket PATH --pid PID` relays a conversation's Codex engine to the desktop as
//!   newline-delimited JSON on its terminal (ADR 0105).
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
    let mut device = None;
    let mut label = None;
    let mut release = None;
    let mut pid = None;
    let mut options = AgentOptions::default();
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(std::path::PathBuf::from),
            "--socket" => {
                socket = args.next().map(std::path::PathBuf::from);
                options.socket = socket.clone();
            }
            "--device" => device = args.next(),
            "--label" => label = args.next(),
            "--release" => release = args.next(),
            "--pid" => pid = args.next().and_then(|pid| pid.parse::<u32>().ok()),
            "--rotate" => options.rotate = true,
            "--json" => options.json = true,
            "--open-home" => {
                eprintln!("prometeu-host: --open-home is no longer supported; configure filesystem access outside Prometeu");
                std::process::exit(2);
            }
            // A bad value must not silently produce a configuration without the check.
            "--client-uid" => match args.next() {
                Some(uid) if uid.parse::<u32>().is_ok() => options.client_uid = uid.parse().ok(),
                _ => {
                    eprintln!("prometeu-host: --client-uid requires a numeric user ID");
                    std::process::exit(2);
                }
            },
            _ => rest.push(arg),
        }
    }
    let root = root.unwrap_or_else(client::default_root);
    let code = match command.as_str() {
        "serve" => match server::serve(&root, release.as_deref()) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("prometeu-host: {error}");
                1
            }
        },
        "attach" => match client::attach(
            &root,
            device.as_deref(),
            label.as_deref(),
            release.as_deref(),
        ) {
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
        "agents" => agents_command(&root, rest.first().map(String::as_str), &options),
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
        // Started by the desktop as a terminal next to a conversation's Codex engine (ADR 0105).
        "codex-relay" => match (socket, pid) {
            (Some(socket), Some(pid)) => {
                match prometeu_host::codex::relay(
                    &socket,
                    pid,
                    prometeu_host::codex::CONNECT_TIMEOUT,
                ) {
                    Ok(()) => 0,
                    Err(error) => {
                        eprintln!("prometeu-host: {error}");
                        1
                    }
                }
            }
            _ => {
                eprintln!("usage: prometeu-host codex-relay --socket PATH --pid PID");
                2
            }
        },
        "version" => {
            println!(
                "{}",
                serde_json::json!({"protocol": prometeu_host::PROTOCOL, "version": env!("CARGO_PKG_VERSION")})
            );
            0
        }
        _ => {
            eprintln!("usage: prometeu-host serve|attach|hook|version|mcp|codex-relay|agents setup|status|disable [--root DIR] [--device ID] [--label NAME] [--release VERSION] [--socket PATH] [--rotate] [--client-uid ID] [--json]");
            2
        }
    };
    std::process::exit(code);
}

#[derive(Default)]
struct AgentOptions {
    socket: Option<std::path::PathBuf>,
    rotate: bool,
    json: bool,
    client_uid: Option<u32>,
}

fn agents_command(root: &std::path::Path, action: Option<&str>, options: &AgentOptions) -> i32 {
    let fail = |error: String| {
        eprintln!("prometeu-host: {error}");
        1
    };
    match action {
        Some("status") => {
            let status = agents::status(root);
            match options.json {
                true => println!("{status}"),
                false => {
                    let enabled = status["enabled"] == serde_json::json!(true);
                    println!(
                        "Agents: {}\nSocket: {}",
                        if enabled { "enabled" } else { "disabled" },
                        status["socket"].as_str().unwrap_or_default()
                    );
                    if let Some(error) = status["error"].as_str() {
                        println!("The host cannot serve it: {error}");
                    }
                }
            }
            0
        }
        Some("disable") => {
            match agents::disable(root) {
                Ok(()) => {
                    match options.json {
                    true => println!("{{}}"),
                    false => println!("Agents disabled: the token is gone and the socket closes within seconds."),
                }
                    0
                }
                Err(error) => fail(error),
            }
        }
        Some("setup") => match agents::setup(root, options.socket.as_deref(), options.rotate) {
            Ok(setup) => {
                // SAFETY: getuid has no preconditions.
                let local_uid = unsafe { libc::getuid() };
                // Local clients see the host's real identity; integrations may override it.
                let client_uid = options.client_uid.unwrap_or(local_uid);
                let config = agents::client_config(&setup, Some(client_uid), "agent");
                if options.json {
                    let mut out = config;
                    out["token"] = serde_json::json!(setup.token);
                    out["socket"] = serde_json::json!(setup.socket.display().to_string());
                    out["client"] = serde_json::json!(setup.client.display().to_string());
                    out["moved_from"] = serde_json::json!(setup
                        .moved_from
                        .as_ref()
                        .map(|p| p.display().to_string()));
                    println!("{out}");
                    return 0;
                }
                println!("Agent socket: {}", setup.socket.display());
                println!(
                    "Client: {} (kept current by the host)",
                    setup.client.display()
                );
                if let Some(old) = &setup.moved_from {
                    println!(
                        "The agent socket moved from {}; clients configured with that path need the configuration below.",
                        old.display()
                    );
                }
                println!("Token: {}", setup.token);
                println!("Anyone with this token can run commands as this user through Claude.");
                println!();
                println!("MCP server configuration:");
                let servers = serde_json::json!({"mcpServers": {"prometeu": config["server"]}});
                println!(
                    "{}",
                    serde_json::to_string_pretty(&servers).unwrap_or_default()
                );
                0
            }
            Err(error) => fail(error),
        },
        _ => {
            eprintln!("usage: prometeu-host agents setup|status|disable [--root DIR] [--socket PATH] [--rotate] [--client-uid ID] [--json]");
            2
        }
    }
}
