import { catalogOf, descriptor, installed, isKnownModel, modelsOf, nativeEffort, onCatalogChange, refreshModels } from "./agents";
import { choiceKey, effortLadder, favoriteChoices, isFavorite, modelLabel, sameChoice, toggleFavorite, type ModelChoice } from "./model-choice";
import { current as locale, t, type Key } from "./i18n";
import { searchablePicker, type SearchPickerItem } from "./ui";
import * as menu from "./menu";
import type { ProviderId } from "./types";

export type ModelPickerOptions = {
  current: ModelChoice;
  only?: ProviderId;
  select: (choice: ModelChoice) => void;
  terminal?: () => void;
  closed?: () => void;
};

/** Shared domain composition; the design system owns search, focus and popup lifetime. */
export function openModelPicker(at: HTMLElement, options: ModelPickerOptions): void {
  let additional = false;
  const choices = new Map<string, ModelChoice>();
  const providers = () => installed().filter(p => !options.only || p.id === options.only);
  const row = (choice: ModelChoice, group: string, favorite = false): SearchPickerItem => {
    const model = modelsOf(choice.agent).find(item => item.id === choice.model);
    const label = modelLabel(choice.model, choice.agent);
    const key = `${favorite ? "favorite:" : "model:"}${choiceKey(choice)}`;
    choices.set(key, choice);
    return {
      key, label, group,
      detail: [descriptor(choice.agent).label, choice.model, model?.additional ? t("models.additionalBadge") : ""].filter(Boolean).join(" · "),
      checked: sameChoice(choice, options.current),
      secondary: choice.model ? {
        label: t(isFavorite(choice) ? "models.unfavorite" : "models.favorite", { model: `${label} · ${descriptor(choice.agent).label}` }),
        pressed: isFavorite(choice),
        run: () => { toggleFavorite(choice); draw(); },
      } : undefined,
    };
  };
  const items = (): SearchPickerItem[] => {
    choices.clear();
    const available = providers();
    const favorites = favoriteChoices().filter(choice => available.some(p => p.id === choice.agent) &&
      modelsOf(choice.agent).some(model => model.id === choice.model && (additional || !model.additional)));
    const result = favorites.map(choice => row(choice, t("models.favorites"), true));
    for (const provider of available) {
      result.push(row({ agent: provider.id, model: "" }, provider.label));
      for (const model of provider.models) {
        if (!additional && model.additional) continue;
        result.push(row({ agent: provider.id, model: model.id }, provider.label));
      }
    }
    if (options.current.model && (!descriptor(options.current.agent).installed || !isKnownModel(options.current.agent, options.current.model))) {
      result.unshift({ key: "historical", label: modelLabel(options.current.model, options.current.agent), group: t("models.current"),
        detail: `${descriptor(options.current.agent).label} · ${t("models.unavailable")}`, checked: true, disabled: true });
    }
    if (options.terminal) result.push({ key: "terminal", label: t("dock.new"), group: t("models.actions") });
    return result;
  };
  const status = () => providers().flatMap(provider => {
    const state = catalogOf(provider.id);
    const messages: string[] = [];
    if (state.status === "loading" || state.status === "idle") messages.push(t("models.loading"));
    if (state.status === "ready" && !provider.models.length) messages.push(t("models.loadedEmpty"));
    if (state.status === "error") {
      messages.push(t((state.error ?? "err.modelsCatalog.failed") as Key));
      if (state.fetchedAt !== null) messages.push(t("models.stale", { time: new Date(state.fetchedAt).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" }) }));
    }
    return messages.length ? [`${provider.label}: ${messages.join(" · ")}`] : [];
  }).join("\n");
  const refresh = (force = false) => { for (const provider of providers()) void refreshModels(provider.id, force); };
  const picker = searchablePicker(at, {
    label: t("models.choose"), searchPlaceholder: t("models.search"), empty: t("models.empty"), items: items(), status: status(),
    select: key => {
      if (key === "terminal") { options.terminal?.(); return; }
      const choice = choices.get(key);
      if (choice && descriptor(choice.agent).installed && (!choice.model || isKnownModel(choice.agent, choice.model))) options.select(choice);
    },
    refresh: { label: t("models.refresh"), run: () => refresh(true) },
    additional: { label: t("models.additional"), checked: additional, change: value => { additional = value; draw(); } },
    closed: () => { forget(); options.closed?.(); },
  });
  function draw() { picker.update(items(), status()); }
  const forget = onCatalogChange(draw);
  refresh();
}

export function openEffortPicker(at: HTMLElement, choice: ModelChoice, current: string, select: (effort: string) => void): void {
  const native = nativeEffort(choice.agent, current);
  const ladder = effortLadder(choice.model, choice.agent);
  const items: menu.Item[] = ladder.map(([id, label]) => ({ label, checked: id === native, run: () => {
    // A catalog can refresh while this short menu is open; never submit a withdrawn option.
    if (effortLadder(choice.model, choice.agent).some(([supported]) => supported === id)) select(id);
  } }));
  if (!ladder.some(([id]) => id === native)) items.unshift({ label: t("models.unavailableEffort", { effort: current }), checked: true, disabled: true }, "sep");
  const box = at.getBoundingClientRect();
  menu.openAt({ x: box.left, y: box.bottom + 4 }, items, undefined, undefined, true);
}
