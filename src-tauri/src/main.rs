#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agents;
mod awake;
mod browser;
mod chat;
mod claude;
mod codex;
mod conversation;
mod dock;
mod domain;
mod github;
mod i18n;
mod linear;
mod lock;
mod machine;
mod mcp;
mod mcp_auth;
mod migration;
mod naming;
mod oauth;
mod paths;
mod plugins;
mod pty;
mod scripts;
mod session;
mod state;
mod team;
mod transcript;
mod usage;

use state::Board;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub struct AppState {
    pub board: Mutex<Board>,
    /// Para onde o quadro vai quando muda: uma thread só, que junta as
    /// gravações. Ver `state::spawn_saver`.
    pub save: state::Saver,
    /// Toda conversa de todo workspace continua rodando com o quadro na
    /// frente. Chave é o id da sessão, que é o id da aba.
    pub chats: Mutex<HashMap<String, chat::Chat>>,
    /// Os terminais do dock — setup, run, shells —, por `<workspace>:<tipo>`.
    pub ptys: Mutex<HashMap<String, pty::Pty>>,
    /// Qual workspace está na tela. O que acontece nele não vira novidade —
    /// você está vendo acontecer.
    pub looking: Mutex<Option<String>>,
    /// Sessões cujo Claude Code já avisou que está de pé — só nessas a primeira
    /// fala pode ir. Importa quando a fala espera o `setup` acabar: o fim dele
    /// não pode escrever num processo que ainda está subindo.
    pub ready: Mutex<HashSet<String>>,
}

/// O app aberto pelo Finder nasce com o PATH mínimo do launchd —
/// `/usr/bin:/bin:/usr/sbin:/sbin`, sem o `claude` que mora em `~/.local/bin`
/// e sem nada do Homebrew. Pergunta ao shell de login qual é o PATH de verdade
/// e adota: toda sessão nasce herdando o ambiente deste processo.
///
/// Rodando do terminal o PATH já está certo e isto só confirma o que veio.
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
    // O profile do usuário é código arbitrário: se ele travar, o app não trava com ele.
    if let Ok(Some(path)) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
        if !path.is_empty() {
            std::env::set_var("PATH", path);
        }
    }
}

/// Quem fala HTTPS aqui — o OAuth do Linear, o teste de um servidor de MCP, o
/// updater — usa `reqwest` com `rustls-no-provider`, e essa combinação exige
/// que o provedor de criptografia seja instalado antes do primeiro cliente. O
/// plugin do updater instala um, mas só quando vai checar atualização: quem
/// falasse HTTPS antes disso entrava em pânico dentro da thread do reqwest.
/// Instalar aqui torna a ordem irrelevante.
///
/// Erro é "já havia um instalado", e nesse caso não há nada a fazer nem a
/// dizer: o que se queria era que existisse um.
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
            i18n::set_lang,
            agents::agents,
            agents::claude_models,
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
            session::set_tab_choice,
            session::set_unread,
            session::set_shared,
            session::look_at,
            session::rename_workspace,
            session::remove_workspace,
            session::diff::workspace_diff,
            session::diff::workspace_branch,
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
            browser::open_external,
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
            chat::chat_buffer,
            chat::chat_snapshot,
            mcp::mcp_hub,
            mcp::mcp_save,
            mcp::mcp_remove,
            mcp::mcp_found,
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
            migration::legacy_import_plan,
            migration::legacy_import_run,
            team::team_config,
            team::team_config_set,
        ])
        .setup(|app| {
            machine::watch(app.handle().clone());
            usage::watch(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("erro ao subir o Prometeu")
        .run(|app, event| {
            // A gravação do quadro é adiada para não pesar no caminho quente.
            // Sair é o único momento em que não existe "daqui a pouco".
            if matches!(event, tauri::RunEvent::Exit) {
                state::save_now(app);
            }
        });
}
