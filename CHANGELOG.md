# Changelog

What changes in Prometeu, version by version, for whoever uses the app.
Versions come from `sh scripts/release.sh`; this file is generated from the
commits by git-cliff, and each release's notes on GitHub are its section.

## [0.18.1] - 2026-09-26

### Fixes

- **design-system:** Show expandable work cards in the gallery
- **resources:** Keep plugin installation busy until confirmation closes

## [0.18.0] - 2026-09-25

### New

- **viewer:** Preview workspace images
- **resources:** List each resource once with every catalog that offers it

### Fixes

- **viewer:** Preserve SVG editing with image preview
- **catalog:** Keep non-GitHub plugin sources exact when comparing

## [0.17.0] - 2026-09-25

### New

- **chat:** Show sent attachments as numbered image tags
- **telemetry:** Add local usage history with export and deletion
- **workspace:** Open worktrees on existing branches

### Fixes

- **linux:** Attach screenshots pasted into the chat
- **linux:** Read pasted screenshots from the gtk clipboard
- **linux:** Keep screenshot paste responsive
- **typesafe:** Make request review work with Jev
- **settings:** Show TypeSafe key prerequisite
- **chat:** Keep sent attachment tags inside the bubble on desktop and mobile
- **telemetry:** Keep local history accurate and responsive
- **telemetry:** Preserve existing files when export fails
- **telemetry:** Persist export replacements before reporting success
- **telemetry:** Record archive transitions after board persistence
- **workspace:** Keep archiving responsive during telemetry save
- **workspace:** Preserve branch ownership per repository

## [0.16.0] - 2026-09-24

### New

- **files:** Create, rename, trash and restore files from the file tree

### Fixes

- **files:** Protect git metadata, keep staged edits on restore and allow case-only renames
- **files:** Keep concurrent case-only renames from replacing each other
- **linux:** Open the AppImage on current Mesa instead of a black window
- **linux:** Keep the libwayland check from passing on a closed pipe

## [0.15.0] - 2026-09-24

### New

- **files:** Mark new, modified and conflicted files in the file tree
- **files:** Tint changed files and show deleted ones struck through
- **files:** Right-click a file in the tree to send it to the conversation
- **viewer:** Preview markdown files in reader mode
- **settings:** Connect an optional typesafe key
- **launcher:** Review a request for missing context before starting
- **changes:** Right-click a changed file to open, stage, discard or copy its path
- **launcher:** Start a conversation from a skill with a project-declared artifact path
- **viewer:** Find text in the open file and quick-open files by name

### Fixes

- **terminal:** Stop duplicating dead-key characters on linux
- **files:** Keep ignored files in new folders unmarked in the file tree
- **files:** Keep the file tree on the workspace on screen
- **files:** Announce a copied path only when the clipboard took it
- **tabs:** Prevent desk labels from overlapping
- **clipboard:** Report success only after copying
- **clipboard:** Translate workspace path copy errors
- **viewer:** Refresh markdown preview when discarding draft
- **chat:** Keep the conversation working until its subagents finish
- **chat:** Stop a resumed turn from settling on its subagents
- **settings:** Bind typesafe evaluations to the key they read and bound their context
- **changes:** Recheck git availability when a menu item runs
- **launcher:** Keep the chosen skill across provider switches and resumes
- **viewer:** Keep quick open and find results accurate while typing and editing
- **files:** Highlight the file the viewer is showing in the tree
- **files:** Show staged-then-deleted files in the tree and keep its marks current

### Performance

- **files:** Keep file tree git scans off the main thread

## [0.14.1] - 2026-09-23

### Fixes

- **release:** Keep linux downloads at their documented address

## [0.14.0] - 2026-09-23

### New

- **desktop:** Run on linux with freedesktop integrations
- **ui:** Show ctrl shortcuts and neutral wording outside macos
- **release:** Offer linux downloads and updates

### Fixes

- **release:** Verify updater release notes

## [0.13.0] - 2026-09-23

### New

- **notifications:** Add opt-in mac alerts with independent sound
- **settings:** Simplify settings navigation

