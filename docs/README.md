# Documentação do Prometeu

Este diretório é a fonte de verdade técnica do projeto. `AGENTS.md` funciona
como índice curto para agentes e `README.md` apresenta o produto.

## Arquitetura

- [`../ARCHITECTURE.md`](../ARCHITECTURE.md): mapa geral do sistema.
- [`architecture/conversation-flow.md`](architecture/conversation-flow.md):
  caminho de uma conversa e ownership do estado.
- [`architecture/dependency-rules.md`](architecture/dependency-rules.md):
  regras entre apresentação, aplicação, domínio e adapters.

## Contratos

- [`contracts/conversation-events-v1.md`](contracts/conversation-events-v1.md):
  protocolo canônico pertencente ao Prometeu.
- [`contracts/agent-runtime.md`](contracts/agent-runtime.md): descoberta,
  capacidades e port de execução dos agentes.
- [`contracts/plugin-marketplace.md`](contracts/plugin-marketplace.md): hub,
  pacote portátil e adaptação por provider.
- [`contracts/ipc.md`](contracts/ipc.md): fronteira TypeScript/Rust.
- [`contracts/persistence.md`](contracts/persistence.md): board e transcripts.
- [`contracts/relay-v3.md`](contracts/relay-v3.md): protocolo de colaboração.

## Decisões

- [`decisions/README.md`](decisions/README.md): índice e ciclo de vida dos ADRs.
- [`decisions/0001-repository-knowledge.md`](decisions/0001-repository-knowledge.md):
  documentação versionada como fonte de verdade.
- [`decisions/0002-canonical-conversation-protocol.md`](decisions/0002-canonical-conversation-protocol.md):
  normalização dos protocolos de agentes — aceita.
- [`decisions/0003-agent-capabilities.md`](decisions/0003-agent-capabilities.md):
  disponibilidade de features por capacidades — aceita.
- [`decisions/0004-prometeu-independent-identity.md`](decisions/0004-prometeu-independent-identity.md):
  identidade, persistência e release independentes do produto anterior — aceita.
- [`decisions/0005-portable-plugin-marketplace.md`](decisions/0005-portable-plugin-marketplace.md):
  um marketplace de plugins para Claude e Codex — aceita.

## Qualidade e operação

- [`quality/provider-matrix.md`](quality/provider-matrix.md): suporte por agente
  e evidência esperada.
- [`operations/development.md`](operations/development.md): ambiente e testes.
- [`operations/release.md`](operations/release.md): CI, versionamento e release.

## Regra de atualização

Uma mudança deve atualizar o documento que responde à pergunta afetada:

- comportamento visível: `README.md` ou documentação da feature;
- responsabilidade ou fluxo: arquitetura;
- formato entre camadas: contrato;
- escolha e trade-offs: ADR;
- suporte por agente: matriz de providers;
- build, teste ou publicação: operação.

Documentação proposta deve declarar seu status. Ela não descreve o sistema
atual até que a implementação correspondente seja aceita.

## Referências da abordagem

- [OpenAI — Harness engineering](https://openai.com/index/harness-engineering/):
  `AGENTS.md` curto como mapa e `docs/` versionado como fonte de verdade.
- [AGENTS.md](https://agents.md/): formato comum de instruções para agentes.
- [Claude Code — project memory](https://code.claude.com/docs/en/memory):
  instruções concisas e escopadas no repositório.
- [C4 Model](https://c4model.com/diagrams): contexto e containers para o mapa
  arquitetural.
- [AWS — Architecture Decision Records](https://docs.aws.amazon.com/prescriptive-guidance/latest/architectural-decision-records/adr-process.html):
  contexto, decisão, consequências e ciclo de vida de ADRs.
