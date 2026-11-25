//! `prometeu-host serve|attach|hook|version [--root DIR]`.
//!
//! - `serve` runs the resident host on `<root>/host-v<protocol>.sock`, one per root.
//! - `attach` is what the desktop runs over SSH: it starts the host when needed and relays its
//!   stdin/stdout to the socket, so the desktop's connection can drop without touching execution.
//! - `hook` is the command native CLI hooks run. It forwards the hook payload to the host and
//!   prints whatever decision the desktop returns.
//! - `agents setup [--socket PATH] [--rotate] [--client-uid ID] [--json]` enables agent clients:
//!   it prepares the agent socket's directory and client executable, writes the token they must
//!   present, and prints an MCP configuration for a container (ADR 0103). `agents status` and
//!   `agents disable` report and undo it.
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
    let mut options = AgentOptions::default();
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(std::path::PathBuf::from),
            "--socket" => {
                socket = args.next().map(std::path::PathBuf::from);
                options.socket = socket.clone();
            }
            "--rotate" => options.rotate = true,
            "--json" => options.json = true,
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
        "version" => {
            println!(
                "{}",
                serde_json::json!({"protocol": prometeu_host::PROTOCOL, "version": env!("CARGO_PKG_VERSION")})
            );
            0
        }
        _ => {
            eprintln!("usage: prometeu-host serve|attach|hook|version|mcp|agents setup|status|disable [--root DIR] [--socket PATH] [--rotate] [--client-uid ID] [--json]");
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
            let socket = agents::socket_path(root);
            let status = serde_json::json!({
                "enabled": agents::enabled(root),
                "socket": socket.display().to_string(),
            });
            match options.json {
                true => println!("{status}"),
                false => println!(
                    "Agents: {}\nSocket: {}",
                    if agents::enabled(root) {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    socket.display()
                ),
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
                // The client checks the listener's user ID as its own user namespace sees it. The
                // default is how another user's rootless container sees this user; the output
                // says so, with the values for the other deployments.
                let client_uid = options.client_uid.unwrap_or_else(agents::overflow_uid);
                let config = agents::client_config(&setup, Some(client_uid), "agent");
                if options.json {
                    let mut out = config;
                    out["token"] = serde_json::json!(setup.token);
                    out["socket"] = serde_json::json!(setup.socket.display().to_string());
                    out["client"] = serde_json::json!(setup.client.display().to_string());
                    out["opened"] = serde_json::json!(setup
                        .opened
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>());
                    out["client_uid"] = serde_json::json!(client_uid);
                    out["local_uid"] = serde_json::json!(local_uid);
                    println!("{out}");
                    return 0;
                }
                let mount = &config["mount"];
                println!("Agent socket: {}", setup.socket.display());
                println!(
                    "Client: {} (kept current by the host)",
                    setup.client.display()
                );
                for folder in &setup.opened {
                    println!(
                        "Made {} traversable by other users (o+x, not readable) so a container can mount the agent folder.",
                        folder.display()
                    );
                }
                println!("Token: {}", setup.token);
                println!("Anyone with this token can run commands as this user through Claude.");
                println!();
                println!("Mount the agent folder into the bot's container:");
                println!(
                    "  -v {}:{}:ro",
                    mount["source"].as_str().unwrap_or_default(),
                    mount["target"].as_str().unwrap_or_default()
                );
                println!("MCP server configuration:");
                let servers = serde_json::json!({"mcpServers": {"prometeu": config["server"]}});
                println!(
                    "{}",
                    serde_json::to_string_pretty(&servers).unwrap_or_default()
                );
                if options.client_uid.is_none() {
                    println!(
                        "PROMETEU_AGENT_HOST_UID is {client_uid}: this user as another user's rootless container sees it. Use --client-uid {local_uid} for a client running directly on this machine, or --client-uid 0 in a rootless container of this user."
                    );
                }
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
