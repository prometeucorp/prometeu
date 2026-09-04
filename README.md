<p align="center">
  <img src="docs/icon.png" alt="" width="128" height="128">
</p>

<h1 align="center">Prometeu</h1>

Sessões de agente — Claude Code ou Codex — organizadas por workspace, cada uma
no seu worktree.

Junta o que é bom no Conductor (worktree isolado por sessão, script de setup por
repo) com o que é bom no Vibe Island (você fica sabendo na hora que o agente
precisa de você), numa lista lateral que mantém cada trabalho à mão.

Sessão é one-off: nasce, faz, morre. Sem passar artefato de uma sessão para outra.

## Documentação do projeto

- [`ARCHITECTURE.md`](ARCHITECTURE.md) é o mapa das responsabilidades e fluxos.
- [`docs/README.md`](docs/README.md) indexa contratos, decisões e operação.
- [`AGENTS.md`](AGENTS.md) é a entrada curta para agentes que trabalham no repo.

Este README descreve o produto e o comportamento visível. Contratos técnicos e
decisões arquiteturais têm fonte de verdade em `docs/`.

## Como funciona

Três camadas. Só a de cima é escrita com carinho.

```
tela (TS)      lista, abas, a conversa desenhada            <- seu
   ^  ConversationEventV1             v ConversationCommandV1
back (Rust)    normaliza, guarda, numera e repassa           <- cola
                                              v protocolo externo
processo       `claude -p` (stream-json) ou `codex app-server` (JSON-RPC)
```

O app **não reimplementa o agente**: roda o CLI de verdade, sem terminal, e
cada coisa que acontece — o texto que ele escreve, a ferramenta que chama, o
resultado dela, a permissão que pede — vira uma linha V1 do Prometeu. O back
(`src-tauri/src/chat.rs`) normaliza, guarda, numera e repassa; a tela as reduz
a uma linha do tempo (`src/timeline.ts`, um reducer puro) e desenha
(`src/chat.ts`): markdown, cards de ferramenta com o diff colorido, pensamento
dobrado. O que a TUI faria com escape codes, aqui é um reducer em cima de JSON.
Skills, MCP, `/compact`, `/context` continuam sendo do CLI — o que muda é só
quem desenha.

As fronteiras maiores ficam em módulos próprios: apresentação da conversa em
`src/chat-presentation.ts`, índice de mudanças em `src/workspace-changes.ts`,
transporte e controle remoto do time em `src/team-transport.ts` e
`src/team-control.ts`; no back, Git/diff e leitura de arquivos ficam em
`src-tauri/src/session/diff.rs` e `src-tauri/src/session/files.rs`. O contrato
da conversa e a compatibilidade com transcripts antigos ficam em
`src/conversation.ts`, `src/conversation-legacy.ts` e
`src-tauri/src/conversation.rs`.

Escolher um modelo GPT no lançador troca o processo por trás da aba pelo
`codex app-server`, e a tela não fica sabendo: `src-tauri/src/codex.rs` traduz
cada notificação dele (`item/started`, `item/agentMessage/delta`,
`turn/completed`…) para eventos canônicos, e cada comando da tela para a
chamada dele (`turn/start`, `turn/interrupt`). O que o Codex faz
diferente — o id de thread que ele escolhe, a conversa que o app grava porque
o rollout dele tem outra forma, os comandos de barra que são do app — está
explicado no cabeçalho desse arquivo. O catálogo de modelos sai do
`models_cache.json` do próprio `codex` (`src-tauri/src/agents.rs`).

### O ida-e-volta

1. Uma fala é um comando `message.send`, traduzido na borda para o processo. Ele fica de
   pé entre um turno e outro — a sessão não é o processo, é o transcript no
   disco, e a próxima fala numa aba desligada o sobe de novo com `--resume`.
2. O que ele escreve no stdout vai para a tela e para o buffer da aba, cada
   linha com um número.
3. A lista lê o estado da mesma linha: ferramenta rodando é **rodando**,
   pedido de permissão é **quer você**, `result` é **pronta**, fim do processo
   é **desligada**. Não há hook nem socket: o stream já conta tudo.

### Sempre solto

