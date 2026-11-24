import { beforeEach, describe, expect, it } from "vitest";
import { fromBack, match, t, tn, use } from "./i18n";

beforeEach(() => use("en"));

describe("match", () => {
  it("matches exact tags and regional variants", () => {
    expect(match(["pt-BR"])).toBe("pt-BR");
    expect(match(["en-US"])).toBe("en");
  });

  it("matches language roots case-insensitively", () => {
    expect(match(["pt-PT"])).toBe("pt-BR");
    expect(match(["PT-pt"])).toBe("pt-BR");
  });

  it("respects the order of system preferences", () => {
    expect(match(["fr-FR", "pt-BR", "en"])).toBe("pt-BR");
  });

  it("falls back to English when no language matches", () => {
    expect(match(["ja"])).toBe("en");
    expect(match([])).toBe("en");
  });
});

describe("t", () => {
  it("interpolates supplied arguments", () => {
    expect(t("rail.newIn", { project: "njord" })).toBe("New workspace in njord");
  });

  it("preserves placeholders with missing arguments", () => {
    expect(t("ws.copied", {})).toBe("{name} copied");
  });
});

describe("tn", () => {
  it("selects singular only for one", () => {
    expect(tn(1, "diff.files")).toBe("1 file");
    expect(tn(0, "diff.files")).toBe("0 files");
    expect(tn(3, "diff.files")).toBe("3 files");
  });
});


describe("fromBack", () => {
  it("decodes structured backend errors and interpolates their arguments", () => {
    expect(fromBack('i18n:{"code":"err.pty.gone"}')).toBe("terminal is not running");
    expect(fromBack('i18n:{"code":"err.session.notGit","args":{"path":"/tmp/x"}}')).toBe(
      "/tmp/x is not a git repository",
    );
  });

  it("preserves unstructured errors", () => {
    expect(fromBack("process.restart not allowed")).toBe("process.restart not allowed");
    expect(fromBack(new Error("boom"))).toBe("Error: boom");
  });

  it("preserves malformed error payloads", () => {
    expect(fromBack("i18n:{invalid json")).toBe("i18n:{invalid json");
  });
});
