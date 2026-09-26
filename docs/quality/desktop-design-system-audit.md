# Diagnóstico e catálogo de componentes do Desktop

Data: 2026-09-25. Base do diagnóstico: `b03e768`.

Status: catálogo de componentes e adoção em Recursos, Ações, chat e Git
implementados. O objetivo é preservar a personalidade visual, corrigir
inconsistências e permitir descoberta por código e inspeção visual por agentes.
O produto Cloud está fora do escopo. A cobertura e os limites estão descritos
por área abaixo.

## Resultado

O aplicativo e a galeria agora usam a mesma apresentação de Recursos. Ela
recebe dados tipados, textos traduzidos e callbacks. A montagem não importa
hubs, IPC, Tauri, o mock ou armazenamento. Os hubs continuam responsáveis por
instalação, autenticação, confirmação, erros e persistência.

O layout de Recursos tem CSS próprio, usa os tokens existentes e responde à
largura disponível no painel. A paleta escura quente, a fonte de sistema, os
raios pequenos e a navegação compacta permanecem como referências visuais.
Os botões compartilhados deixam de receber as variantes legadas de `ui.css`.

Abra `/design-system.html#resources-preview` após `npm run dev`. O exemplo
permite alternar lista preenchida, catálogo vazio, carregamento, erro
recuperável, operação em andamento, ação indisponível e texto longo. Busca e
filtros são locais. Editar um item simulado atualiza seus dados preservando
foco; nenhuma ação do exemplo chama operações reais.

## Componentes reutilizáveis

Recursos e Ações agora compartilham `sectionHeader`, `toolbar`, `itemRow`,
`overflowAction` e `listState`, definidos em `src/components/compositions.ts`. Cabeçalhos,
linhas, ações e estados têm API tipada, CSS comum e entradas de apresentação.
Não dependem dos modelos de Recursos, catálogos, IPC ou traduções globais.

A galeria `/design-system.html#components-preview` monta outra composição com
essas partes, dados fictícios e callbacks locais, sem importar nenhuma das
duas telas. O verificador de dependências protege essa fronteira. A
[receita para agentes](../architecture/desktop-composition.md) documenta as APIs
e os pontos de uso reais.

## Catálogo e adoção no Desktop

A revisão ampliada resultou em `src/components/`: componentes, estilos e
exemplos ficam organizados por família, com um manifesto `catalog.json` que
informa código, exports, estados e consumidores reais. As primitivas continuam
pertencendo ao pacote compartilhado, acessíveis pela mesma pasta via re-export.

A galeria agora permite busca, navegação por componente, seleção de estado e
URL direta. Exemplo: `/design-system.html?component=diff&state=split`.
`embed=1` abre somente o exemplo para inspeção visual por agentes. O manifesto
JSON também tem um link servido no build. O catálogo possui 21 entradas e
75 estados executáveis, inspecionados no navegador sem inicializar o aplicativo. Não foi adicionado outro framework.

| Área | Implementação atual | Responsabilidade preservada no controlador |
| --- | --- | --- |
| Ícones | Famílias de ícones em `components/icons.ts`, catálogo visual, botão com ícone comum no composer e Git. | Escolha do ícone conforme os dados. |
| Blocos do chat | `chat/blocks.ts` renderiza texto, pensamento, ferramenta, trabalho e erro; ChatView chama essas funções. | Timeline, streaming e coordenação das atualizações. |
| Perguntas, permissões e planos | `chat/requests.ts` apresenta cartões e retorna respostas tipadas por callbacks. | Mudança de permissão e envio da resposta canônica. |
| Composer e anexos | `chat/composer.ts` constrói a estrutura e os controles; ChatView usa essa implementação. | Rascunhos, conclusão de comandos, voz, arquivos, capacidades e envio. |
| Conteúdo e Markdown | `chat/content.ts` e `chat/markdown.ts` concentram os renderizadores existentes, com facades de compatibilidade. | Contexto da conversa e relatório do resultado da cópia. |
| Git | Linhas, grupos e formulário de commit extraídos e consumidos por Workspace Changes. | Disponibilidade, recusas, confirmação, conflitos e operações Git. |
| Diff | `git/diff-view.ts` tem cache, colapsos e observador por instância. A galeria monta dois leitores independentes. | Persistência de revisão em `src/diff.ts`, com as mesmas chaves e assinaturas. |
| Recursos e Ações | Compartilham cabeçalhos, barras, linhas, menus e estados; Recursos tem apresentação isolada. | Hubs, editores e persistência. |

As stories importam componentes de produção; não mantêm uma cópia visual.
O teste do catálogo verifica que os consumidores declarados alcançam os arquivos
por imports reais. Os limites de dependência proíbem IPC, controladores,
imports de stories e efeitos diretos de rede ou armazenamento nos componentes.
Componentes de domínio usam o adaptador i18n existente; ele lê a preferência de
idioma, sem carregar dados do aplicativo.

O [guia da pasta components](../../src/components/README.md) descreve APIs,
descoberta, exemplos, integração e descarte. O contrato de apresentação registra
quais estados pertencem aos componentes e quais permanecem no aplicativo.

A migração não transforma cada função auxiliar ou layout do shell em componente.
Outros fluxos de Configurações, launcher, árvore e painéis mantêm suas composições
atuais, embora consumam as primitivas e os ícones canônicos. O manifesto lista
os componentes efetivamente disponíveis; não apresenta essa cobertura como
refatoração completa de todas as telas.

## Inspeção visual reproduzível