Toda sessão nasce com `--dangerously-skip-permissions`, sem chavinha. Nada
para para pedir, que é o que permite acompanhar várias sessões: agente que
trava a cada `Write` não trabalha enquanto você olha outra coisa. O worktree
separa as mudanças do Git e reduz acidentes no clone, mas não é sandbox: o
processo continua com o acesso do seu usuário ao Mac. Rodar **sem** worktree
ganha o aviso adicional em laranja porque o agente mexe direto no clone em que
você trabalha.

Mesmo solto, `AskUserQuestion` e `ExitPlanMode` continuam chegando — pelo
`--permission-prompt-tool stdio`, como `control_request` — e viram cards na
conversa. É deles que sai o **quer você** da lista.

### Modelo, esforço e plan mode

O rodapé do lançador é o do Conductor: modelo, esforço, **Plan** e o clipe de
anexar (ou soltar arquivo em cima da folha). Modelo e esforço viram `--model`
e `--effort` e ficam no workspace — ⌘T e retomar nascem com os mesmos.
"Modelo padrão" é não passar a flag. Esforço é uma escada de clique, Baixo a
Máximo e depois **Ultracode** (`--effort ultracode`: `xhigh` mais a
orquestração de workflows, para conta que a tem), e dá a volta.

Nada ali é `<select>`: o popup nativo do WKWebView não abre nesta janela (o
clique chega no elemento, o menu não vem), então modelo e projeto abrem o menu
do próprio app, o mesmo do botão direito no workspace da barra lateral.

**Plan** liga o plan mode na primeira conversa, e aqui tem uma sutileza
levantada na marra (Claude Code 2.1.240): `--permission-mode plan` junto de
`--dangerously-skip-permissions` nasce em bypass, e o plano nunca acontece. O
que funciona é `--permission-mode plan --allow-dangerously-skip-permissions`:
a sessão nasce em plan, o plano chega como card, e o "sim" manda antes um
`set_permission_mode` para bypass — senão a primeira ferramenta do plano já
pergunta de novo. No Codex o botão some: o app-server não expõe plan mode.

### MCP e plugins por workspace

MCP e plugins são escolhidos uma vez no workspace e acompanham a sessão tanto
no Claude Code quanto no Codex. O catálogo é o mesmo: o Claude recebe os itens
por flags e configuração estrita; para o Codex, o Prometeu cria um
marketplace local privado e um `CODEX_HOME` de configuração isolado para o
workspace. Conta, sessões, skills e cache continuam sendo os do Codex da pessoa,
mas a ativação dos plugins do Prometeu fica somente naquele workspace; o
`config.toml` global não é reescrito.

Uma pasta de plugin é o formato portátil. Ela pode trazer os manifests
`.claude-plugin/plugin.json` e `.codex-plugin/plugin.json`; plugins antigos só
com o primeiro são adaptados numa cópia, sem alterar a origem. Hooks do Codex
só recebem confiança quando pertencem ao plugin escolhido e seu hash atual é
conhecido; a seleção também os liga desde o `SessionStart`. Se um hook declarado
não puder nascer ativo, a conversa Codex não abre em um modo diferente do que a
pessoa escolheu. `.zip` e URL continuam disponíveis para sessões Claude e
mostram um erro antes do spawn se forem escolhidos com Codex. Skills, comandos,
MCP e hooks formam o subconjunto portátil; `agents/*.md` continua exclusivo do
Claude.

O instalador reconhece marketplaces `.agents/plugins/marketplace.json` e
`.claude-plugin/marketplace.json`. O contrato e as limitações estão em
[`docs/contracts/plugin-marketplace.md`](docs/contracts/plugin-marketplace.md).

### Pergunta, plano e permissão são cards

Os três chegam pelo mesmo cano (`control_request`) e viram cards na conversa:
a pergunta com uma aba por questão, como na TUI, e "Responder" só quando todas
estiverem; o plano em markdown com **sim**, **sim, perguntando** e **mudar**
(o que você escrever volta ao agente como a recusa); a permissão com o input
da ferramenta. Esc interrompe o turno. Um colega olhando a conversa responde
o mesmo card, e a resposta viaja até o Mac do dono.

### Onde a conversa dorme

