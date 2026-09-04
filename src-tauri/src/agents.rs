//! Quais agentes esta máquina tem, e o que cada um aceita.
//!
//! São dois CLIs: o `claude` e o `codex`. O lançador só oferece o que está
//! instalado — quem tem um só não pode ver a lista do outro e escolher um modelo
//! que nunca vai rodar.
//!
//! A aba é a mesma nos dois: uma conversa desenhada pelo app, um card, uma nota
//! de atividade, um transcript. O que troca é o processo por trás dela — o
//! `claude -p` falando stream-json (`chat.rs`) ou o `codex app-server` falando
//! JSON-RPC (`codex.rs`), traduzido para as mesmas linhas. Aqui fica só o que
//! é catálogo: quem está instalado, quais modelos o Codex oferece, e o nome
//! que ele dá a cada degrau de esforço.
//!
//! A lista de modelos não está escrita aqui: cada CLI mantém o próprio
//! catálogo — o `codex` num arquivo (`models_cache.json`), o `claude` numa
//! pergunta (o control request `list_models`) — e é deles que o lançador tira o
//! que oferecer. Modelo novo aparece no dropdown sem release do Prometeu.

use crate::paths;
use crate::state::ProviderId;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Quais dos dois CLIs estão no PATH. É o `command -v` de um shell de login, e
/// não um teste de arquivo: o `claude` e o `codex` moram onde o profile da
/// pessoa disser. Um shell só para os dois — abrir um shell de login custa
/// perto de um segundo, e isto acontece com a janela subindo.
fn installed() -> (bool, bool) {
    let out = std::process::Command::new("sh")
        .args([
            "-lc",
            "command -v claude && echo TEM_CLAUDE; command -v codex && echo TEM_CODEX; true",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    (out.contains("TEM_CLAUDE"), out.contains("TEM_CODEX"))
}

/// `$CODEX_HOME`, ou o `~/.codex` de sempre. É o home do usuário de propósito:
/// conta, skills, memórias e config do Codex continuam valendo dentro do app.
fn home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths::home().join(".codex"))
}

/// Um modelo como o lançador o mostra.
#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub label: String,
    /// Os níveis de esforço que este modelo aceita — o Sol vai até `ultra`, o
    /// 5.4 para no `xhigh`. O lançador não deixa escolher o que o CLI recusaria.
    pub efforts: Vec<String>,
}

/// Features que o restante do app pode oferecer sem conhecer o provider. O
/// nome é o do contrato TypeScript; serde faz a travessia em camelCase.
#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub initial_plan_mode: bool,
    pub workspace_mcp_selection: bool,
    pub workspace_plugin_selection: bool,
    pub resume: bool,
    pub compact: bool,
    pub context_report: bool,
    pub approvals: bool,
    pub user_questions: bool,
    pub attachments: bool,
}

/// Um runtime descoberto, seu catálogo e o que esta versão consegue fazer com
/// ele. A UI recebe os dois providers inclusive quando não estão instalados;
/// assim ausência e catálogo momentaneamente vazio continuam coisas distintas.
#[derive(serde::Serialize)]
pub struct AgentDescriptor {
    pub id: ProviderId,
    pub label: String,
    pub installed: bool,
    pub models: Vec<Model>,
    pub capabilities: AgentCapabilities,
}

#[derive(serde::Serialize)]
pub struct Agents {
    pub providers: Vec<AgentDescriptor>,
}

fn capabilities(id: ProviderId) -> AgentCapabilities {
    let common = AgentCapabilities {
        initial_plan_mode: false,
        workspace_mcp_selection: true,
        workspace_plugin_selection: true,
        resume: true,
        compact: true,
        context_report: true,
        approvals: true,
        user_questions: true,
        // O app injeta caminhos locais na fala; ambos os runtimes podem lê-los
        // no mesmo worktree. Não é upload nem payload binário do provider.
        attachments: true,
    };
    match id {
        ProviderId::Claude => AgentCapabilities {
            initial_plan_mode: true,
            ..common
        },
        ProviderId::Codex => common,
    }
}

fn descriptor(id: ProviderId, installed: bool, models: Vec<Model>) -> AgentDescriptor {
    AgentDescriptor {
        id,
        label: match id {
            ProviderId::Claude => "Claude".into(),
            ProviderId::Codex => "Codex".into(),
        },
        installed,
        models,
        capabilities: capabilities(id),
    }
}

