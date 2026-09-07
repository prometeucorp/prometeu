import { EN } from "./i18n.en";
import { PT } from "./i18n.pt";

/// O idioma da tela. Uma escolha só, guardada neste Mac. A conta opcional do
/// Prometeu não sincroniza o idioma entre máquinas.
/// Sem escolha, vale o idioma do computador: quem abre o app pela primeira vez
/// não devia ter que ir nas configurações antes de entender a tela.
///
/// O catálogo é um mapa achatado de chave para frase, e o português é a fonte:
/// o inglês é `Record<Key, string>`, então uma chave nova sem tradução não
/// compila. Toda frase que a pessoa lê passa por aqui — o que o agente escreve
/// (a saída do terminal, a pergunta dele, o plano) não, porque isso é o
/// trabalho, e não a tela.

/// Os idiomas que o app fala. O rótulo é como o idioma se chama nele mesmo:
/// quem procura "English" numa lista não procura "Inglês".
export const LANGS = [
  ["pt-BR", "Português (Brasil)"],
  ["en", "English"],
] as const;

export type Lang = (typeof LANGS)[number][0];
export type Key = keyof typeof PT;

/// Só as chaves que têm par no plural. É o que faz `tn` não aceitar uma chave
/// que não tenha `.one` e `.other`.
type Stem<K> = K extends `${infer S}.one` ? S : never;
export type PluralKey = Stem<Key>;

export type Params = Record<string, string | number>;

const STORE = "prometeu:idioma";

/// O que o back manda no lugar de uma frase: `i18n:` e um JSON com o código e
/// os pedaços que entram nos buracos. Ver `src-tauri/src/i18n.rs`.
const BACK = "i18n:";

const DICTS: Record<Lang, Record<string, string>> = { "pt-BR": PT, en: EN };

/// O idioma do computador, na ordem em que ele prefere. Primeiro quem bate
/// inteiro (`pt-BR`), depois quem bate na raiz — `pt-PT` cai no português que
/// temos, `en-GB` no inglês. Nada bateu é inglês, que é o que mais gente lê.
///
/// Exportada porque é a única conta deste módulo, e errar nela é o app abrir
/// no idioma errado para quem nunca escolheu nenhum.
export function match(prefs: readonly string[]): Lang {
  for (const pref of prefs) {
    const tag = pref.toLowerCase();
    const exact = LANGS.find(([id]) => id.toLowerCase() === tag);
    if (exact) return exact[0];
    const near = LANGS.find(([id]) => id.split("-")[0] === tag.split("-")[0]);
    if (near) return near[0];
  }
  return "en";
}

/// Fora do navegador (vitest roda em node) não há `localStorage` nem
/// `navigator`: o módulo continua de pé, no inglês, e o teste escolhe o que
/// quiser com `use`.
function stored(): string | null {
  try {
    return localStorage.getItem(STORE);
  } catch {
    return null;
  }
}

function detect(): Lang {
  return chosen() ?? fromSystem();
}

let lang: Lang = detect();

/// Qual idioma está na tela. Serve ao seletor das configurações e a tudo que
/// formata número e data — o `Intl` quer a mesma tag.
export const current = () => lang;

/// Troca o idioma sem gravar nada e sem recarregar. É o que os testes usam;
/// quem clica no seletor usa `choose`.
export function use(next: Lang) {
  lang = next;
  if (typeof document !== "undefined") document.documentElement.lang = next;
}

/// A escolha da pessoa: grava e recarrega a janela. `null` é voltar a seguir o
/// computador — a escolha some, e quem trocar o idioma do Mac troca o do app.
///
/// Recarregar, e não redesenhar: metade da tela é montada uma vez só — a barra
/// de baixo, os títulos do HTML, os rótulos que já foram para dentro de um nó
/// que ninguém refaz — e sair chamando `draw` deixaria pedaços no idioma
/// anterior. Os processos vivem no back e voltam anexados; nenhuma conversa se
/// perde no caminho.
export function choose(next: Lang | null) {
  if (next === chosen()) return;
  try {
    next ? localStorage.setItem(STORE, next) : localStorage.removeItem(STORE);
  } catch {
    // Sem localStorage a escolha não gruda, mas a recarga ainda vale a pena.
  }
  location.reload();
}

/// O idioma que a pessoa escolheu, ou `null` se ela nunca escolheu — o seletor
/// precisa saber a diferença entre "português" e "português porque o Mac está
/// em português".
export function chosen(): Lang | null {
  const saved = stored();
  return saved && LANGS.some(([id]) => id === saved) ? (saved as Lang) : null;
}

/// Qual idioma o computador pediria, para o seletor dizer no que "do sistema" dá.
export function fromSystem(): Lang {
  if (typeof navigator === "undefined") return "en";
  return match(navigator.languages?.length ? navigator.languages : [navigator.language]);
}

/// A frase de uma chave, com `{buraco}` trocado pelo que veio em `params`.
/// Chave sem tradução cai no inglês, e depois na própria chave — que é feia na
/// tela, mas é o que diz onde está o buraco.
export function t(key: Key, params?: Params): string {
  const raw = DICTS[lang][key] ?? EN[key] ?? key;
  if (!params) return raw;
  return raw.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  );
}

/// Singular e plural, com o número já em `{n}`. Português e inglês partem a
/// contagem no mesmo lugar (um, e o resto), e é por isso que duas formas bastam.
export function tn(n: number, stem: PluralKey, params?: Params): string {
  return t(`${stem}.${n === 1 ? "one" : "other"}` as Key, { n, ...params });
}

/// Nome de etapa na tela. As etapas são dados — vêm gravadas no quadro e o
/// `stage` de cada workspace aponta para uma delas pelo nome —, então traduzir
/// o que está guardado quebraria quadro que já existe. O que se traduz é a
/// aparência das que o app criou sozinho na primeira vez; qualquer outro nome
/// sai como está.
export function stage(name: string): string {
  const key = `stage.${name}` as Key;
  return key in PT ? t(key) : name;
}

/// O que está escrito direto no `index.html`: `data-t` é o texto do nó e
/// `data-t-title` é o `title` do botão. Marcar no HTML mantém o arquivo
/// legível — a alternativa era mudar vinte rótulos para dentro do `main.ts` só
/// para conseguir traduzi-los.
export function paint(root: ParentNode = document) {
  for (const el of root.querySelectorAll<HTMLElement>("[data-t]")) {
    el.textContent = t(el.dataset.t as Key);
  }
  for (const el of root.querySelectorAll<HTMLElement>("[data-t-title]")) {
    el.title = t(el.dataset.tTitle as Key);
  }
}

/// Traduz o que veio do back. O Rust não escreve frase: escreve um código, e o
/// que atravessa a ponte é `i18n:{"code":…,"args":…}` — ver `src-tauri/src/
/// i18n.rs`. O que não estiver nesse formato volta como veio: erro de plugin,
/// pânico, mensagem de biblioteca. Melhor em inglês cru do que engolida.
export function fromBack(value: unknown): string {
  const text = typeof value === "string" ? value : String(value);
  if (!text.startsWith(BACK)) return text;
  try {
    const { code, args } = JSON.parse(text.slice(BACK.length)) as {
      code: string;
      args?: Params;
    };
    return t(code as Key, args);
  } catch {
    return text;
  }
}

use(lang);
