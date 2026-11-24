import { EN } from "./i18n.en";
import { PT } from "./i18n.pt";
import { keys, mac, machine } from "./platform";

/// Store one local interface-language preference; optional accounts do not synchronize it. Without a choice, follow the system language. Portuguese defines catalog keys and English must implement them. Translate interface text, preserving user and agent content.

/// Display each language's native name in the language picker.
export const LANGS = [
  ["pt-BR", "Português (Brasil)"],
  ["en", "English"],
] as const;

export type Lang = (typeof LANGS)[number][0];
export type Key = keyof typeof PT;

/// Restrict tn to keys with both singular and plural forms.
type Stem<K> = K extends `${infer S}.one` ? S : never;
export type PluralKey = Stem<Key>;

export type Params = Record<string, string | number>;

const STORE = "prometeu:idioma";

/// Backend messages use an i18n: prefix with a code and interpolation arguments; see src-tauri/src/i18n.rs.
const BACK = "i18n:";

const DICTS: Record<Lang, Record<string, string>> = { "pt-BR": PT, en: EN };

/// Try preferred system languages by exact tag, then language root; fall back to English. Export this pure selection for tests.
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

/// Node tests have no browser storage or navigator; default to English and allow use() to override it.
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

/// Share the active locale with Settings and Intl number/date formatters.
export const current = () => lang;

/// Change language without persistence or reload for tests; UI selection uses choose().
export function use(next: Lang) {
  lang = next;
  if (typeof document !== "undefined") document.documentElement.lang = next;
}

/// Persist the choice and reload to translate UI that is built only once. Null restores system language. Backend processes survive and conversations reattach.
export function choose(next: Lang | null) {
  if (next === chosen()) return;
  try {
    next ? localStorage.setItem(STORE, next) : localStorage.removeItem(STORE);
  } catch {
    // Reload still applies the language when storage is unavailable.
  }
  location.reload();
}

/// Distinguish an explicit language choice from following the system.
export function chosen(): Lang | null {
  const saved = stored();
  return saved && LANGS.some(([id]) => id === saved) ? (saved as Lang) : null;
}

/// Resolve the system language for the picker label.
export function fromSystem(): Lang {
  if (typeof navigator === "undefined") return "en";
  return match(navigator.languages?.length ? navigator.languages : [navigator.language]);
}

/// Outside macOS, `<key>.generic` replaces text naming macOS or its apps.
function lookup(key: string): string | undefined {
  return DICTS[lang][key] ?? (EN as Record<string, string>)[key];
}

const COMPUTER: Record<Lang, string> = { "pt-BR": "computador", en: "computer" };

/// Interpolate parameters; missing translations fall back to English, then the key itself. Outside macOS, the text is made platform-neutral.
export function t(key: Key, params?: Params): string {
  const text = (mac ? undefined : lookup(`${key}.generic`)) ?? lookup(key) ?? key;
  const raw = keys(machine(text, COMPUTER[lang]));
  if (!params) return raw;
  return raw.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  );
}

/// Portuguese and English use singular for one and plural for other counts.
export function tn(n: number, stem: PluralKey, params?: Params): string {
  return t(`${stem}.${n === 1 ? "one" : "other"}` as Key, { n, ...params });
}

/// Translate index.html nodes marked data-t and tooltips marked data-t-title without moving their markup into main.ts.
export function paint(root: ParentNode = document) {
  for (const el of root.querySelectorAll<HTMLElement>("[data-t]")) {
    el.textContent = t(el.dataset.t as Key);
  }
  for (const el of root.querySelectorAll<HTMLElement>("[data-t-title]")) {
    el.title = t(el.dataset.tTitle as Key);
  }
}

/// Translate structured backend i18n messages. Preserve unstructured plugin, library, and panic messages instead of hiding them.
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