A do Claude Code é o transcript dele, `~/.claude/projects/<slug>/<id>.jsonl`
— o app não escreve nele, só lê para reabrir a aba e para contar quanto o
contexto pesa. A do Codex o app grava, em `~/.prometeu/chats/<aba>.jsonl`,
nas mesmas linhas que a tela desenhou: o rollout do Codex tem outra forma, e o
id da thread dele fica no estado do workspace para o `thread/resume`.

### Migrar do Prometheus

Quem usava a instalação anterior encontra **Configurações → Aplicativo →
Migrar do Prometheus** enquanto o Prometeu ainda está vazio. A prévia conta
projetos, workspaces, conversas, plugins e configurações antes da confirmação.

O quadro e os históricos são copiados com backup; tokens e caches ficam de fora.
Os worktrees continuam em `~/prometheus/worktrees`, sem duplicar o que costuma
ser a maior parte do disco. Por isso o Prometheus deve ficar fechado depois da
troca: enquanto esses worktrees não forem limpos ou movidos, os dois aplicativos
apontam para as mesmas pastas Git.

### Os scripts do repositório

Worktree separado só serve para editar até a hora de **testar**: worktree novo
vem sem nada que o `.gitignore` esconde — dependências, `.env`, banco, build.
Por isso o repositório declara três comandos, em `.prometeu/settings.toml`
(ou no `.conductor/settings.toml` que ele já tinha):

```toml
[scripts]
setup   = "npm install"                        # quando um worktree nasce
run     = "npm run dev -- --port $PROMETEU_PORT"   # o botão Run
archive = "docker compose down"                # antes de arquivar
```

O `setup` roda sozinho quando o worktree nasce, e a primeira fala do lançador
só vai ao agente depois que ele termina — agente que roda teste antes do
`npm install` conclui coisa errada. Enquanto isso ela fica na tela, apagada,
e o que você escrever vai atrás dela. Se o setup falhar, a fala vai mesmo
assim, com um aviso na frente.

`run` também aceita a forma de vários, e aí o seletor ao lado do botão escolhe:

```toml
[scripts.run.web]
command = "bin/dev --port $PROMETEU_PORT"
default = true
```

No ambiente de todo script: `$PROMETEU_WORKSPACE_PATH`, `$PROMETEU_ROOT_PATH`,
`$PROMETEU_WORKSPACE_NAME` e `$PROMETEU_PORT` — mais os mesmos nomes com
prefixo `CONDUCTOR_`, para um settings.toml copiado de lá funcionar sem edição.
E `$PORT`, com o mesmo valor: é a convenção que Rails, Next, Express e o
Procfile do Heroku já leem, então um `npm run dev` digitado no terminal do dock
sobe na porta do worktree sem script nenhum.

**A porta é o detalhe que faz a coisa toda funcionar.** Cada workspace guarda
dez portas suas, `$PROMETEU_PORT` até `+9`. Porta fixa no script faz o segundo
worktree não subir — e não subir dois é justamente não conseguir comparar duas
mudanças. A porta sai do caminho do worktree, então o mesmo worktree ganha a
mesma porta em qualquer Prometeu — o instalado e o `tauri dev` de cada
worktree têm estados separados, e sem isso cada um entregava 3100 para o seu.

Worktree sem o arquivo usa o do clone de origem. É o que faz um `.prometeu/`
no `.gitignore` — configuração sua, num repositório de empresa — continuar
valendo em todo worktree que nasce dele; "Abrir o settings.toml" nesse worktree
copia o herdado para lá, e a cópia passa a mandar.

Nada disso é descoberto: o repositório declara. O que o Prometeu faz é não
deixar isso virar trabalho manual — a aba **Setup** de um repo que não declara
nada oferece **Perguntar ao agente**, que abre uma conversa com o prompt pronto
para o Claude Code ler o repositório e escrever o arquivo.

### Os arquivos do worktree

A aba **Arquivos** do painel da direita é a árvore do worktree, e clicar num
arquivo o abre no centro, no lugar da conversa. Código abre pronto para
escrever, com ⌘S para salvar e Esc para desistir; se o agente mexeu no arquivo
enquanto você editava, salvar recusa em vez de passar por cima. PDF abre com
rolagem e zoom, página a página. CSV vira tabela com cabeçalho fixo: o
separador é adivinhado pela primeira linha (`;` do Excel em pt-BR, tab ou
vírgula), campo entre aspas com vírgula ou quebra de linha dentro fica
inteiro, arquivo latin-1 não vira caractere quebrado, e as linhas entram aos
lotes conforme a rolagem — um CSV de cem mil linhas abre sem travar a janela.
PDF e CSV são só leitura. Testes em `src/csv.test.ts` e nos testes Rust de
`src-tauri/src/session/files.rs`.

