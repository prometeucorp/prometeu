# Registro de decisões arquiteturais

Cada ADR registra uma decisão significativa, seu contexto e suas consequências.
Use um arquivo novo para mudar uma decisão aceita; não reescreva o passado.

## Estados

- **Proposto:** em discussão; não é regra vigente.
- **Aceito:** orientação atual do projeto.
- **Substituído:** outro ADR tomou seu lugar; mantenha link nos dois sentidos.
- **Rejeitado:** considerado e não adotado.

## Template

```md
# ADR NNNN — título

Data: AAAA-MM-DD
Status: Proposto

## Contexto

## Opções consideradas

## Decisão

## Consequências

## Evidência
```

## Índice

| ADR | Estado | Assunto |
| --- | --- | --- |
| [0001](0001-repository-knowledge.md) | Aceito | conhecimento do repositório como fonte de verdade |
| [0002](0002-canonical-conversation-protocol.md) | Aceito; espelho substituído | protocolo canônico de conversa |
| [0003](0003-agent-capabilities.md) | Aceito | features dirigidas por capacidades |
| [0004](0004-prometeu-independent-identity.md) | Aceito | identidade independente do Prometeu |
| [0005](0005-portable-plugin-marketplace.md) | Aceito | marketplace portátil para Claude e Codex |
