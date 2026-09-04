# Contrato do hub de plugins

Status: contrato vigente; adaptação portátil decidida pelo ADR 0005.

Este contrato define como um único catálogo do Prometeu alimenta sessões do
Claude Code e do Codex. O hub é estado do produto; manifests, argumentos e
caches de cada CLI são detalhes dos adapters.

## Hub

O cadastro vive em `<root>/plugins.json` e conserva a forma existente:

```ts
type Plugin = {
  id: string;
  source: string;
  note: string;
  made: boolean;
  from: string;
};
```

`id` é a identidade estável guardada no workspace. `source` pode ser uma pasta
local, um `.zip` local ou a URL de um `.zip`; cada adapter decide quais dessas
origens consegue materializar. `made` apenas diz se a pasta foi criada ou
clonada pelo Prometeu e, portanto, se pode ser apagada pelo app.

Ao instalar um repositório de marketplace, o importador procura, nesta ordem:

- um plugin compatível na raiz;
- `.agents/plugins/marketplace.json`;
- `.claude-plugin/marketplace.json`;
- plugins compatíveis um nível abaixo da raiz ou dentro de `plugins/`.

Uma entrada local do marketplace Codex usa
`{"source":{"source":"local","path":"./plugins/x"}}`; a forma Claude usa
`{"source":"./plugins/x"}`. Somente caminhos internos ao clone são seguidos.
Entradas remotas são outra instalação e não fazem o importador atravessar a
fronteira do repositório.

## Seleção por workspace

`Workspace.plugins` contém IDs do hub e vale para qualquer provider escolhido
para aquele workspace:

- `null` preserva o comportamento anterior e não injeta uma seleção;
- `[]` não injeta nenhum item do hub;
- uma lista injeta apenas itens ainda presentes no hub;
- um ID removido do hub é ignorado para que um workspace antigo continue
  abrindo.

A seleção controla o que o Prometeu injeta. Plugins que a pessoa habilitou
diretamente no cadastro global do CLI continuam sujeitos às regras daquele
CLI. No Codex, a configuração real é relida ao preparar cada spawn para que
essas preferências continuem acompanhando o usuário; somente as entradas dos
marketplaces reservados `prometeu` e `prometeu-dev` são controladas pelo
workspace.

Selecionar é ativar. O pacote deve estar habilitado desde o início da sessão e,
quando declara hooks, eles precisam estar ativos antes de a primeira resposta.
Um modo contínuo pode usar `SessionStart` para pôr sua instrução no contexto e
`UserPromptSubmit` para reforçá-la; não depende de comando, menção à skill ou
uma segunda ativação.

## Pacote portátil

Uma pasta que deva funcionar nos dois providers contém
`.claude-plugin/plugin.json`. Ela pode trazer também
`.codex-plugin/plugin.json`; quando existe, o manifesto nativo funciona como
overlay para os campos equivalentes. `name` deve ser o mesmo nos dois e usar
minúsculas e hífens, com no máximo 64 caracteres.

Skills, comandos, scripts, assets, MCP e hooks permanecem dentro da mesma
pasta. Caminhos de manifesto são relativos à raiz do plugin. Quando um pacote
Claude traz `.mcp.json`, o adapter acrescenta `mcpServers` ao manifesto Codex
derivado; comandos são migrados pelo próprio Codex para seu mecanismo de
skills. O Codex também fornece `CLAUDE_PLUGIN_ROOT` aos hooks compatíveis,
portanto o pacote não precisa duplicar scripts apenas para trocar de provider.

Os dois manifests aceitam caminho para um arquivo de hooks, mas seus objetos
inline não têm exatamente o mesmo envelope: no manifesto Claude o objeto é o
mapa de eventos; no Codex ele é um arquivo de hooks completo, com esse mapa sob
`hooks`. Ao criar o overlay derivado, o adapter acrescenta esse envelope sem
mexer na origem. Um overlay `.codex-plugin/plugin.json` fornecido pelo próprio
pacote já é nativo e não recebe essa conversão.

`agents/*.md` não é parte do formato de plugin do Codex atual. Pode coexistir
no pacote e continua funcionando no Claude, mas um fluxo que precise dos dois
providers deve ser modelado como skill. Essa diferença não é escondida por uma
conversão silenciosa para a configuração de subagentes do Codex.

O criador do Prometeu deve gerar os dois manifests. Pacotes antigos com
apenas o manifesto Claude continuam portáveis: o adapter escreve o manifesto
Codex mínimo somente na cópia derivada, sem alterar a origem.

## Adapters

### Claude Code