### Fixes

- **notifications:** Refine notch and clarify banner authorization
- **notifications:** Sign development bundles for native authorization

## [0.12.1] - 2026-09-22

### Fixes

- **cleanup:** Clarify worktree retention after archiving

## [0.12.0] - 2026-09-22

### New

- **feedback:** Support native image drops

### Fixes

- **feedback:** Keep pending images in sync with the form
- **dictation:** Allow microphone access on macos
- **browser:** Deny remote media capture
- **mcp:** Expose connection checks and local login
- **mcp:** Guard refreshes and avoid redundant login
- **catalog:** Avoid duplicate local installs
- **catalog:** Keep duplicate checks responsive and consistent
- **projects:** Serialize local changes with catalog installs

## [0.11.0] - 2026-09-21

### New

- **chat:** Copy complete code blocks

### Fixes

- **antigravity:** Scope sessions to worktree
- **antigravity:** Normalize workspace path
- **chat:** Keep copy confirmation during streaming
- **chat:** Preserve newer status after copying

## [0.10.0] - 2026-09-20

### New

- **models:** Add searchable live catalogs and favorites

### Fixes

- **ui:** Improve workspace and dialog layouts
- **ui:** Align accounts and space model favorites

## [0.9.0] - 2026-09-20

### New

- **accounts:** Manage authentication methods in shared account settings
- **gemini:** Run conversations with isolated google and api key accounts
- **antigravity:** Replace the discontinued gemini cli integration
- **antigravity:** Show account quota usage by model group
- **projects:** Clone and register catalog repositories

### Fixes

- **accounts:** Preserve unknown provider entries
- **gemini:** Open google login directly in the browser
- **antigravity:** Explain denied commands instead of reporting success
- **antigravity:** Allow automatic execution in ordinary conversations

## [0.8.0] - 2026-09-19

### New

- **feedback:** Add public bug reporting

### Fixes

- **chat:** Preserve request kinds and per-tab transcripts
- **cleanup:** Warn before deleting ignored files
- **files:** Clarify the finder button tooltip
- **settings:** Remove duplicate tool defaults

## [0.7.8] - 2026-09-18

### New

- **mcp:** Delegate tasks to agents in isolated workspaces
- **mcp:** Run workspace scripts and open previews
- **mcp:** Support external local agents

### Fixes

- **update:** Keep offered release current
- **relay:** Preserve requests after oversized input

## [0.7.7] - 2026-09-17

### New

- **workspaces:** Offer cleanup after archiving

### Fixes

- **chat:** Stop unresponsive dictation

## [0.7.6] - 2026-09-16

### New

- **tools:** Complete layered selection with project trust and provenance
- **tools:** Show the CLI's MCP servers as the picker's inherited base

### Fixes

- **tools:** Harden layered selection after adversarial review
- **tools:** Preserve inheritance and project trust

## [0.7.5] - 2026-09-15

### New

- **team:** Organization tab lists people and adopts a peer's new key automatically
- **chat:** Dictate messages through the microphone in the message box

## [0.7.4] - 2026-09-11

### New

- **open source:** Source code is public under MIT and updates now come from prometeucorp/prometeu

## [0.7.3] - 2026-09-11

### New

- **catalog:** Installs organization tools directly on desktop

## [0.7.2] - 2026-09-11

### New

- **settings:** Removes the Prometheus migration
- **browser:** Integrates browsing and element selection into chat

## [0.7.1] - 2026-09-10

### New

- **team:** A second Mac on the same account shares as your own device

## [0.7.0] - 2026-09-10

### New

- **feedback:** Asks for a connected account to send reports
- **attachments:** Pastes clipboard images into chat and workspace creation

### Fixes

- **tabs:** Selects the remaining conversation when a tab closes
- **pairing:** Stabilizes connections and improves mobile messaging

## [0.6.2] - 2026-09-10

### New

- **feedback:** Allows sending feedback with images and screen captures

### Fixes

- **chat:** Organizes controls in the conversation footer
- **tabs:** Keeps conversations accessible when opening a workspace after project files

