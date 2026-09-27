/// Review thresholds are versioned hypotheses, independent of the adapter's requested model.
/// A transport version bump alone must not silently approve a new calibration.
export const CALIBRATED_MODEL = "jev-1.13.0";
export type ReviewLanguage = "en" | "pt" | "other";
export type Thresholds = { kind: number; presence: number; resolver: number; ruleKind: number };
export const THRESHOLDS: Record<ReviewLanguage, Readonly<Thresholds>> = {
  en: { kind: 0.7, presence: 0.8, resolver: 0.7, ruleKind: 0.6 },
  pt: { kind: 0.75, presence: 0.85, resolver: 0.75, ruleKind: 0.65 },
  other: { kind: 0.8, presence: 0.9, resolver: 0.8, ruleKind: 0.7 },
};

const EN = new Set("the a an this that with without for from should when which please fix add rename implement investigate why is are to of in existing customers import button".split(" "));
const PT = new Set("o a os as um uma de do da dos das para por que como quando com sem nao deve deveria corrigir adicionar trocar implementar investigar porque clientes importacao botao".split(" "));

/// Small vocabulary, no dependency or network. Ambiguous/short/mixed text is conservative;
/// UI language and third-party issue text never lower a draft's thresholds.
export function languageOf(draft: string): ReviewLanguage {
  const tokens = draft.toLowerCase().normalize("NFD").replace(/\p{M}/gu, "").match(/\p{L}+/gu) ?? [];
  const words = new Set(tokens);
  const score = (dictionary: Set<string>, other: Set<string>) => [...words].filter(word => dictionary.has(word) && !other.has(word)).length;
  const en = score(EN, PT), pt = score(PT, EN);
  if (en >= 2 && en >= 2 * pt + 1) return "en";
  if (pt >= 2 && pt >= 2 * en + 1) return "pt";
  return "other";
}

export const thresholdsFor = (draft: string): Readonly<Thresholds> => THRESHOLDS[languageOf(draft)];