## A dois na mesma conversa

Um time, e dentro dele sessões compartilhadas: o colega vê a conversa
inteira, ao vivo, fala nela, responde os cards, e deixa nota citando o trecho
que quer discutir.

```
Mac do dono                        relay (Worker + 1 DO por time)        Mac do colega
evento `chat` ──► linhas JSON ──►  presença · shares · quem olha    ──►  a mesma conversa
chat_send    ◄──  fala/card   ◄──  notas · caixa "para mim"         ◄──  o que ele escreve
```

**A sessão continua rodando só no Mac do dono.** Não há VM, não há sessão na
nuvem: o `claude` é o mesmo processo de sempre, no worktree de sempre. O relay
só coordena — repassa frames e guarda o pouco que precisa sobreviver a alguém
estar offline (membros, o que está compartilhado, as notas). Dono fora do ar =
conversa congelada para os outros, e o card diz isso.

**O relay é uma fronteira de confiança, não criptografia ponta a ponta.** Quem
opera o Worker pode ver metadados, conversa e notas que passam por ele. Fora da
máquina local o app só aceita HTTPS/WSS; para conteúdo que o operador do relay
não possa ler, ainda é preciso hospedar o seu próprio relay.

Quem fala com o relay é o **front**: ele já recebe toda linha de toda
conversa e já sabe falar nelas. O back só guarda `~/.prometeu/team.json`
(`0600`, com segredo do convite e credencial individual) e a marca de
"compartilhado" no workspace.

### O time

Configurações → **Time**: criar gera o código de convite
(`pm2.<time>.<segredo>`); entrar é colar o código e dizer seu nome. O convite
serve só para matrícula: o relay devolve uma identidade e uma credencial
próprias para aquele app, e é ela — nunca o segredo coletivo — que autentica o
WebSocket. Códigos `pm1` não migram com segurança; o time precisa ser recriado
ou recebido de novo por um convite `pm2`.

### A sessão ao vivo

Na barra de um workspace seu: **Compartilhar com o time**. Ele aparece em
"Do time" na barra lateral dos colegas, com o seu nome. Abrir mostra a conversa
inteira e o que chega ao vivo; a caixa de escrever está liberada. Você vê quem
está olhando cada conversa em chips ao lado do estado.

Falas de colegas entram identificadas pelo nome. Respostas a cards passam por
uma segunda validação no Mac do dono: só respondem um pedido realmente aberto,
reutilizam o input que o dono viu e não podem ativar modo irrestrito.

Duas coisas fazem isso funcionar sem coordenação nenhuma:

- **cada linha sai numerada** (`Lines`, em `chat.rs`), e a conversa que o dono
  manda a quem acabou de abrir vem com "até a linha N" — então o colega
  descarta o que já estava dentro dela e emenda o resto. Ela vai em partes,
  cortadas em linha inteira, porque o relay limita cada mensagem a 1 MB e a
  aba guarda até 4;
- **o dono só transmite a aba que alguém está olhando.** Sem espectador, o
  custo é zero — o que importa porque o relay cobra por mensagem recebida.

O colega desenha no tamanho da janela dele: são as mesmas linhas, e o mesmo
reducer dos dois lados. Fora do que viaja: o dock (setup/run/shells).

### As notas

Nota não é fala para o agente: é recado entre pessoas **sobre** a sessão, e
entra na própria conversa, na hora em que foi escrita — entre a pergunta do
agente e a resposta que alguém deu. O caso que ela resolve é o agente
levantar uma dúvida de desenho e você precisar de alguém para responder.

A caixa de escrever tem dois modos, **Agente** e **Nota**. A âncora é a
**citação** — o trecho selecionado na conversa (⌘⇧M, ou o botão que aparece
quando há seleção). `@` abre a lista do time; quem foi marcado ganha **Para
mim** na barra, com a nota, mesmo que estivesse offline. ⌘↵ envia; Enter
quebra linha.

### O relay

