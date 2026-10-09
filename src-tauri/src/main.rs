#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account_login;
mod account_profiles;
mod account_store;
mod accounts;
mod actions;
mod agent_launch;
mod agents;
mod antigravity;
mod awake;
mod background;
mod board_store;

#[cfg(test)]
mod boundary_contract;
mod browser;
mod catalog;
mod chat;
mod claude;
mod cloud;
mod codex;
mod conversation;
mod delegation;
mod dock;
mod domain;
mod embedded_mcp;
mod evaluation;
mod feedback;
mod file_drop;
mod github;
mod github_auth;
mod github_issues;
mod github_notifications;
mod i18n;
mod island;
mod kickoff;
mod linear;
mod lock;
mod machine;
mod mcp;
mod mcp_access;
mod mcp_auth;
mod naming;
mod notifications;
mod oauth;
mod paths;
mod platform;
mod plugins;
mod pty;
mod review_calibration;
mod scripts;
mod selection;
mod session;
mod skills;
mod state;
mod team;
mod telemetry;
mod tool_materialization;
mod transcript;
mod transcript_store;
mod typesafe;
mod usage;
mod usage_scheduler;
mod workspace_lifecycle;
mod workspace_tools;

use prometeu_core::publication::{BoardPublisher, BoardStore};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub project_entries: Arc<dyn prometeu_core::files::ProjectEntries<std::path::Path>>,
    pub project_search: Arc<dyn prometeu_core::files::ProjectSearch>,
    pub worktree_cleanup: Arc<dyn prometeu_core::workspace_lifecycle::WorktreeCleanup>,
    pub repository_git: Arc<dyn prometeu_core::git::RepositoryGit>,
    pub authentication: Arc<dyn prometeu_core::accounts::login::AccountAuthentication>,
    pub providers:
        Arc<dyn prometeu_core::session::provider::ProviderPreparation<agent_launch::PreparedAgent>>,
    pub tasks: Arc<dyn prometeu_core::tasks::TaskExecutor>,
    pub command_runner: Arc<dyn prometeu_core::command::CommandRunner<std::process::Command>>,
    pub query_launcher: Arc<dyn prometeu_core::command::QueryLauncher<std::process::Command>>,
    pub terminal_factory:
        Arc<dyn prometeu_core::terminal::TerminalFactory<portable_pty::CommandBuilder>>,
    pub sessions: prometeu_core::session::host::SessionHost<chat::Chat>,
    pub process_launcher: Arc<dyn prometeu_core::process::ProcessLauncher<std::process::Command>>,
    pub telemetry: Mutex<telemetry::Service>,
    pub board: Mutex<state::Board>,
    /// One ordered persistence queue, with its store injected at composition.
    pub save: BoardPublisher,
    /// Dock terminals, setup, and run processes keyed by workspace:type.
    pub ptys: Mutex<HashMap<String, pty::Pty>>,
    /// The visible workspace does not acquire unread status for live updates.
    pub looking: Mutex<Option<String>>,
}

