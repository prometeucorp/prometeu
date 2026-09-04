# Matriz de providers

Status: comportamento atual observado no código. “A confirmar” significa que a
feature pode existir, mas ainda não possui evidência de conformidade suficiente
para virar capacidade contratual.

## Legenda

- **Nativo:** o CLI já fala a forma consumida hoje.
- **Adaptado:** o Prometeu converte ou implementa a feature.
- **Indisponível:** a UI não oferece porque o provider não suporta o fluxo.
- **A confirmar:** falta fixture ou teste dedicado.

| Capacidade | Claude | Codex | Evidência principal |
| --- | --- | --- | --- |
| detectar instalação | adaptado | adaptado | `agents.rs` |
| catálogo vivo de modelos | nativo via control request | nativo via cache do CLI | `agents.rs` |
| iniciar sessão | nativo | adaptado para JSON-RPC | `chat.rs`, `codex.rs` |
| retomar sessão | id/transcript do Claude | thread do app-server | `session.rs`, testes Rust |
| escolher modelo | nativo por flag | adaptado no `thread/start`/`thread/resume` | `session.rs`, `codex.rs` |
| níveis de esforço | catálogo + fallback | catálogo do Codex | `agents.rs`, `launcher.ts` |
| plan mode inicial | nativo por permission mode | indisponível | `session.rs`, `launcher.ts` |
| texto em streaming | adaptado para V1 | adaptado para V1 | `conversation.test.ts`, `timeline.test.ts`, testes de `codex.rs` |
| pensamento | adaptado para V1 | adaptado para V1 | `timeline.test.ts`, testes de `codex.rs` |
| tool call e resultado | adaptado para V1 | adaptado para V1 | `conversation.test.ts`, testes de `claude.rs`/`codex.rs` |
| perguntas ao usuário | nativo | adaptado de request JSON-RPC | `chat.ts`, testes de `codex.rs` |
| pedidos de aprovação | nativo | adaptado | `chat.rs`, `codex.rs` |
| interrupção | control request | `turn/interrupt` | `codex.rs`, testes Rust |
| compactação | comando do CLI | `thread/compact/start` | testes de `codex.rs` |
| relatório de contexto | stream/transcript | sintetizado de token usage | `context.test.ts`, testes de `codex.rs` |
| seleção de MCP por workspace | config estrita do CLI | tabela e ambiente montados pelo app | `mcp.rs`, `codex.rs`; erro de preparação impede spawn |
| seleção de plugins por workspace | flags de sessão | marketplace + config isolada por workspace | `plugins.rs`, smoke do CLI, `codex.rs`, `launcher.ts`, E2E |
| hooks de plugin escolhido | ativos desde `SessionStart` | `enabled = true` + confiança limitada a `pluginId` e hash antes da thread | testes de `codex.rs`; falha impede a thread |
| anexos na fala | adaptado por caminho local | adaptado por caminho local | capability + `chat.ts`; falta teste transversal dedicado |
| evento externo desconhecido | ignorado pelo adapter | ignorado pelo adapter | `conversation.test.ts`, testes de `claude.rs`/`codex.rs` |
| compartilhamento ao vivo | V1 após normalização | V1 após normalização | `team*.test.ts`, E2E sobre mock |

## Regra para feature nova

Antes de habilitar uma feature para um provider:

1. declarar sua semântica comum no contrato;
2. adicionar ou ajustar a capacidade no descriptor;
3. capturar fixture real do CLI sem segredo ou dado pessoal;
4. provar tradução para eventos comuns;
5. executar o mesmo cenário no reducer e no mock;
6. atualizar esta matriz com o caminho da evidência.

Ausência de teste não deve virar `true` por semelhança entre providers.

## Limitações conhecidas

- origem de plugin em `.zip` local ou URL funciona no Claude e é recusada com
  erro visível no Codex; pasta local é o formato portátil;
- skills, comandos, MCP e hooks possuem adaptação portátil; `agents/*.md`
  permanece exclusivo do Claude porque não integra o manifesto Codex atual;
- plugins habilitados fora do Prometeu continuam sujeitos ao cadastro global
  de cada CLI e não fazem parte da seleção do workspace;
- anexos ainda não possuem teste transversal dedicado.

## Suíte de conformidade desejada

| Cenário | Fixture externa | Adapter | Reducer | E2E |
| --- | --- | --- | --- | --- |
| fala simples | por provider | obrigatório | obrigatório | smoke |
| streaming + mensagem final | por provider | obrigatório | obrigatório | crítico |
| ferramenta bem-sucedida | por provider | obrigatório | obrigatório | crítico |
| ferramenta com erro | por provider | obrigatório | obrigatório | crítico |
| pergunta e resposta | por provider | obrigatório | obrigatório | crítico |
| interrupção | por provider | obrigatório | obrigatório | smoke |
| resume | por provider | obrigatório | replay | crítico |
| compactação | por provider capaz | obrigatório | obrigatório | smoke |
| background task | por provider capaz | obrigatório | obrigatório | smoke |
| evento desconhecido | sintética | obrigatório | obrigatório | não necessário |

E2E não substitui contrato: ele cobre poucos caminhos caros. Fixtures e
reducers dão diagnóstico rápido para todas as combinações.
