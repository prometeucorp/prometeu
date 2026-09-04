# ADR 0005 — Um marketplace portátil para Claude e Codex

Data: 2026-09-04
Status: Aceito

## Contexto

O Prometeu já possuía um hub de plugins e seleção por workspace, mas o
adapter entregava a seleção apenas ao Claude Code por `--plugin-dir` ou
`--plugin-url`. A capability ficava desabilitada para o Codex.

O Codex não possui flag equivalente para uma pasta arbitrária. Seu runtime
descobre plugins por marketplace, instala uma versão no cache global e guarda
o estado de ativação no `config.toml`. Cadastrar esse marketplace diretamente
no worktree contaminaria o repositório; habilitar a instalação globalmente
faria uma escolha de workspace escapar para todas as outras sessões.

Além disso, hooks instalados no Codex não são confiáveis apenas por estarem no
cache. A confiança é ligada ao hash atual e precisa preservar a fronteira entre
o pacote escolhido e hooks globais ou de outro projeto.

## Opções consideradas

1. Manter plugins exclusivos do Claude e esconder o seletor no Codex.
2. Criar dois hubs e pedir que a pessoa cadastre versões distintas do mesmo
   plugin.
3. Gravar `.agents/plugins/marketplace.json` e configuração Codex em cada
   worktree.
4. Manter um hub comum e materializar um `CODEX_HOME` de configuração derivado
   por workspace, fora do repositório.

## Decisão

Adotar a opção 4. O hub e `Workspace.plugins` continuam independentes de
provider. `plugins.rs` passa a conter dois adapters:

- Claude recebe a origem por flags de sessão;
- Codex recebe um marketplace local gerado sob a raiz privada do Prometeu,
  uma cópia com versão derivada do conteúdo e um home estável por workspace.

O lifecycle de instalação usa `codex plugin list/add/remove`, em vez dos
métodos experimentais de CRUD de plugin do app-server. Todos esses comandos e
o app-server recebem o home derivado. Seu `config.toml` parte da configuração
real, mas marketplace, ativação e confiança dos hooks do Prometeu são
gravados somente nessa cópia. As demais entradas — autenticação, sessões,
skills, memória e cache — apontam para o home real do Codex.

Não usar override `-c` para a ativação. A versão estudada do CLI aceita
`plugins."id@marketplace".enabled=true` na linha de comando, mas o loader não
aplica essa camada; o comportamento também está registrado no issue oficial
[openai/codex#35289](https://github.com/openai/codex/issues/35289). Uma camada
persistida e isolada produz o comportamento verificável sem alterar o
`config.toml` global nem contaminar o worktree.

Selecionar o pacote também ativa e autoriza, por hash, somente os hooks que o
Codex atribui àquele `pluginId`. Antes de abrir a thread, o adapter exige que
todo pacote que declarou hooks tenha sido descoberto e grava `enabled = true`
junto do hash. Falha nessa etapa encerra a abertura em vez de degradar o pacote
silenciosamente para skills. A escrita pelo app-server acontece no home
derivado, que pode ser reconstruído caso uma versão do protocolo altere ou
perca campos de configuração.

O formato portátil tem `.claude-plugin/plugin.json` como base e aceita
`.codex-plugin/plugin.json` como overlay. O criador gera ambos; a adaptação de
pacotes antigos acontece apenas na cópia derivada. Nessa adaptação, objetos de
hooks inline do Claude ganham o envelope `hooks` exigido pelo manifesto Codex;
a revisão do formato participa do cachebuster para migrar cópias já existentes.

## Consequências

Positivas:

- marketplace, instalação e seleção continuam sendo uma experiência única;
- trocar Claude por Codex no workspace não perde a seleção;
- a adaptação não escreve no worktree nem modifica o pacote de origem;
- um update sem bump de versão ainda invalida o cache pelo hash;
- plugins do Prometeu não passam a valer fora dele;
- o `config.toml` real nunca entra no ciclo de escrita do adapter;
- confiança de hook fica limitada ao pacote escolhido e à revisão atual;
- um modo baseado em `SessionStart` já vale na primeira resposta.

Negativas:

- a primeira sessão Codex com um plugin precisa copiar e instalar o pacote;
- o cache do Codex continua global, embora ativação e marketplace sejam por
  workspace;
- cada workspace mantém uma config e uma cópia de marketplace reconstruíveis;
- armazenamento de login explicitamente configurado como `keyring` continua
  seguindo a identidade independente de cada `CODEX_HOME`;
- o adapter depende do formato e dos comandos de plugin do Codex;
- uma política gerenciada que impeça ativar um hook selecionado também impede
  abrir a conversa;
- `.zip` local e URL continuam sendo origens exclusivas do Claude até o Codex
  oferecer uma instalação segura equivalente;
- diferenças reais entre eventos ou componentes dos dois CLIs ainda podem
  exigir conteúdo condicional dentro do pacote.

## Evidência

- `plugins.rs` testa descoberta dos dois formatos de marketplace e geração do
  pacote/marketplace Codex a partir de um plugin compatível;
- um teste opt-in instala uma skill e um hook `SessionStart` temporários com o
  CLI real, executa o handshake do app-server, prova que ambos chegaram ao
  runtime derivado e compara o `config.toml` global antes e depois;
- `codex.rs` testa que só hooks do `pluginId` escolhido recebem ativação e o
  hash de confiança antes da abertura da thread, e que descoberta ou gravação
  incompleta impede a thread;
- `agents.rs` e o E2E do lançador demonstram a capability nos dois providers;
- [`plugin-marketplace.md`](../contracts/plugin-marketplace.md) registra o
  contrato operacional;
- a documentação oficial aceita marketplaces `.agents` e `.claude-plugin` e
  pacotes compatíveis: [OpenAI — Plugin management](https://learn.chatgpt.com/pt-BR/docs/enterprise/plugin-management);
- a estrutura nativa e os hooks seguem
  [OpenAI — Build plugins](https://developers.openai.com/codex/plugins/build)
  e [OpenAI — Hooks](https://developers.openai.com/codex/hooks);
- o comportamento independente de autenticação por `CODEX_HOME` é deliberado
  no [openai/codex#15410](https://github.com/openai/codex/issues/15410); o modo
  padrão em arquivo é compartilhado por link no adapter;
- manter a escrita de confiança no home derivado também contém o risco de
  perda de chaves relatado em
  [openai/codex#42116](https://github.com/openai/codex/issues/42116).
