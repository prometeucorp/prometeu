# Contrato IPC TypeScript ↔ Rust

Status: contrato atual documentado; tipagem completa é trabalho futuro.

## Fontes atuais

- `src/ipc.ts` mantém a lista de nomes aceitos pelo frontend.
- `src-tauri/src/main.rs` registra os handlers Rust em `generate_handler!`.
- `src/mock.ts` implementa respostas para desenvolvimento no navegador.
- testes verificam paridade entre usos do frontend, handlers e mock.

Essa proteção detecta comando ausente ou escrito errado. Argumentos, retornos e
erros ainda não têm uma fonte tipada compartilhada: `invoke<T>` aceita o tipo
escolhido pelo chamador e os argumentos são `InvokeArgs`.

## Regras de comando

- nomes usam `snake_case` e precisam ser únicos;
- argumentos TypeScript usam as chaves esperadas pela desserialização Tauri;
- retorno Rust deve implementar `Serialize`;
- argumento Rust deve implementar `Deserialize`;
- operação falível retorna `Result<Resposta, Erro>`;
- erro de domínio usa código estável traduzido pelo frontend;
- detalhe externo ou de I/O pode acompanhar o código sem virar texto fixo da UI;
- operação pesada não bloqueia a thread principal.

## Eventos emitidos atualmente

| Evento | Emissor | Payload emitido |
| --- | --- | --- |
| `board` | `state.rs` | `Board` completo |
| `chat` | `chat.rs` | `[session, conversationEventV1JsonLine, seq]` |
| `chat-closed` | `chat.rs` | id da sessão |
| `pty` | `pty.rs` | `[session, bytes, seq]` |
| `pty-closed` | `pty.rs` | `[session, exitCode]` |
| `usage` | `usage.rs` | snapshot de uso por agente |
| `machine` | `machine.rs` | estado da máquina |
| `linear` | `linear.rs` | `LinearStatus` |
| `plugin-make` | `plugins.rs` | `[run, step]` |
| `plugin-made` | `plugins.rs` | `[run, error]` |
| `browser:url` | `browser.rs` | `[workspace, url]` |

O snapshot do evento e comando `usage` é um mapa por provider. Cada entrada
tem `{ windows, at }`; cada janela tem `{ kind, pct, resets, scope?, label? }`.
`scope` identifica cotas independentes para que atualizações esparsas de um
modelo não apaguem as demais, e `label` é texto externo opcional para exibição.
Consumidores devem aceitar os dois campos ausentes por compatibilidade com o
cache anterior.

Eventos Tauri são dinâmicos; o generic passado a `listen<T>` não valida o
payload Rust em build time. Um evento novo precisa de teste do emissor e do
consumidor.

`chat_snapshot.text` pode misturar linhas V1 e legado depois de uma importação.
`Timeline` valida V1 e envia o restante ao leitor legado; projeções históricas
`prometheusV1Mirror` são ignoradas pelo leitor atual. O Prometeu não produz
essas projeções em logs novos.

## Importação legada

`legacy_import_plan` não altera estado. Ele devolve a origem, uma das situações
`ready | missing | imported | targetNotEmpty | invalid`, as contagens da prévia
e, quando aplicável, erro, instante e caminho do backup.

`legacy_import_run` não recebe caminhos da apresentação: origem e destino são
resolvidos pelo backend. Ele repete todas as validações, recusa o Prometheus
aberto e um destino ocupado, executa a importação e devolve o mesmo DTO no
estado `imported`. A mudança do board continua sendo publicada pelo evento
`board`.

## Checklist de mudança

Ao criar ou mudar comando:

1. alterar a função Rust e seu tipo de erro;
2. registrar o handler em `main.rs`;
3. atualizar o nome em `src/ipc.ts`;
4. implementar ou recusar conscientemente no `src/mock.ts`;
5. atualizar todos os consumidores TypeScript;
6. adicionar teste da forma dos argumentos e retorno;
7. documentar compatibilidade quando houver estado persistido envolvido.

Ao criar ou mudar evento:

1. definir payload e ownership;
2. testar serialização no emissor;
3. validar o payload no consumidor quando vier de fronteira não confiável;
4. garantir que listeners sejam desmontados junto do lifecycle da tela;
5. atualizar a tabela acima.

## Direção de evolução

Primeiro passo sem dependência nova: substituir `invoke<T>` por um mapa de
comandos no TypeScript:

```ts
type Commands = {
  load_board: { args: undefined; result: Board };
  chat_send: { args: { session: string; text: string }; result: void };
};
```

O passo seguinte pode gerar bindings a partir dos DTOs Rust. A ferramenta deve
ser escolhida em ADR depois de uma prova pequena com:

- enums com `serde(rename_all)`;
- `Option` e campos default;
- erros serializados;
- eventos, além de commands;
- integração com Tauri 2 e a versão de Rust usada no projeto.

Gerar tipos de structs internas inteiras não é o objetivo. Apenas DTOs da
fronteira devem aparecer no binding.