/// Roda uma vez por sessão do app: nem CLI se instala, nem catálogo muda com a
/// janela aberta.
#[tauri::command]
pub fn agents() -> Agents {
    let (claude, codex) = installed();
    Agents {
        providers: vec![
            descriptor(ProviderId::Claude, claude, vec![]),
            descriptor(
                ProviderId::Codex,
                codex,
                if codex { codex_models() } else { vec![] },
            ),
        ],
    }
}

/// O catálogo do Claude Code, perguntado a ele mesmo: o `claude -p` responde
/// ao control request `list_models` com a mesma lista do seletor `/model` —
/// modelo novo da Anthropic entra no dropdown sem release do Prometeu, e
/// modelo que a conta não tem nem aparece. É comando à parte do `agents` de
/// propósito: isto sobe um processo e leva segundos, e a faixa de baixo não
/// pode esperar por ele para dizer quais agentes existem.
///
/// Vazio é "não deu" — CLI antigo que não conhece o request, ou resposta que
/// não veio — e aí o lançador fica com a lista fixa que sempre teve.
#[tauri::command]
pub async fn claude_models() -> Vec<Model> {
    tauri::async_runtime::spawn_blocking(ask_claude_models)
        .await
        .unwrap_or_default()
}

fn ask_claude_models() -> Vec<Model> {
    let mut cmd = Command::new("claude");
    // Sem sessão gravada e sem hooks: isto é uma pergunta de catálogo, não uma
    // conversa — não pode deixar transcript nem acordar hook de gente a cada
    // janela que abre.
    cmd.args([
        "-p",
        "--verbose",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--no-session-persistence",
        "--settings",
        r#"{"hooks":{}}"#,
    ])
    .current_dir(paths::home())
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    // Como em `chat.rs`: um `claude` dentro de outro herda CLAUDE_* e muda de
    // comportamento. O resto do ambiente vai inteiro — é dele que sai o PATH.
    cmd.env_clear();
    for (k, v) in std::env::vars() {
        if !k.starts_with("CLAUDE") {
            cmd.env(k, v);
        }
    }
    let Ok(mut child) = cmd.spawn() else {
        return vec![];
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return vec![];
    };
    let asked = stdin
        .write_all(
            b"{\"type\":\"control_request\",\"request_id\":\"models\",\"request\":{\"subtype\":\"list_models\"}}\n",
        )
        .is_ok();
    // O stdin fica aberto até a resposta: fechar é encerrar a sessão antes dela.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains("\"control_response\"") {
                let _ = tx.send(parse_claude_models(&line));
                return;
            }
        }
        let _ = tx.send(vec![]);
    });
    let models = if asked {
        rx.recv_timeout(Duration::from_secs(20)).unwrap_or_default()
    } else {
        vec![]
    };
    let _ = child.kill();
    let _ = child.wait();
    models
}