## [0.6.1] - 2026-09-09

### New

- **remote control:** Lets you control workspaces from mobile devices

### Fixes

- **alerts:** Remove completion and comment sounds

## [0.6.0] - 2026-09-08

### New

- **collaboration:** Follow shared conversations from your phone

## [0.5.13] - 2026-09-08

### New

- **organizations:** Joins the only organization automatically

### Fixes

- **alerts:** Plays completion sound once per run

## [0.5.12] - 2026-09-08

### New

- **collaboration:** Protects shared content with end-to-end encryption
- **changes:** Makes reviewing diffs and preparing commits easier

### Fixes

- Preserves edits and sessions during concurrent operations

## [0.5.11] - 2026-09-07

### New

- **projects:** A folder without git can be a project too
- **ui:** Tabs, menu separators and compact density in the design system
- **ui:** Avatar with photo or glyph in the account menu and identity blocks
- **organizations:** Shares workspaces through Cloud organizations

### Fixes

- **changes:** Clicking a file no longer pushes the app header off screen
- **tabs:** Preserves clicks during workspace updates

## [0.5.10] - 2026-09-07

### New

- **catalog:** Share plugins, MCPs and skills by choice
- **sidebar:** Agent rows in the list fit on a single line

## [0.5.9] - 2026-09-07

### New

- **changes:** Shows the whole group's diff in one scroll
- **cloud:** Plugins, MCP and Actions follow your account across Macs

### Fixes

- **account:** Sidebar account button is compact

## [0.5.8] - 2026-09-07

### Fixes

- **chat:** Supports dragging screenshot thumbnails into conversations

## [0.5.7] - 2026-09-07

### New

- **account:** Connects an optional account through the browser
- **projects:** Opens the project files without creating a workspace

## [0.5.6] - 2026-09-06

### New

- **sidebar:** Multi-repo workspaces get a Sets group with a sliced avatar
- **terminal:** Free terminal becomes a center tab
- **changes:** Changes tab only shows up when you open it
- **actions:** Adds reusable agents and shared interface components
- **accounts:** Manages Claude and Codex accounts from the footer

### Fixes

- **alerts:** Prevents repeated sounds while agents are running

## [0.5.5] - 2026-09-05

### New

- **board:** Unnamed tabs show the model and what the agent is doing

### Fixes

- **conversation:** Stops repeating the first message when launching a workspace
- **conversation:** Bell stops ringing while Codex is still working

## [0.5.4] - 2026-09-05

### New

- **git:** Manage staging, commits and branches in the changes panel
- **sidebar:** Shows agents and their status per workspace
- **team:** Ships with a default relay for joining a team
- **workspaces:** Expands variety of generated branch names

### Fixes

- **desk:** Centers empty state on screen
- **codex:** Hides experimental feature warning when starting a session

## [0.5.3] - 2026-09-05

### New

- **distribution:** Signs app and publishes notes in both languages
- **collaboration:** Keeps comments beside shared sessions

## [0.5.2] - 2026-09-04

### New

- **projetos:** Permite remover projeto
- **conversa:** Mostra tokens acumulados por sessão
- **marca:** Adota nova identidade visual
- **sidebar:** Usa marca oficial e oculta etapa
- **mesa:** Mostra todas as conversas de pé na tela inicial

### Fixes

- **migração:** Mantém o texto da confirmação dentro da caixa
- **conversa:** Recolhe erros técnicos contínuos

## [0.5.1] - 2026-09-04

### New

- **migração:** Importa dados do Prometheus

## [0.5.0] - 2026-09-04

### New

- **Breaking:** **marca:** Apresenta o Prometeu como aplicativo independente
- **plugins:** Leva o marketplace de workspace ao Codex

### Fixes

- **cotas:** Exibe limites do codex por modelo
- **linear:** Usa cadastro OAuth do Prometeu

## [0.4.19] - 2026-09-04

### New

- **sidebar:** Mostra o provedor dos workspaces
- **viewer:** Abrir PDF e CSV na tela, direto da árvore de arquivos

### Fixes