/// Finder and desktop-launcher starts inherit a minimal PATH. Adopt the user's login-shell PATH so agents and
/// Homebrew tools can be found; terminal launches retain equivalent behavior.
fn adopt_login_path() {
    let shell = platform::shell();
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
    if std::env::args().nth(1).as_deref() == Some("--prometeu-mcp") {
        if let Err(error) = embedded_mcp::stdio() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--prometeu-mcp-client") {
        match mcp_access::cli(&std::env::args().skip(2).collect::<Vec<_>>()) {
            Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    install_crypto();
    adopt_login_path();
    let root = paths::root();
    let profiles: Arc<dyn prometeu_profiles::ProfileBackend> = Arc::new(account_profiles::native());
    let telemetry = telemetry::Service::new(root.clone());
    let board_store = Arc::new(board_store::FileBoardStore::new(root));
    let mut board = board_store.load();
    board.revive();
    board.prepare_telemetry_ids();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            project_entries: Arc::new(prometeu_files::entries::NativeEntries {
                trash: Arc::new(prometeu_files::entries::SystemTrash),
            }),
            project_search: Arc::new(prometeu_files::search::NativeSearch::default()),
            worktree_cleanup: Arc::new(prometeu_git::cleanup::NativeCleanup),
            repository_git: Arc::new(prometeu_git::NativeGit),
            providers: Arc::new(agent_launch::NativeProviders {
                profiles: profiles.clone(),
                tools: Arc::new(tool_materialization::native()),
            }),
            authentication: Arc::new(account_login::NativeAuthentication { profiles }),
            tasks: Arc::new(prometeu_process::ThreadExecutor),
            command_runner: Arc::new(prometeu_process::command::UnixCommandRunner),
            query_launcher: Arc::new(prometeu_process::query::UnixQueryLauncher),
            terminal_factory: Arc::new(prometeu_process::terminal::UnixTerminalFactory),
            sessions: prometeu_core::session::host::SessionHost::default(),
            process_launcher: Arc::new(prometeu_process::UnixProcessLauncher),
            telemetry: Mutex::new(telemetry),
            board: Mutex::new(board),
            save: BoardPublisher::new(board_store),
            ptys: Mutex::new(HashMap::new()),
            looking: Mutex::new(None),
        })
        .manage(background::State::default())
        .manage(usage::Service::new())
        .manage(machine::Service::new())
        .invoke_handler(tauri::generate_handler![
            background::background_context,
            notifications::notification_permission,
            notifications::notification_show,
            notifications::notification_current,
            notifications::notification_dismiss,
            notifications::notification_open,
            notifications::notification_sound,
            island::island_enable,
            island::island_update,
            island::island_current,
            island::island_resize,
            island::island_open,
            actions::actions_save,
            actions::action_start,
            i18n::set_lang,
            agents::agents,
            agents::agent_models,
            accounts::accounts,
            accounts::account_select,
            accounts::account_remove,
            accounts::account_login,
            accounts::account_login_cancel,
            usage::usage,
            usage::usage_refresh,
            telemetry::telemetry_summary,
            telemetry::telemetry_insights,
            telemetry::telemetry_turns,
            telemetry::telemetry_events,
            telemetry::telemetry_export,
            telemetry::telemetry_clear,
            machine::machine,
            machine::set_resource_detail,
            awake::set_awake,
            session::load_board,
            session::add_project,
            session::remove_project,
            session::reorder_projects,
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
            session::set_confirm_messages,
            session::look_at,
            session::rename_workspace,
            session::remove_workspace,
            session::diff::workspace_branch,
            session::git::workspace_git_status,
            session::git::tree_git_status,
            session::git::tree_restore,
            session::git::file_base,
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
            session::files::create_path,
            session::files::rename_path,
            session::files::trash_path,
            session::files::reveal_path,
            dock::open_dock,
            dock::close_dock,
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
            feedback::feedback_image,
            feedback::feedback_send,
            file_drop::paste_files,
            session::pr_prompt,
            github::pr_open,
            github_auth::github_status,
            github_auth::github_connect,
            github_auth::github_disconnect,
            github_issues::github_issues,
            github_issues::github_claim,
            github_notifications::github_feed,
            github_notifications::github_subjects,
            github_notifications::github_detail,
            github_issues::github_issue_open,
            github_issues::github_projects,
            github_issues::github_prepare,
            github::refresh_prs,
            github::open_pr,
            session::new_tab,
            session::close_tab,
            session::focus_tab,
            session::rename_tab,
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
            linear::linear_claim,
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
            catalog::catalog_share,
            catalog::catalog_copy,
            catalog::projects::catalog_install_project,
            catalog::catalog_install_plugin,
            catalog::catalog_install_skill,
            catalog::catalog_install_organization_item,
            skills::skill_hub,
            skills::skill_save,
            skills::skill_remove,
            kickoff::plugin_skills,
            typesafe::typesafe_status,
            typesafe::typesafe_save_key,
            typesafe::typesafe_remove_key,
            typesafe::typesafe_set_enabled,
            typesafe::context_evaluate,
            review_calibration::review_calibration_status,
            review_calibration::review_calibration_set_enabled,
            review_calibration::review_calibration_append,
            review_calibration::review_calibration_clear,
            review_calibration::review_calibration_export,
        ])
        .setup(|app| {
            if let Some(window) = tauri::Manager::get_window(app, "main") {
                background::refresh_window(&window);
            }
            background::watch(app.handle().clone());
            notifications::install(app.handle());
            embedded_mcp::start(app.handle().clone())?;
            // The account connectors are part of the inherited MCP base; fetch them before the
            // first picker or spawn asks for them (ADR 0063).
            mcp::warm_connectors();
            file_drop::install(app.handle())?;
            machine::watch(app.handle().clone());
            usage::watch(app.handle().clone());
            Ok(())
        })
        .on_webview_event(file_drop::on_webview_event)
        .on_window_event(|window, _event| background::refresh_window(window))
        .build(tauri::generate_context!())
        .expect("erro ao subir o Prometeu")
        .run(|app, event| {
            // Flush deferred board writes during shutdown, when no later save can be assumed.
            if matches!(event, tauri::RunEvent::Exit) {
                awake::shutdown();
                embedded_mcp::shutdown();
                notifications::shutdown();
                accounts::shutdown();
                state::save_now(app);
            }
        });
}
