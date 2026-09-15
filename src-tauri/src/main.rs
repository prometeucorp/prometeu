#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod actions;
mod agents;
mod awake;
mod browser;
mod catalog;
mod chat;
mod claude;
mod cloud;
mod codex;
mod conversation;
mod dock;
mod domain;
mod feedback;
mod file_drop;
mod github;
mod i18n;
mod linear;
mod lock;
mod machine;
mod mcp;
mod mcp_auth;
mod naming;
mod oauth;
mod paths;
mod plugins;
mod pty;
mod scripts;
mod selection;
mod session;
mod skills;
mod state;
mod team;
mod transcript;
mod usage;

use state::Board;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub struct AppState {
    pub board: Mutex<Board>,
    /// A single saver thread coalesces board writes; see state::spawn_saver.
    pub save: state::Saver,
    /// Running conversations keyed by their session/tab IDs, independent of the displayed
    /// workspace.
    pub chats: Mutex<HashMap<String, chat::Chat>>,
    /// Dock terminals, setup, and run processes keyed by workspace:type.
    pub ptys: Mutex<HashMap<String, pty::Pty>>,
    /// The visible workspace does not acquire unread status for live updates.
    pub looking: Mutex<Option<String>>,
    /// Only ready agents may receive their initial message. Setup completion must not write to a
    /// process that is still starting.
    pub ready: Mutex<HashSet<String>>,
}

/// Finder launches inherit launchd's minimal PATH. Adopt the user's login-shell PATH so agents and
/// Homebrew tools can be found; terminal launches retain equivalent behavior.
fn adopt_login_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = std::process::Command::new(shell)
            .args(["-ilc", r#"printf %s "$PATH""#])
            .output();
        let path = out
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let _ = tx.send(path);
    });
    // Bound login-shell execution because arbitrary user profile code may stall.
    if let Ok(Some(path)) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
        if !path.is_empty() {
            std::env::set_var("PATH", path);
        }
    }
}

/// Install the rustls crypto provider before any HTTPS client starts. The updater would otherwise
/// install it only during its first check, leaving earlier OAuth or MCP requests vulnerable to a
/// runtime panic. An existing provider is already sufficient.
fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn main() {
    install_crypto();
    adopt_login_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            board: Mutex::new(Board::load()),
            save: state::spawn_saver(),
            chats: Mutex::new(HashMap::new()),
            ptys: Mutex::new(HashMap::new()),
            looking: Mutex::new(None),
            ready: Mutex::new(HashSet::new()),
        })
        .invoke_handler(tauri::generate_handler![
            actions::actions_save,
            actions::action_start,
            actions::action_pause,
            i18n::set_lang,
            agents::agents,
            agents::claude_models,
            accounts::accounts,
            accounts::account_select,
            accounts::account_remove,
            accounts::account_login,
            accounts::account_login_cancel,
            usage::usage,
            machine::machine,
            awake::set_awake,
            session::load_board,
            session::add_project,
            session::remove_project,
            session::list_branches,
            session::create_workspace,
            session::set_stage,
            session::archive_workspace,
            session::finish_workspace,
            session::cleanup_worktree,
            session::cleanup_list,
            session::pin_workspace,
            session::set_workspace_mcp,
            session::set_workspace_plugins,
            session::set_workspace_skills,
            session::set_tools_global,
            session::workspace_tools,
            session::project_tools,
            session::project_tools_trust,
            session::set_tab_choice,
            session::set_unread,
            session::set_shared,
            session::look_at,
            session::rename_workspace,
            session::remove_workspace,
            session::diff::workspace_diff,
            session::diff::workspace_branch,
            session::git::workspace_git_status,
            session::git::workspace_git_diff,
            session::git::workspace_git_action,
            session::git::workspace_git_history,
            session::git::workspace_git_branches,
            session::git::workspace_git_conflict,
            session::git::workspace_git_resolve,
            session::find::find_paths,
            session::files::list_dir,
            session::files::read_file,
            session::files::read_bytes,
            session::files::file_stamp,
            session::files::write_file,
            dock::open_dock,
            dock::close_dock,
            dock::reveal,
            dock::workspace_scripts,
            dock::dock_state,
            dock::create_scripts_file,
            dock::scripts_prompt,
            dock::open_run,
            browser::browser_open,
            browser::browser_url,
            browser::browser_navigate,
            browser::browser_bounds,
            browser::browser_hide,
            browser::browser_back,
            browser::browser_forward,
            browser::browser_reload,
            browser::browser_close,
            browser::browser_inspect,
            browser::browser_selection,
            browser::browser_capture,
            browser::open_external,
            feedback::feedback_capture,
            feedback::feedback_send,
            file_drop::paste_files,
            session::pr_prompt,
            github::pr_open,
            github::refresh_prs,
            github::open_pr,
            session::new_tab,
            session::close_tab,
            session::focus_tab,
            session::rename_tab,
            session::resume_tab,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_buffer,
            chat::chat_send,
            chat::chat_control,
            chat::chat_control_remote,
            chat::chat_snapshot,
            mcp::mcp_hub,
            mcp::mcp_save,
            mcp::mcp_remove,
            mcp::mcp_found,
            session::mcp_inherited,
            mcp::mcp_check,
            mcp::mcp_login,
            mcp::mcp_logout,
            mcp::mcp_logins,
            plugins::plugin_hub,
            plugins::plugin_save,
            plugins::plugin_remove,
            plugins::plugin_look,
            plugins::plugin_install,
            plugins::plugin_scrap,
            plugins::plugin_update,
            plugins::plugin_make,
            plugins::plugin_make_stop,
            linear::linear_status,
            linear::linear_connect,
            linear::linear_disconnect,
            linear::linear_issues,
            linear::linear_open,
            team::team_config,
            team::team_config_set,
            team::team_security,
            team::team_security_set,
            cloud::cloud_status,
            cloud::cloud_organizations,
            cloud::cloud_relay_ticket,
            cloud::cloud_login_start,
            cloud::cloud_login_poll,
            cloud::cloud_login_cancel,
            cloud::cloud_logout,
            catalog::catalog_state,
            catalog::catalog_refresh,
            catalog::catalog_share,
            catalog::catalog_copy,
            catalog::catalog_install_plugin,
            catalog::catalog_install_skill,
            catalog::catalog_install_organization_item,
            skills::skill_hub,
            skills::skill_save,
            skills::skill_remove,
        ])
        .setup(|app| {
            file_drop::install(app.handle())?;
            actions::watch(app.handle().clone());
            machine::watch(app.handle().clone());
            usage::watch(app.handle().clone());
            Ok(())
        })
        .on_webview_event(file_drop::on_webview_event)
        .build(tauri::generate_context!())
        .expect("erro ao subir o Prometeu")
        .run(|app, event| {
            // Flush deferred board writes during shutdown, when no later save can be assumed.
            if matches!(event, tauri::RunEvent::Exit) {
                accounts::shutdown();
                state::save_now(app);
            }
        });
}
