import { beforeEach, describe, expect, it, vi, type Mock } from "vitest";

vi.mock("./ipc", () => ({ invoke: vi.fn(() => Promise.resolve()) }));

import { invoke } from "./ipc";
// `vi.mocked(invoke)` makes the checker relate the whole IPC registry to the mock type, which costs minutes.
const invokeMock = invoke as unknown as Mock;
import { trackClicks, trackRequestResponse, unidentifiedControls, viewScreen, type ClickEvent } from "./analytics";

beforeEach(() => invokeMock.mockClear());

type Spec = { disabled?: boolean; ariaDisabled?: boolean; tag?: string; role?: string; type?: string; text?: string; t?: string; tTitle?: string; track?: string; trackArea?: string; trackDoubleClick?: string; trackContextMenu?: string };
type TestElement = HTMLElement & { spec: Spec };

/** Selector behavior is exercised in a real browser by scripts/check-analytics.mjs. */
function node(spec: Spec, parent: TestElement | null = null): TestElement {
  const element = {
    spec, tagName: (spec.tag ?? "div").toUpperCase(), parentElement: parent,
    dataset: { t: spec.t, tTitle: spec.tTitle, track: spec.track, trackArea: spec.trackArea,
      trackDoubleClick: spec.trackDoubleClick, trackContextMenu: spec.trackContextMenu }, textContent: spec.text ?? "",
    getAttribute: (name: string) => name === "role" ? spec.role ?? null : null,
    matches(selector: string) {
      return selector.split(/,\s*/).some(part => {
        if (part === ":disabled") return !!spec.disabled;
        if (part === "[aria-disabled=true]") return !!spec.ariaDisabled;
        if (part === "[data-track-area]") return spec.trackArea !== undefined;
        if (part === "[data-track]") return spec.track !== undefined;
        if (part === "[data-track-double-click]") return spec.trackDoubleClick !== undefined;
        if (part === "[data-track-context-menu]") return spec.trackContextMenu !== undefined;
        const role = /^\[role=(\w+)\]$/.exec(part);
        if (role) return spec.role === role[1];
        if (part.startsWith("input[type=")) return spec.tag === "input" && part.includes(`"${spec.type}"`);
        if (part === "input:not([type=hidden])") return spec.tag === "input" && spec.type !== "hidden";
        return spec.tag === part;
      });
    },
    closest(selector: string) {
      for (let at: TestElement | null = element as unknown as TestElement; at; at = at.parentElement as TestElement | null) {
        if (at.matches(selector)) return at;
      }
      return null;
    },
  };
  return element as unknown as TestElement;
}

function page(controls: TestElement[] = []) {
  const listeners: Record<string, (event: { target: TestElement }) => void> = {};
  const captured: ClickEvent[] = [];
  const root = {
    querySelectorAll: () => controls,
    addEventListener: (name: string, run: (event: { target: TestElement }) => void) => { listeners[name] = run; },
    dispatchEvent: vi.fn(),
  } as unknown as Document;
  trackClicks(root, event => captured.push(event));
  return { root, captured, click: (target: TestElement, kind = "click") => listeners[kind]({ target }) };
}

