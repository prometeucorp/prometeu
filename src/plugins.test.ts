import { describe, expect, it } from "vitest";
import { t } from "./i18n";
import { flatLabel, label, SKILL_WORDS } from "./plugins";

/// Distinguish inherited plugin selection from an explicitly empty selection; their launcher labels must remain different.
describe("o rótulo do seletor", () => {
  it("nunca escolher e escolher nenhum são frases diferentes", () => {
    expect(flatLabel(null)).not.toBe(flatLabel([]));
    expect(flatLabel(null)).toBeTruthy();
    expect(flatLabel([])).toBeTruthy();
  });

  it("um escolhido aparece pelo nome, e vários pela conta", () => {
    expect(flatLabel(["caveman"])).toBe("caveman");
    expect(flatLabel(["caveman", "ponytail"])).toContain("2");
  });
});

/// The skills axis shares the plugin label function; it must speak about skills and strip the
/// `skill-` prefix the pipeline adds.
describe("o rótulo por eixo", () => {
  it("skills usam seu próprio texto e despem o prefixo", () => {
    expect(label(null, SKILL_WORDS)).not.toBe(label(null));
    expect(label({ base: "none", add: [], remove: [] }, SKILL_WORDS)).toBe(t("skill.zero"));
    expect(label({ base: "none", add: ["skill-caveman"], remove: [] }, SKILL_WORDS)).toBe("caveman");
    expect(label({ base: "inherit", add: ["skill-caveman"], remove: [] }, SKILL_WORDS)).toBe("caveman");
    expect(label({ base: "none", add: ["skill-a", "skill-b"], remove: [] }, SKILL_WORDS)).toBe(t("skill.count", { n: "2" }));
    expect(label({ base: "inherit", add: ["skill-a"], remove: ["skill-b"] }, SKILL_WORDS)).toBe("+1 −1");
  });

  it("plugins mantêm o texto de plugins", () => {
    expect(label({ base: "none", add: [], remove: [] })).toBe(t("plugin.zero"));
    expect(label({ base: "none", add: ["caveman"], remove: [] })).toBe("caveman");
    expect(label({ base: "none", add: ["caveman", "ponytail"], remove: [] })).toBe(t("plugin.count", { n: "2" }));
  });
});