- **viewer:** Trocar de PDF para CSV não deixa o PDF em cima da tabela
- **viewer:** Cabeçalho do CSV gruda no topo sem linha passando por cima
- **viewer:** Some a fresta de 1px acima do cabeçalho do CSV ao rolar
- **viewer:** PDF e CSV mostram só o conteúdo, e reabrir o CSV começa do topo

## [0.4.18] - 2026-09-03

### New

- **configurações:** Escolher o modelo, o esforço, os MCP e os plugins padrão

### Fixes

- **conversa:** Arrastar arquivos para o chat volta a funcionar

## [0.4.17] - 2026-09-03

### New

- **conversa:** Trocar o modelo e o esforço sem abrir conversa nova
- **plugins:** Instalar plugin pelo endereço do repositório, sem passar pelo CLI

### Fixes

- **conversa:** Marcar plugin ou MCP responde na hora, em vez de parecer travado

## [0.4.15] - 2026-09-02

### New

- **rodapé:** A cota aparece atualizada desde que o app abre, sem esperar conversa
- **ferramentas:** Cadastrar servidor de MCP em dois passos, vendo cada etapa
- **ferramentas:** Escolher por workspace quais plugins o agente carrega
- **issues:** Filtrar a lista por time

### Fixes

- **rodapé:** Manter a tela e o Mac acordados de verdade
- **conversa:** O que o app põe no ambiente do agente para de ser apagado
- **conversa:** Pergunta com opção longa não escapa mais do card
- **lançador:** Mantém controles dentro da caixa com branches longas
- **lançador:** Lista de issues volta a caber e mostrar os títulos

## [0.4.14] - 2026-09-02

### New

- **ferramentas:** A escolha de MCP passa a valer também nas abas do Codex
- **lançador:** Modelos do Claude Code acompanham o catálogo da conta

### Fixes

- **navegador:** Janelas do app aparecem por cima da aba de navegador
- **navegador:** Endereço digitado não escapa para o Chrome no primeiro redirecionamento
- **navegador:** A aba navega de verdade, sem despejar os anúncios da página no navegador do computador

## [0.4.13] - 2026-09-01

### New

- **ferramentas:** O app cadastra servidores de MCP, testa e entra nos que pedem login
- **ferramentas:** Escolha quais MCP o agente enxerga, e Configurações vira páginas
- **conversa:** O "+" abre o Finder e o arquivo vira anexo da fala

## [0.4.12] - 2026-09-01

### New

- **conversa:** Um "+" na caixa aponta um arquivo do workspace
- **lançador:** A branch do workspace novo tem nome de palavras, sem data e hora

### Fixes

- **conversa:** O rascunho da fala fica na aba em que foi escrito

## [0.4.11] - 2026-09-01

### New

- **navegador:** Botões de voltar e avançar na barra da aba

### Fixes

- **atalhos:** Funcionam com o cursor dentro da página, e ⌘W fecha a aba
- **lançador:** A branch já aberta em outro workspace avisa antes de criar

## [0.4.10] - 2026-09-01

### New

- **rodapé:** Quanto da cota de cada agente já foi, na barra de baixo
- **rodapé:** Memória, terminais e portas do app na barra de baixo
- **rodapé:** Escolher quando o Mac não pode dormir

### Fixes

- **rodapé:** A cota some da barra antes da primeira resposta do agente
- **rodapé:** Esclarece cotas e quando o Mac fica acordado

## [0.4.9] - 2026-08-31

### New

- **novidades:** O que mudou no app aparece dentro dele, e não só na página de releases

### Fixes

- **quadro:** PR continua aparecendo quando a branch do worktree foi renomeada
- **quadro:** Repositório some da lista de mudanças quando o filtro está ligado

## [0.4.8] - 2026-08-31

### New

- **arquivos:** Editar e salvar o arquivo aberto sem passar pelo agente
- **mudanças:** Duplo clique num arquivo mudado abre ele para editar
- **arquivos:** O arquivo abre pronto para escrever, e continua colorido
- **conversa:** O @ na caixa completa o caminho de um arquivo do workspace
- **conversa:** A lista do @ acerta melhor, e o arquivo recém-mexido vem na frente

