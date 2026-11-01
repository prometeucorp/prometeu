import { describe, expect, it } from "vitest";
import { matchesSettings, settingsDestination } from "./settings-navigation";

describe("settings navigation compatibility", () => {
  it("keeps every former page reachable in its new group", () => {
    const old = {
      contas: { page: "agentes", section: "accounts" }, padroes: { page: "agentes", section: "defaults" },
      notifications: { page: "geral", section: "notifications" }, app: { page: "geral", section: "app" },
      ferramentas: { page: "recursos", filter: "mcp" }, plugins: { page: "recursos", filter: "plugins" },
      skills: { page: "recursos", filter: "skills" }, projects: { page: "trabalho", section: "projects" },
      time: { page: "trabalho", section: "team" }, integracoes: { page: "trabalho", section: "integrations" },
    };
    for (const [page, destination] of Object.entries(old)) expect(settingsDestination(page)).toEqual(destination);
    for (const page of ["geral", "agentes", "recursos", "acoes", "trabalho", "analytics"]) expect(settingsDestination(page)).toEqual({ page });
    expect(settingsDestination(null)).toEqual({ page: "geral" });
    expect(settingsDestination("unknown")).toEqual({ page: "geral" });
  });
  it("matches words across labels without case or accent sensitivity", () => {
    expect(matchesSettings("  NOTIFICACOES som  ", "Som · Notificações")).toBe(true);
    expect(matchesSettings("codex model", "Model defaults · Codex")).toBe(true);
    expect(matchesSettings("codex team", "Model defaults · Codex")).toBe(false);
    expect(matchesSettings("", "anything")).toBe(true);
  });
});