Cada item escolhido vira `--plugin-dir` para pasta ou `.zip` local e
`--plugin-url` para URL. O source original é entregue diretamente ao CLI.

### Codex

O Codex recebe somente pastas locais. `.zip` e URL continuam suportados pelo
Claude, mas uma tentativa de usá-los numa sessão Codex falha antes de iniciar e
explica qual plugin precisa ser instalado como pasta.

Para toda seleção explícita, inclusive `[]`, `plugins.rs`:

1. deriva de SHA-256 do ID persistido do workspace um home estável em
   `<root>/codex-workspaces/<workspace-hash>/`; usar o ID mantém separados
   inclusive dois workspaces que rodam no mesmo clone sem worktree;
2. espelha nesse home as entradas do `CODEX_HOME` real, exceto os arquivos de
   configuração, mantendo os arquivos de conta, sessões, skills, memória e
   cache no lugar de sempre;
3. calcula um SHA-256 determinístico de cada pasta de plugin, sem `.git`, e
   combina uma revisão do formato derivado para que correções do adapter também
   invalidem snapshots antigos;
4. copia o pacote para
   `<home>/marketplace/plugins/<id>`, mescla o manifesto compatível com o
   overlay nativo e acrescenta o hash à versão como cachebuster;
5. grava `.agents/plugins/marketplace.json` naquele snapshot de workspace;
6. reconstrói `<home>/config.toml` a partir da configuração real, preserva o
   estado de hooks do workspace, desliga entradas antigas do Prometeu e liga
   somente os IDs atuais;
7. consulta e instala pelo CLI estável `codex plugin`, sempre com
   `CODEX_HOME=<home>`;
8. inicia o `codex app-server` com o mesmo `CODEX_HOME`.

O marketplace derivado se chama `prometeu` em release e
`prometeu-dev` em debug, evitando colisão entre os dois estados. Cada
workspace possui seu snapshot, e preparação e instalação são serializadas
porque o cache instalado continua compartilhado.

Instalar ou atualizar pode escrever no cache global do Codex e no
`config.toml` derivado. O `config.toml` do home real não é aberto para escrita.
Remover um item do hub tenta retirar a instalação compartilhada, a entrada de
cada config derivada e as cópias de marketplace.

O isolamento por home é necessário porque o estado `plugins.<id>.enabled` não
aceita hoje um override confiável por `-c`: o CLI consome o argumento, mas o
loader de plugins continua usando a camada persistida. Por isso a ativação
fica numa camada de configuração real, porém descartável, em vez de depender
de uma flag que não produz o efeito prometido.

O armazenamento padrão de login do Codex é `auth.json`. O home derivado liga
esse arquivo ao original e fixa o modo `file` quando a configuração usa o
padrão ou `auto`, para que refreshes não criem tokens divergentes. Uma escolha
explícita por `keyring` ou `ephemeral` é preservada; como o próprio Codex trata
cada `CODEX_HOME` como identidade independente nesses modos, ela pode exigir
API key no ambiente ou autenticação específica para o home.

## Hooks e confiança

Escolher um plugin autoriza o código e os hooks daquele pacote, assim como
passar `--plugin-dir` já autoriza no Claude. Antes de abrir a thread Codex, o
adapter consulta `hooks/list` e grava no config derivado `enabled = true` e a
confiança pelo `currentHash`, apenas para hooks cujo `pluginId` pertence à
seleção atual. Um hook já confiável mas desativado também é religado: a seleção
do workspace prevalece sobre o estado derivado anterior.

Hooks globais, do projeto ou de outro plugin nunca ganham confiança por esse
fluxo. Se o conteúdo mudar, o novo hash só é aceito quando uma sessão que ainda
seleciona o plugin abrir. Se um pacote declara hooks e `hooks/list` não os
atribui ao seu `pluginId`, ou se a gravação da ativação/confiança falha, o
adapter mostra um erro e não abre a thread. Assim a sessão não pode nascer como
uma coleção de skills quando a pessoa escolheu um comportamento automático.

## Falhas e compatibilidade

- falha ao preparar ou instalar um plugin escolhido impede o spawn da sessão;
- falha ao preparar MCP escolhido também impede o spawn, em vez de iniciar uma
  conversa silenciosamente sem ferramentas;
- o home, o marketplace e a cópia em `<root>/codex-workspaces/` são derivados
  e podem ser reconstruídos a partir do hub, da config real e das origens;
- remover ou limpar um workspace apaga somente seu home derivado; os links não
  transformam conta, sessões ou cache compartilhado em ownership do app;
- mudança no esquema do hub, na semântica de seleção ou nos manifests gerados
  exige atualização deste contrato e teste de compatibilidade.
