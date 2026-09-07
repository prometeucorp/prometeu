import { ChatView, type Ctx, type Info } from "./chat";
import * as team from "./team";
import { $ } from "./util";

/// A conversa: o `claude` de verdade rodando atrás, e a tela desenhada a partir
/// do que ele escreve (`chat.ts`). Pergunta, plano e permissão chegam como
/// cards, e é aqui que se responde.

const view = new ChatView();
let attachVersion = 0;

export function init(onError: (m: string) => void, info: () => Info, comments: Pick<Ctx, "comment" | "thread"> = {}) {
  view.open($("chatwrap"), { say: onError, info, ...comments });
  team.setSink({
    live: (tab, bytes) => view.remoteWrite(tab, bytes),
    reset: (tab, bytes) => {
      if (tab === view.current()) view.attachRemote(tab, bytes);
    },
  });
}

/// Liga a tela numa conversa. `remote` é o id do workspace de um colega
/// quando a conversa é dele: aí as linhas vêm do relay, e não do back daqui.
export async function attach(id: string, remote?: string): Promise<boolean> {
  const version = ++attachVersion;
  if (remote) {
    const r = await team.attach(remote, id);
    if (!r || version !== attachVersion) return false;
    view.attachRemote(id, r.bytes);
  } else {
    // Trocar direto de um workspace remoto para um local não passa por
    // `workspace.leave`: o watcher e a espera antigos precisam cair aqui.
    team.detach();
    await view.attach(id);
    if (version !== attachVersion || view.current() !== id) return false;
  }
  if (version !== attachVersion) return false;
  view.focus();
  return true;
}

export function detach() {
  attachVersion++;
  team.detach();
  view.detach();
}

/// Aba que sumiu do quadro leva junto a fala que ficou pela metade nela.
export const forget = (alive: Set<string>) => view.forget(alive);

export const currentSession = () => view.current();
export const focus = () => view.focus();
export const refresh = () => view.refresh();
export const fileDropTarget = () => view.fileDropTarget();
export const quoteSelection = () => view.quoteSelection();
export const focusAnchor = (anchor: string) => view.focusAnchor(anchor);