Mora em `relay/`: um Worker que cria times e encaminha cada conexão ao Durable
Object daquele time. Toda decisão está em `relay/src/logic.ts`, um `reduce`
puro; `room.ts` converte WebSocket em evento e efeito em `send`/`storage`. Além
dos testes puros, a suíte sobe o Worker local e confere matrícula e autenticação
reais. Sobe uma vez:

```sh
npm run relay:deploy   # precisa de `wrangler login`
npm run relay:dev      # ou o relay local, em ws://127.0.0.1:8787
```

Com o relay local, `VITE_RELAY=ws://127.0.0.1:8787` aponta o app (ou o
navegador sobre o `src/mock.ts`) para ele, e dois deles testam o
compartilhamento de ponta a ponta. Sem relay publicado, o app não tem padrão:
a URL vai à mão em Configurações → Time → Relay.

## Rodar

```sh
npm install
npm run app          # o app deste worktree, isolado (ver scripts/app.sh)
npm run dev          # só o front, no navegador, com um back falso — para mexer na UI
PORT=1421 npm run dev  # …e em outra porta, para dois lado a lado
```

Aponte um repositório git e um nome de branch, e clique em **Criar sessão**.

### Dois Prometeu ao mesmo tempo

`npm run app` passa por `scripts/app.sh`, que dá a este worktree porta e
`~/.prometeu-dev-<workspace>` próprios. Sem isso duas coisas colidem: a
porta do vite (`strictPort`, e o segundo não sobe) e o `board.json`.

O `.prometeu/settings.toml` deste repositório é o dogfooding: `run.app` sobe o
app de verdade deste worktree, e `run.browser` abre a mesma UI no Chrome sobre o
`src/mock.ts`. O segundo testa a tela e não o Rust, mas é o único caminho que um
Playwright dirige — a webview do Tauri no macOS é WKWebView e não fala CDP.

## Testes

```sh
npx playwright install chromium  # uma vez nesta máquina
npm test                       # web, relay, Rust e quatro fluxos de navegador
```

Cobre o que erra calado:

- a linha do tempo da conversa (`src/timeline.ts`): o rascunho do streaming
  virando a linha inteira, o resultado achando a ferramenta, o card que fecha
  quando alguém responde, a compactação, as tarefas em segundo plano, o
  transcript reaberto que não pode terminar "chegando";
- o protocolo V1 (`src/conversation.test.ts` e `conversation.rs`): parser,
  equivalência de replay, evento desconhecido e espelho de rollback;
- o tradutor do Codex (`codex.rs`) contra as formas que o app-server manda de
  verdade: a numeração dos blocos, o diff montado do `fileChange`, a pergunta
  que volta no id certo, o `/compact` com antes e depois, a retomada que cai
  para conversa nova;
- o contrato entre o front e os **dois** backs: todo `invoke` de `src/*.ts` tem
  que existir no `generate_handler!` e ter resposta no `src/mock.ts`. Sem essa
  checagem o mock apodrece calado, devolvendo `null` para um comando que nasceu
  só do lado do Rust;
- que **encerrar uma sessão encerra mesmo** — o filho que ignora o desligamento
  educado e o neto que o filho deixou para trás, que é o `node` do servidor de
  dev segurando a porta depois de você mandar fechar;
- a leitura do settings.toml, e a base de onde a branch nova sai, contra um git
  de verdade;
- o corte de um `git diff` em um patch por arquivo, e a conta de número de linha
  que o front faz em cima dele;
- o **relay** pela lógica pura (audiência, cotas, quem recebe o quê, dono que
  cai e volta, menção que vira caixa) e no runtime local do Worker (matrícula,
  credencial individual e recusa do protocolo antigo), o formato dos frames
  binários e a regra de juntar a rolagem do dono com os pedaços ao vivo;
- quatro fluxos Playwright sobre o mock: criação pelo lançador, pergunta e
  resposta de card, troca rápida de abas com snapshot atrasado e recolhimento
  da saída técnica de ferramentas que falharam.

## Estado

Uma lista de workspaces, cada um num worktree, com várias conversas dentro. A
etapa é sua e o estado é do agente — dois eixos que não se misturam. Com time,
a lista também traz o workspace de um colega: a conversa dele ao vivo e as
notas dentro dela.

Se o botão da pergunta não fosse bom, nada disso valeria — então ele veio
primeiro.