/// Lê a resposta do `list_models`. Fora ficam o "Default (recommended)" — no
/// lançador escolher é sempre escolher um nome — e o que o CLI marca como
/// `disabled`, que é anúncio de modelo pedindo CLI mais novo, não escolha.
fn parse_claude_models(line: &str) -> Vec<Model> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    v["response"]["response"]["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter(|m| m["value"].as_str() != Some("default"))
                .filter(|m| m["disabled"].as_bool() != Some(true))
                .filter_map(|m| {
                    let id = m["value"].as_str()?.to_string();
                    let label = m["displayName"].as_str().unwrap_or(&id).to_string();
                    let efforts = m["supportedEffortLevels"]
                        .as_array()
                        .map(|ls| {
                            ls.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(Model { id, label, efforts })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// O catálogo do Codex, filtrado pelo que ele mesmo marca como visível. Lista
/// vazia é "não há Codex nesta máquina" — o `codex` fora do PATH, ou instalado e
/// nunca aberto (o catálogo só existe depois do primeiro login).
fn codex_models() -> Vec<Model> {
    let Ok(raw) = std::fs::read_to_string(home().join("models_cache.json")) else {
        return vec![];
    };
    let Ok(cache) = serde_json::from_str::<Value>(&raw) else {
        return vec![];
    };
    cache["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter(|m| m["visibility"].as_str() == Some("list"))
                .filter_map(|m| {
                    let id = m["slug"].as_str()?.to_string();
                    let label = m["display_name"].as_str().unwrap_or(&id).to_string();
                    let efforts = m["supported_reasoning_levels"]
                        .as_array()
                        .map(|ls| {
                            ls.iter()
                                .filter_map(|l| l["effort"].as_str())
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(Model { id, label, efforts })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// O modelo com que o Codex nomeia um workspace: o mais barato que o catálogo
/// oferece, que é o de maior `priority` — o catálogo ordena do carro-chefe (1)
/// para o mini (23). São cinco palavras a partir de um parágrafo, e gastar o
/// modelo do trabalho nisso é caro e mais lento. Vazio é catálogo ausente: aí o
/// nomeador cai no modelo do próprio workspace.
pub fn codex_namer_model() -> String {
    let Ok(raw) = std::fs::read_to_string(home().join("models_cache.json")) else {
        return String::new();
    };
    let Ok(cache) = serde_json::from_str::<Value>(&raw) else {
        return String::new();
    };
    cache["models"]
        .as_array()
        .and_then(|models| {
            models
                .iter()
                .filter(|m| m["visibility"].as_str() == Some("list"))
                .max_by_key(|m| m["priority"].as_u64().unwrap_or(0))
                .and_then(|m| m["slug"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// O nível de esforço como o Codex o chama. A escada da tela é a do Claude Code,
/// e o último degrau tem nome diferente aqui: `ultracode` é a orquestração de
/// subagentes de lá, `ultra` é a de cá.
pub fn effort(level: &str) -> &str {
    match level.trim() {
        "ultracode" => "ultra",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ultracode_vira_ultra() {
        assert_eq!(effort("ultracode"), "ultra");
        assert_eq!(effort("max"), "max");
    }

    // A resposta como o `claude` 2.1.251 a escreve, encurtada: o "Default" e o
    // anúncio de modelo desabilitado ficam de fora, o resto vira catálogo.
    #[test]
    fn le_o_catalogo_do_claude() {
        let line = r#"{"type":"control_response","response":{"subtype":"success","request_id":"models","response":{"models":[
            {"value":"default","resolvedModel":"claude-opus-5[1m]","displayName":"Default (recommended)","supportsEffort":true,"supportedEffortLevels":["low","medium","high","xhigh","max"]},
            {"value":"opus[1m]","resolvedModel":"claude-opus-5[1m]","displayName":"Opus (1M context)","supportsEffort":true,"supportedEffortLevels":["low","medium","high","xhigh","max"]},
            {"value":"haiku","resolvedModel":"claude-haiku-4-5","displayName":"Haiku"},
            {"value":"cc-update-required-1","resolvedModel":"cc-update-required-1","displayName":"Fable 5.1 (disabled)","disabled":true}
        ]}}}"#;
        let models = parse_claude_models(line);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "opus[1m]");
        assert_eq!(models[0].label, "Opus (1M context)");
        assert_eq!(models[0].efforts, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(models[1].id, "haiku");
        assert!(models[1].efforts.is_empty());
    }

    /// Sobe o `claude` de verdade e pergunta o catálogo. Fora do `cargo test`
    /// de sempre porque precisa do CLI instalado e leva segundos:
    /// `cargo test -- --ignored pergunta`.
    #[test]
    #[ignore]
    fn pergunta_o_catalogo_de_verdade() {
        let models = ask_claude_models();
        for m in &models {
            println!("{} = {} [{}]", m.id, m.label, m.efforts.join(","));
        }
        assert!(!models.is_empty());
    }

    #[test]
    fn resposta_estranha_e_catalogo_vazio() {
        assert!(parse_claude_models("nem json").is_empty());
        assert!(parse_claude_models(
            r#"{"type":"control_response","response":{"subtype":"error"}}"#
        )
        .is_empty());
    }

    #[test]
    fn capacidades_sao_do_descriptor_e_nao_da_tela() {
        let claude = descriptor(ProviderId::Claude, true, vec![]);
        let codex = descriptor(ProviderId::Codex, true, vec![]);

        assert!(claude.capabilities.initial_plan_mode);
        assert!(claude.capabilities.workspace_plugin_selection);
        assert!(!codex.capabilities.initial_plan_mode);
        assert!(codex.capabilities.workspace_plugin_selection);
        assert!(codex.capabilities.workspace_mcp_selection);
        assert!(codex.capabilities.resume);

        let json = serde_json::to_value(codex).unwrap();
        assert_eq!(json["id"], "codex");
        assert_eq!(json["capabilities"]["initialPlanMode"], false);
        assert_eq!(json["capabilities"]["workspaceMcpSelection"], true);
    }
}
