# ADR 0004 — Identidade independente do Prometeu

Data: 2026-09-04
Status: Aceito

## Contexto

O nome público do produto passou a ser Prometeu e seu domínio é `prometeu.co`.
Alterar em lugar a identidade do aplicativo anterior misturaria bundle,
updater, estado local, worktrees e contratos de script durante a transição.
Isso também impediria instalar os dois aplicativos lado a lado para conferir a
nova linha antes de migrar dados reais.

O código e o histórico Git foram copiados para um repositório novo. Os ADRs e
o changelog anteriores continuam sendo registro histórico e não são
reescritos para fingir que o nome novo sempre existiu.

## Opções consideradas

1. Renomear o aplicativo existente e migrar seus dados durante uma atualização.
2. Compartilhar as mesmas raízes de estado entre os dois nomes.
3. Criar o Prometeu como aplicativo independente e importar dados depois.

## Decisão

O Prometeu possui repositório, bundle, executável, updater, namespace local,
configuração de projeto e variáveis de ambiente próprios:

- bundle id `co.prometeu.desktop`;
- estado em `~/.prometeu` e `~/.prometeu-dev`;
- worktrees em `~/prometeu/worktrees[-dev]`;
- configuração em `.prometeu/settings.toml`;
- variáveis públicas com prefixo `PROMETEU_`;
- branches criadas com prefixo `prometeu/`;
- releases publicadas em `gbrancaglione/prometeu-releases`.

O aplicativo não consulta nem modifica dados do Prometheus automaticamente.
A migração será um caso de uso posterior, explícito e idempotente, que cria
backup, preserva a origem e trata worktrees com operações Git em vez de mover
pastas diretamente.

O Prometeu deixa de gravar a projeção de rollback definida no ADR 0002. Não há
versão anterior do Prometeu que dependa dela. O leitor conserva suporte aos
tokens históricos `prometheusV1Mirror` e `type: "prometheus"` para permitir
uma importação futura sem reescrever transcripts. Esta decisão substitui
somente a política temporária de espelho do ADR 0002; o protocolo V1 permanece.

## Consequências

Positivas:

- os dois produtos podem ser instalados e executados lado a lado;
- o desenvolvimento do Prometeu não arrisca o estado existente;
- uma falha na migração futura não apaga a origem;
- nomes novos não carregam contratos públicos acidentalmente.

Negativas:

- dados existentes não aparecem antes da importação;
- configurações `.prometheus` de outros repositórios precisam ser recriadas
  ou importadas conscientemente;
- integrações externas, assinatura e infraestrutura de release precisam de
  credenciais novas;
- o leitor legado ainda contém dois identificadores com o nome anterior.

O cadastro OAuth do Linear é uma exceção temporária: o primeiro ciclo de
desenvolvimento reutiliza seu client id anterior para não desativar a feature.
Antes da primeira release pública, ele deve ser substituído por um cadastro do
Prometeu; até lá, a tela de consentimento do Linear pode mostrar a marca antiga.

Atualização de implementação em 2026-09-04: a exceção foi encerrada. O
Prometeu passou a usar seu próprio cadastro OAuth antes da primeira release
pública, preservando o fluxo Authorization Code com PKCE e o escopo de leitura.

Atualização de implementação em 2026-09-04: a importação posterior foi
implementada pelo [ADR 0006](0006-explicit-prometheus-import.md), mantendo a
origem independente e adotando os worktrees antigos sem movê-los.

## Evidência

- testes de `paths.rs` cobrem as raízes novas;
- testes de `scripts.rs` cobrem arquivo e variáveis novas;
- testes de `branch.ts` cobrem o prefixo novo;
- testes de conversa mantêm fixtures dos tokens históricos;
- configuração Tauri define produto, binário e bundle id independentes.
