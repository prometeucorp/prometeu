# Contrato do runtime de agente

Status: contrato vigente; identidade/capabilities implementados pelo ADR 0003
e eventos canônicos implementados pelo ADR 0002.

Este contrato define a fronteira entre o Prometeu e um CLI de agente. Ele não
é uma API para modelos de linguagem: descreve processos locais que possuem
catálogo, sessão, protocolo e capacidades próprias.

## Identidade

Provider e modelo são conceitos distintos. Um modelo pertence a um provider;
o nome do modelo não deve ser usado para redescobrir seu provider depois que o
catálogo foi carregado.

```ts
type ProviderId = "claude" | "codex";

type AgentModel = {
  id: string;
  label: string;
  efforts: string[];
};

type AgentDescriptor = {
  id: ProviderId;
  label: string;
  installed: boolean;
  models: AgentModel[];
  capabilities: AgentCapabilities;
};
```

Ao adicionar um provider, `ProviderId` cresce explicitamente. Um valor antigo
ou desconhecido vindo do disco cai no provider padrão somente durante a
migração de persistência; código novo usa matching exaustivo. O campo no JSON
continua chamado `agent` por compatibilidade, mas seu valor normalizado é
`"claude" | "codex"`.

## Capacidades

```ts
type AgentCapabilities = {
  initialPlanMode: boolean;
  workspaceMcpSelection: boolean;
  workspacePluginSelection: boolean;
  resume: boolean;
  compact: boolean;
  contextReport: boolean;
  approvals: boolean;
  userQuestions: boolean;
  attachments: boolean;
};
```

Este é o conjunto mínimo observado pela interface atual. Uma capacidade só
entra aqui quando altera comportamento oferecido pela aplicação. Detalhes de
protocolo, como o nome de um método JSON-RPC, não são capacidades.

Algumas capacidades podem variar por versão do CLI ou modelo. Nesse caso, o
descriptor retornado em runtime é a fonte de verdade; o frontend não mantém uma
tabela paralela. Antes da descoberta, ou se o IPC falhar, o bootstrap do
frontend mantém apenas Claude instalado e não anuncia capacidade opcional.

## Configuração de sessão

```ts
type SessionLaunch = {
  provider: ProviderId;
  model: string | null;
  effort: string | null;
  initialPlanMode: boolean;
  mcp: string[] | null;
  plugins: string[] | null;
  cwd: string;
  resume: string | null;
};
```

Semântica dos valores opcionais:

- `null` em modelo ou esforço deixa o provider escolher seu padrão;
- `null` em MCP/plugins significa não impor seleção e preservar a configuração
  do CLI;
- lista vazia significa não injetar nenhum item do hub do Prometeu; cadastro
  global que o próprio CLI carrega permanece sob controle dele;
- `resume` é uma identidade opaca aceita pelo provider. Pode ter sido escolhida
  pelo Prometeu, como no Claude, ou devolvida pelo provider, como no Codex.

O core valida `SessionLaunch` contra as capacidades antes de iniciar o adapter.
O adapter não deve corrigir silenciosamente uma combinação inválida.
Configuração MCP ou de plugin escolhida que não possa ser materializada falha
antes do spawn; hook declarado que não possa ser ativado falha antes da abertura
da thread. Iniciar sem o comportamento solicitado não é fallback válido. O
contrato detalhado de plugins está em
[`plugin-marketplace.md`](plugin-marketplace.md).

## Port conceitual

O desenho pode ser implementado com trait, enum dispatch ou funções agrupadas.
A semântica é mais importante que a forma sintática:

```text
AgentCatalog
  discover() -> AgentDescriptor[]

AgentRuntime
  start(SessionLaunch) -> SessionHandle
  send(SessionHandle, ConversationCommand)
  stop(SessionHandle)
  output(SessionHandle, RawProviderEvent) -> ConversationEvent[]
```

Responsabilidades do adapter:

- iniciar o executável e configurar seu ambiente;
- converter `SessionLaunch` para argumentos ou requests do provider;
- materializar MCP e plugins na forma exigida pelo provider sem expor essa
  forma ao domínio;
- correlacionar requests e respostas próprias do protocolo externo;
- transformar saída externa em `ConversationEventV1`;
- transformar `ConversationCommandV1` em entrada externa;
- expor falhas com código estável e detalhe diagnóstico;
- encerrar o processo e os descendentes conforme a política do app.

Responsabilidades que ficam fora do adapter:

- escolher o que a UI mostra;
- persistir estado do quadro;
- aplicar audiência do time;
- renderizar ferramentas ou markdown;
- decidir políticas globais de retomada e fila de fala.

## Compatibilidade

- Mudança apenas no protocolo externo deve alterar um adapter e suas fixtures.
- Mudança no comportamento comum altera este contrato e a suíte de
  conformidade.
- Capacidade nova começa como `false` nos providers existentes até haver
  evidência e teste.
- Provider indisponível não impede o catálogo dos outros de carregar.
- Falha de descoberta não deve inventar suporte; o fallback precisa ser
  explícito e observável.

## Conformidade mínima

Cada provider precisa demonstrar, quando a capacidade existir:

1. início de sessão nova;
2. retomada sem duplicar mensagens;
3. fala e resposta final;
4. streaming seguido do evento autoritativo;
5. tool call e resultado;
6. pergunta ou aprovação e resposta correlacionada;
7. interrupção;
8. compactação e relatório de contexto;
9. encerramento do processo e descendentes;
10. tolerância a evento externo desconhecido.

A matriz viva dessas evidências está em
[`../quality/provider-matrix.md`](../quality/provider-matrix.md).
