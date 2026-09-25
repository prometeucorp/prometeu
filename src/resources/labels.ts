import { t, tn } from "../i18n";
import type { ResourceLabels } from "../components/resource-view";

export function resourceLabels(): ResourceLabels {
  return {
    hint: t("settings.resourceHint"), defaults: t("settings.resourceDefaults"), add: t("settings.resourceAdd"),
    search: t("settings.resourceSearch"), name: t("settings.resourceName"), type: t("settings.resourceType"),
    origin: t("settings.resourceOrigin"), empty: t("settings.resourceEmpty"), noResults: t("settings.resourceNoResults"),
    loading: t("settings.resourceLoading"), retry: t("settings.resourceRetry"),
    filters: { all: t("settings.resourceAll"), plugins: t("settings.plugins"), mcp: t("settings.resourceMcp"), skills: t("skill.title") },
    kinds: { plugins: t("settings.resourcePlugin"), mcp: t("settings.resourceMcp"), skills: t("settings.resourceSkill") },
    count: n => tn(n, "settings.resourceCount"), more: name => `${t("actions.more")} · ${name}`,
  };
}