Execute `npm run dev` e abra `/design-system.html` para inspecionar os
componentes atuais. O exemplo `/design-system.html?component=diff&state=two-instances`
mostra dois leitores independentes; `#components-preview` e `#resources-preview`
apresentam as composições compartilhadas e a biblioteca de Recursos.

Para conferir o layout, use a interface em inglês com viewports de 1440 × 1000
e 900 × 800 no aplicativo com mock, e 1280 × 900 ou 460 × 800 na galeria.
Essas verificações no navegador não substituem a validação nativa no WKWebView.
A galeria executa os componentes de produção; screenshots não são necessárias
para seu funcionamento nem para os testes.

## Achados e tratamento

| Achado na base anterior | Tratamento no piloto |
| --- | --- |
| Em 900 × 800, o ícone continuava visível após a remoção de sua coluna. O nome recebia apenas 48 px. | A composição deixa de usar `.setrow` e `.glyph`. As container queries de `resources` ocultam o ícone e removem sua coluna juntas. Os testes verificam espaço para o nome e ações dentro do painel. |
| `button.md` e `button.outline` modificavam os botões compartilhados: padding de 16 px virava 12 px, e `--fg` virava `--fg-2`. | Os seletores legados excluem `.ui-button`. Um teste de navegador compara a aparência compacta entre a galeria independente e o Desktop. |
| Ações chamava `ui.button`, mas substituía `className`, descartando a identidade do componente. | As variantes usam `classList.replace`; as classes locais são acrescentadas, mantendo `.ui-button`. |
| Recursos obtinha `settingsRows().slice(1)` dos hubs e extraía metadados de DOM. | Hubs projetam `ResourceItem[]`; composição recebe dados e callbacks. Não existem controles ocultos para intermediar essas ações. |
| As galerias mostravam primitivas, mas nenhuma composição real isolada de Configurações. | A galeria importa `resourceView`, a mesma função usada pelo aplicativo. |

A extração mantém as regras e os fluxos existentes dos catálogos no Desktop.
A integração com a main mantém uma linha por recurso, com badges de todas as
origens e instalação por origem; chaves de foco distinguem itens locais e grupos
de definições pendentes. Operações pendentes permanecem sob controle dos hubs durante
atualizações da tela. Recursos embutidos continuam sem ações de edição.

## Organização entregue

- `src/resources/model.ts`: dados de apresentação e correspondência de busca.
- `src/components/resource-view.ts` e `resource-view.css`: composição, interação e layout.
- `src/resources/labels.ts`: adaptação do catálogo i18n.
- `src/resources/gallery.ts`: exemplos executáveis, sem integração real.
- `src/settings-resources.ts`: conexão entre apresentação e projeções dos hubs.

O [contrato de apresentação](../contracts/desktop-presentation.md) define
entradas, callbacks, atualização, foco e descarte. A
[receita para agentes](../architecture/desktop-composition.md) mostra um exemplo
mínimo e as regras de composição. A
[ADR 0060](../decisions/0060-isolated-desktop-presentation.md) registra a escolha
por essa separação incremental com o DOM e as dependências existentes.

## Validação

`npm run check` passou: documentação, dependências, formatação Rust, builds
Desktop e mobile, typechecks, testes de release, suítes web, Rust e navegador,
e Clippy. A suíte Rust mantém 7 testes
ignorados já declarados no projeto.

A evidência específica inclui:

- `src/components/catalog.test.ts`: manifesto, stories e consumidores reais.
- `src/resources/adapters.test.ts`: projeções dos hubs sem DOM, identidade por
  origem, ações permitidas e bloqueio durante autenticação MCP.
- `src/settings-navigation.test.ts`: destinos anteriores e correspondência de busca.
- `e2e/settings.spec.ts`: preservação de foco e filtros, galeria isolada,
  recuperação de erro e geometria em Chromium e WebKit.
- `e2e/mcp.spec.ts` e `e2e/cloud.spec.ts`: fluxos existentes do Desktop, incluindo
  resultados obsoletos, cópia local, confirmação e revisão do catálogo.
- `e2e/design-system.spec.ts`: paridade de espaçamento e cor do botão compacto.
- `scripts/architecture-dependencies.test.mjs`: rejeição de imports indiretos
  que fariam a apresentação depender da integração.

Os testes de navegador adicionados tratam riscos concretos de foco após
substituição do DOM e cascata CSS. Não repetem a matriz de interação das
primitivas em cada tela. As operações e o catálogo têm provas de nível inferior.

## Limites e continuidade

As stories isoladas de chat e Git não têm E2E dedicado. A galeria permite
inspeção visual; os testes existentes do produto cobrem sua integração.

A implementação cobre o catálogo, Recursos, Ações e as extrações de chat e Git
descritas acima. TypeSafe, telemetria e o restante de Configurações mantêm
suas composições atuais. Carregamento e erro da lista inteira são entradas demonstradas na
galeria; a política de carregamento inicial dos hubs não foi alterada.

Nenhum formato persistido, IPC ou protocolo mudou. Não foram adicionados
frameworks ou dependências. A execução nativa, provedores reais e o produto
Cloud não foram exercitados nesta validação.

Na inspeção com Vite dev, o mock apresentou a rejeição de menu nativo
`object null is not iterable (cannot read property Symbol(Symbol.iterator))`,
com stack em `@tauri-apps/api/menu` e `src/appmenu.ts`. Esse problema anterior
não impediu a navegação inspecionada e não foi atribuído ao Design System.

As próximas extrações devem partir de uma tela concreta, usando a galeria e os
componentes existentes. Promover uma composição ao pacote compartilhado exige
outro uso real; este piloto não impõe uma reorganização geral de pastas.
