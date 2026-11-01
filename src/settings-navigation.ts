export type SettingsPage = "geral" | "agentes" | "recursos" | "acoes" | "trabalho" | "analytics";
export type ResourceFilter = "all" | "plugins" | "mcp" | "skills";

/** Old bookmarks keep reaching the same preferences after grouping the navigation. */
export function settingsDestination(saved: string | null): { page: SettingsPage; section?: string; filter?: ResourceFilter } {
  switch (saved) {
    case "contas": return { page: "agentes", section: "accounts" };
    case "padroes": return { page: "agentes", section: "defaults" };
    case "notifications": return { page: "geral", section: "notifications" };
    case "app": return { page: "geral", section: "app" };
    case "ferramentas": return { page: "recursos", filter: "mcp" };
    case "plugins": return { page: "recursos", filter: "plugins" };
    case "skills": return { page: "recursos", filter: "skills" };
    case "projects": return { page: "trabalho", section: "projects" };
    case "time": return { page: "trabalho", section: "team" };
    case "integracoes": return { page: "trabalho", section: "integrations" };
    case "agentes": case "recursos": case "acoes": case "trabalho": case "analytics": return { page: saved };
    default: return { page: "geral" };
  }
}

/** Settings and resource searches ignore case, accents and surrounding whitespace. */
export function matchesSettings(query: string, text: string): boolean {
  const fold = (value: string) => value.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLocaleLowerCase();
  return fold(query.trim()).split(/\s+/).every(word => fold(text).includes(word));
}
