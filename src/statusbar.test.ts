import { beforeEach, describe, expect, it } from "vitest";
import { use } from "./i18n";
import { ago, awakeMode, bytes, groups, span, until, type Window } from "./statusbar";

beforeEach(() => use("en"));

it("preserves saved display modes and enables system-only inhibition only when opted in", () => {
  expect(awakeMode("off", true)).toBe("off");
  expect(awakeMode("on", false)).toBe("display");
  expect(awakeMode("agent", true)).toBe("display");
  expect(awakeMode("agent", false)).toBe("off");
  expect(awakeMode("agent-system", true)).toBe("system");
  expect(awakeMode("agent-system", false)).toBe("off");
});

describe("span", () => {
  it("uses two duration units: days and hours, or hours and minutes", () => {
    expect(span(3 * 3600 + 15 * 60)).toBe("3h 15m");
    expect(span(3 * 86400 + 4 * 3600 + 50 * 60)).toBe("3d 4h");
    expect(span(12 * 60)).toBe("12m");
  });

  it("omits a trailing zero unit for exact durations", () => {
    expect(span(2 * 3600)).toBe("2h");
    expect(span(5 * 86400)).toBe("5d");
  });

  it("clamps durations under a minute to zero minutes", () => {
    expect(span(30)).toBe("0m");
    expect(span(-90)).toBe("0m");
  });
});

describe("until", () => {
  it("counts down until the quota window resets", () => {
    expect(until(1000 + 3 * 3600, 1000)).toBe("3h");
  });

  // A stale usage response must not display a negative reset countdown.
  it("does not count backward after a window expires", () => {
    expect(until(900, 1000)).toBe("now");
  });
});

describe("ago", () => {
  it("omits elapsed time for freshly received readings", () => {
    expect(ago(980, 1000)).toBe("updated just now");
  });

  it("shows elapsed time after one minute", () => {
    expect(ago(1000 - 4 * 60, 1000)).toBe("updated 4m ago");
    expect(ago(1000 - 96 * 60, 1000)).toBe("updated 1h 36m ago");
  });
});

describe("bytes", () => {
  it("uses one decimal below ten and none above it", () => {
    expect(bytes(822 * 1024 * 1024)).toBe("822 MB");
    expect(bytes(1.25 * 1024 * 1024 * 1024)).toBe("1.3 GB");
    expect(bytes(4096)).toBe("4.0 KB");
  });

  it("handles processes that have not allocated memory yet", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(900)).toBe("900 B");
  });
});

describe("groups", () => {
  it("groups Codex windows by quota without reordering them", () => {
    const windows: Window[] = [
      { kind: "weekly", pct: 6, resets: 30, scope: "general" },
      { kind: "session", pct: 1, resets: 10, scope: "spark", label: "Spark" },
      { kind: "weekly", pct: 0, resets: 40, scope: "spark", label: "Spark" },
    ];
    expect(groups(windows)).toEqual([
      ["general", [windows[0]]],
      ["spark", [windows[1], windows[2]]],
    ]);
  });

  it("treats legacy persisted snapshots as the general quota", () => {
    const window: Window = { kind: "session", pct: 4, resets: 10 };
    expect(groups([window])).toEqual([["general", [window]]]);
  });
});