### Fixes

- **conversa:** Mensagens não ficam presas ao retomar uma aba

### Performance

- **conversa:** A lista do @ aguenta um monorepo

## [0.4.7] - 2026-08-31

### Fixes

- **mudanças:** Arquivo não passa mais por cima do cabeçalho do repositório
- **barra:** Simplifica o topo e estabiliza seus estados
- **time:** Aviso de workspace fora do time não aparece mais sozinho

## [0.4.6] - 2026-08-30

### New

- **conversa:** Aba nova pode falar com outro modelo, sem sair do workspace
- **lançador:** A lista de modelos não tem mais "Modelo padrão" — escolhe-se sempre um
- **mudanças:** O painel diz se os commits já foram para o remoto
- **barra:** Um botão de PR só, com os PRs de cada repositório num menu

### Fixes

- **navegador:** Link clicado abre no navegador do computador, e a aba de dentro fica só no Run
- **lançador:** Branch do workspace novo leva o dia, e não repete nome já usado
- **mudanças:** Commit feito no terminal some da lista sem esperar o agente

### Performance

- **mudanças:** Workspace com muitos arquivos abre na hora e rola liso

## [0.4.5] - 2026-08-30

### New

- **mudanças:** Workspace com mais de um repositório mostra as mudanças de todos, uma seção por repo
- **mudanças:** A tela mostra a branch inteira contra a base, com "visto" por arquivo e filtro do que está fora de commit
- **lançador:** Workspace com mais de um repositório tem um PR por repo, e "Concluir" só quando todos entraram

### Fixes

- **mudanças:** As linhas de contexto do diff voltam a ser linhas, e não caixas
- **segurança:** Protege projetos, sessões e colaboração
- **interface:** Remove confirmação e recolhe erros técnicos

## [0.4.4] - 2026-08-29

### New

- **lançador:** Um workspace pode juntar mais de um repositório, cada um num worktree na mesma branch

## [0.4.3] - 2026-08-29

### New

- **conversa:** Escrever / na caixa lista os comandos e skills da sessão

## [0.4.2] - 2026-08-28

### New

- **quadro:** Compartilhar um workspace só com alguns colegas do time

### Fixes

- **conversa:** O texto da skill fica dentro do card dela, não como mensagem

## [0.4.1] - 2026-08-28

### New

- **quadro:** Pling e bolinha no Dock quando um agente para ou uma nota te marca

### Fixes

- **quadro:** Escolher um nome na lista do @ troca o que foi digitado em vez de repetir
- **chat:** Nota nova rola a conversa até o fim

## [0.4.0] - 2026-08-28

### New

- **Breaking:** **quadro:** Remove o Kanban e usa Issues como tela inicial
- **conversa:** A conversa e o painel no desenho do Conductor

### Fixes

- **codex:** Aceita caminhos absolutos e omite logs duplicados
- **conversa:** Cada workspace lembra se a caixa estava em fala ou nota
- **setup:** Preparar um worktree novo não quebra mais no meio

## [0.3.1] - 2026-08-28

### New

- **conversa:** O trabalho seguido do agente vira um cartão só

### Fixes

- **time:** Digitar @ na nota escreve o @, e escolher o nome completa ele
- **time:** Nota que não sai avisa, em vez de sumir com o que você escreveu

## [0.3.0] - 2026-08-28

### New

- **quadro:** Marcar o vermelho na limpeza apaga o worktree do mesmo jeito
- **lançador:** Escolher um modelo GPT roda a sessão no Codex
- **conversa:** A conversa vira chat desenhado pelo app, com plano, pergunta e notas do time dentro dela
- **time:** Quem abre a conversa de um colega recebe a conversa inteira, não só o fim
- **conversa:** Perguntas do agente em abas, uma por pergunta, como na TUI
- **conversa:** A fala em espera do setup, o que roda em segundo plano, a compactação e o diff aparecem na tela
- **conversa:** /context vira um painel — barra por categoria e seções dobradas por servidor
- **conversa:** Escolher um GPT abre a conversa no Codex, com o mesmo chat
- **conversa:** O Codex faz pergunta com card, como o Claude Code

