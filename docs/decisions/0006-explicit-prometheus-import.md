# ADR 0006 — Importação explícita do Prometheus

Data: 2026-09-04
Status: Aceito

## Contexto

O Prometeu nasceu com identidade e raízes independentes para que pudesse ser
validado sem arriscar a instalação anterior. As poucas pessoas que já usavam o
Prometheus, porém, têm quadro, conversas, plugins e worktrees que não devem ser
abandonados na troca.

Copiar toda a raiz não basta: transcripts do Claude dependem do caminho do
worktree, plugins gerenciados guardam caminhos absolutos, credenciais pertencem
à identidade externa que as emitiu e worktrees podem ocupar dezenas de
gigabytes. Mover tudo também tiraria a instalação antiga da condição de backup.

## Opções consideradas

1. Fazer o Prometeu consultar `~/.prometheus` automaticamente em toda abertura.
2. Copiar a raiz inteira e mover os worktrees para o namespace novo.
3. Oferecer uma importação única, explícita, somente para um destino vazio.

## Decisão

O Prometeu oferece temporariamente **Configurações → Aplicativo → Migrar do
Prometheus** quando encontra `~/.prometheus/board.json`.

A operação:

- exige que o quadro do Prometeu ainda não tenha projetos nem workspaces;
- mostra uma prévia e exige a confirmação de que o Prometheus está fechado;
- cria snapshot e manifesto privados sob `~/.prometeu/imports/`;
- desserializa e normaliza o board pelo modelo atual, desligando processos e
  removendo as marcas de compartilhamento;
- copia todos os logs Codex para `~/.prometeu/chats/` sem sobrescrever conflito;
- conserva os transcripts Claude em `~/.claude`, pois os caminhos de trabalho
  não mudam;
- copia plugins gerenciados para a raiz nova, reescreve somente suas origens no
  cadastro e recusa colisões;
- cria `.prometeu/settings.toml` somente quando há um
  `.prometheus/settings.toml` e o destino não existe, trocando apenas o prefixo
  de variável pública `PROMETHEUS_` por `PROMETEU_`;
- não importa credenciais de Linear/time, cotas, caches, WebKit ou processos
  temporários;
- preserva branches e worktrees no caminho antigo. A limpeza de um workspace
  multi-repo aceita a raiz antiga somente quando ela é exatamente a que o
  Prometheus teria calculado.

O board é gravado por último. O manifesto leva o hash SHA-256 da origem e os
ids importados; uma repetição reconhece o resultado e não duplica dados. A
origem nunca é escrita nem apagada.

A interface pode ser removida quando a janela de transição acabar. O suporte
de leitura ao estado importado e a segurança dos caminhos antigos permanecem.

## Consequências

Positivas:

- a migração tem prévia, rollback e conflitos visíveis;
- gigabytes de worktrees não são duplicados;
- conversas e plugins continuam disponíveis;
- credenciais de uma identidade OAuth ou relay não vazam para outra.

Negativas:

- enquanto um worktree não for movido ou limpo, os dois aplicativos apontam
  para a mesma pasta e não devem operar aquele workspace ao mesmo tempo;
- um Prometeu que já tenha dados precisa de migração manual; esta feature não
  implementa merge de boards;
- o Linear precisa ser autorizado novamente;
- arquivos `.prometeu/settings.toml` criados em repositórios podem aparecer no
  `git status` quando o diretório não estiver ignorado.

## Evidência

- testes Rust cobrem prévia, cópia, normalização, idempotência, destino
  ocupado e conflito de transcript;
- o E2E cobre a prévia e a confirmação da interface;
- o contrato de persistência enumera dados incluídos e excluídos.