describe("authored click capture", () => {
  it("captures static metadata without visible text or user IDs", () => {
    const area = node({ trackArea: "workspace.tabs" });
    const control = node({ tag: "button", track: "tab.conversation", text: "private-project" }, area);
    const { click, captured } = page();
    click(node({ tag: "span", text: "private-project" }, control));
    expect(captured).toEqual([{ name: "ui_clicked", control: "button", target: "tab.conversation", area: "workspace.tabs" }]);
  });

  it("accepts explicitly authored translation keys without reverse lookup", () => {
    const area = node({ trackArea: "navigation" });
    const { click, captured } = page();
    click(node({ tag: "button", tTitle: "nav.back", text: "Back" }, area));
    click(node({ tag: "button", t: "nav.forward", text: "Avançar" }, area));
    expect(captured.map(event => event.target)).toEqual(["nav.back", "nav.forward"]);
  });

  it("exposes missing target and area before a control is clicked", () => {
    const area = node({ trackArea: "settings" });
    const noTarget = node({ tag: "button", text: "Save" }, area);
    const noArea = node({ tag: "button", track: "settings.save" });
    const neither = node({ tag: "input" });
    const alternate = node({ trackDoubleClick: "workspace.rename" }, area);
    const invalidAlternate = node({ trackContextMenu: "/private/path" }, area);
    const { root } = page([noTarget, noArea, neither, alternate, invalidAlternate]);
    expect(unidentifiedControls(root).map(problem => problem.missing)).toEqual([["target"], ["area"], ["target", "area"], ["target"]]);
  });

  it("checks missing metadata when disabled controls become eligible", () => {
    const disabled = node({ tag: "button", disabled: true });
    const ariaDisabled = node({ role: "menuitem", ariaDisabled: true });
    const { root, click, captured } = page([disabled, ariaDisabled]);
    expect(unidentifiedControls(root)).toEqual([]);
    click(disabled); click(ariaDisabled);
    expect(captured).toEqual([]);
    expect(root.dispatchEvent).not.toHaveBeenCalled();
    disabled.spec.disabled = false; ariaDisabled.spec.ariaDisabled = false;
    expect(unidentifiedControls(root).map(problem => problem.missing)).toEqual([["target", "area"], ["target", "area"]]);
  });

  it("reports invalid or missing identities locally and sends no null or catch-all", () => {
    vi.stubGlobal("CustomEvent", class { constructor(public type: string, public options: unknown) {} });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const area = node({ trackArea: "settings" });
    const { click, captured, root } = page();
    const missing = node({ tag: "button", text: "Save" }, area);
    click(missing);
    click(node({ tag: "button", track: "/Users/private/repository" }, area));
    click(node({ tag: "button", track: "settings.save" }));
    click(node({ tag: "button", track: "settings.save", trackDoubleClick: "/private/path" }, area), "dblclick");
    expect(captured).toEqual([]);
    expect(missing.dataset.trackMissing).toBe("target");
    expect(root.dispatchEvent).toHaveBeenCalledTimes(4);
    warn.mockRestore(); vi.unstubAllGlobals();
  });

  it("classifies native checkboxes and menu items and opts into alternate clicks", () => {
    const area = node({ trackArea: "git.files" });
    const { click, captured } = page();
    click(node({ tag: "input", type: "checkbox", track: "git.reviewed" }, area));
    click(node({ tag: "button", role: "menuitem", track: "git.stage" }, area));
    const row = node({ trackDoubleClick: "git.file.open", trackContextMenu: "git.file.menu" }, area);
    click(row, "dblclick"); click(row, "contextmenu");
    expect(captured.map(event => [event.control, event.target])).toEqual([
      ["checkbox", "git.reviewed"], ["menuitem", "git.stage"], ["button", "git.file.open"], ["button", "git.file.menu"],
    ]);
  });

  it("ignores content outside controls", () => {
    const { click, captured } = page();
    click(node({ text: "agent output" }));
    expect(captured).toEqual([]);
  });
});

describe("screen views", () => {
  it("counts changes, not repeats", () => {
    viewScreen("desk"); viewScreen("desk"); viewScreen("workspace"); viewScreen("workspace"); viewScreen("desk");
    const screens = invokeMock.mock.calls.map(([, args]) => (args as { event: { screen: string } }).event.screen);
    expect(screens).toEqual(["desk", "workspace", "desk"]);
  });
});

describe("accepted request responses", () => {
  it.each([
    ["allow", { name: "approval_submitted", approved: true }],
    ["deny", { name: "approval_submitted", approved: false }],
    ["answer", { name: "request_answered" }],
  ] as const)("classifies %s using only the closed outcome", (outcome, event) => {
    trackRequestResponse(outcome);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("analytics_track", { event });
  });
});