### Fixes

- **conversa:** A primeira fala vai assim que a conversa sobe, e as seguintes não ficam presas
- **conversa:** O texto do agente quebra linha, chega sem tremer e mostra que está trabalhando
- **conversa:** Abrir "Pensando…" mostra o pensamento
- **conversa:** O caret some quando a mensagem termina, e "Pensou" perde a moldura tracejada
- **conversa:** Reabrir uma conversa não deixa a última mensagem "chegando"

### Performance

- **lançador:** Criar sessão abre na hora e o worktree prepara por trás

## [0.1.14] - 2026-08-26

### New

- **time:** Criar um time, entrar com o código e ver quem está online, em Configurações
- **quadro:** Compartilhar um workspace com o time, e ver quem está olhando
- **terminal:** Abrir a conversa de um colega ao vivo, com a rolagem inteira e o teclado liberado
- **notas:** Comentar uma sessão — a sua ou a de um colega — citando o trecho do terminal
- **quadro:** Os arquivados saem da barra e ganham uma tela com busca
- **dock:** Open abre o run numa janela do próprio app, e ⌥-clique no navegador
- **quadro:** O Run abre numa aba de navegador, ao lado da conversa
- **quadro:** A aba de navegador ganha barra de endereço
- **quadro:** Pergunta, plano e permissão ficam no terminal, sem card por cima

### Fixes

- **time:** Dono que volta acorda o que compartilhou, e id de colega não colide com o seu
- **quadro:** Limpar worktrees abre na hora e deixa de listar quem roda no próprio clone
- **dock:** A porta reservada nunca cai numa que o navegador recusa

## [0.1.13] - 2026-08-26

### New

- **workspace:** Worktree novo já nasce com o .env do projeto
- **workspace:** PR aberto da branch aparece na barra e leva até ele no navegador
- **lançador:** Workspace novo ganha nome escrito pelo agente
- **quadro:** O PR que entrou vira "Concluir", e a barra devolve os worktrees ao disco

### Fixes

- **quadro:** "Devolver worktrees" vira linha da barra, em vez de ícone que só o mouse achava
- **quadro:** A limpeza de worktree se chama "Limpar worktrees" na tela inteira

## [0.1.12] - 2026-08-25

### New

- **quadro:** Barra lateral agrupa os workspaces por projeto, com a etapa na linha

## [0.1.11] - 2026-08-25

### New

- **dock:** Worktree herda o settings.toml do clone, e a porta do Run não se repete entre worktrees
- **idioma:** O app fala inglês, e começa no idioma do computador
- **issues:** Recolher grupo de issues, e o grupo fechado continua fechado
- **updater:** Botão "Buscar atualizações" em Configurações, com a versão e a hora da última busca

## [0.1.10] - 2026-08-24

### New

- **painel:** Arrastar a borda esquerda muda a largura do painel da direita
- **dock:** Botão Open abre o run no navegador enquanto ele está de pé
- **workspace:** Botão Open PR pede o pull request à conversa ativa

## [0.1.9] - 2026-08-24

### New

- **configurações:** Conectar o Linear pela nova tela de configurações
- **issues:** Aba com as issues do Linear no seu nome, e criar workspace a partir de uma
- **lançador:** Sem a seção Detalhes — nome, branch e etapa saem sozinhos
- **lançador:** Escolher uma issue do Linear no próprio lançador

### Fixes

- **updater:** Clicar em reiniciar depois de baixar a atualização reinicia o app de verdade

## [0.1.8] - 2026-08-23

### Other

- Tira o aviso de "solto no seu clone" do rodapé
- O card mostra quantos tokens de contexto a conversa tem

## [0.1.7] - 2026-08-23

### Other

- Arrastar card entre colunas volta a funcionar
- Marcar o package-lock junto com o package.json
- Modelo, esforço e plan mode no rodapé, e sempre solto
