import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SearchPickerItem } from "./ui";

type Options = {
  select: (key: string) => void;
  search: (query: string) => void;
  closed?: () => void;
};
type Picker = { options: Options; rows: SearchPickerItem[][]; closed: boolean };

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), pickers: [] as Picker[] }));
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));
// The stand-in records what the shared picker would draw and closes as the menu does.
vi.mock("./ui", () => ({
  searchablePicker: (_anchor: unknown, options: Options) => {
    const picker: Picker = { options, rows: [], closed: false };
    mocks.pickers.push(picker);
    return {
      update: (rows: SearchPickerItem[]) => picker.rows.push(rows),
      close: () => {
        if (picker.closed) return;
        picker.closed = true;
        options.closed?.();
      },
    };
  },
}));

import { findPlace, openQuickOpen, takesProjectFind, type FindPlace } from "./quick-open";

/// A find_paths answer that the test releases when it wants, to put responses out of order.
function deferred() {
  let resolve!: (value: { name: string; path: string; dir: boolean }[]) => void;
  const promise = new Promise<{ name: string; path: string; dir: boolean }[]>((done) => (resolve = done));
  return { promise, resolve };
}

const file = (path: string) => ({ name: path.split("/").pop()!, path, dir: false });
const flush = () => new Promise((done) => setTimeout(done, 0));
const anchor = {} as HTMLElement;

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.pickers.length = 0;
});

describe("Command-F in the project view", () => {
  const key = (over: Partial<KeyboardEvent> = {}) =>
    ({ key: "f", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, defaultPrevented: false, ...over }) as KeyboardEvent;
  const place = (over: Partial<FindPlace> = {}): FindPlace => ({ mac: true, project: true, blocked: false, focusOwnsKey: false, ...over });

  it("takes Command-F on macOS and Control-F on other systems", () => {
    expect(takesProjectFind(key({ metaKey: true }), place())).toBe(true);
    expect(takesProjectFind(key({ metaKey: true, key: "F" }), place())).toBe(true);
    expect(takesProjectFind(key({ ctrlKey: true }), place({ mac: false }))).toBe(true);
    // Control-F on macOS moves the cursor in terminals and text fields.
    expect(takesProjectFind(key({ ctrlKey: true }), place())).toBe(false);
    expect(takesProjectFind(key({ metaKey: true }), place({ mac: false }))).toBe(false);
  });

  it("keeps other shortcuts and modified forms of the key", () => {
    expect(takesProjectFind(key({ metaKey: true, key: "p" }), place())).toBe(false);
    expect(takesProjectFind(key({ metaKey: true, shiftKey: true }), place())).toBe(false);
    expect(takesProjectFind(key({ metaKey: true, altKey: true }), place())).toBe(false);
    expect(takesProjectFind(key(), place())).toBe(false);
  });

  it("leaves the key to other views, dialogs and focused controls that use it", () => {
    expect(takesProjectFind(key({ metaKey: true }), place({ project: false }))).toBe(false);
    expect(takesProjectFind(key({ metaKey: true }), place({ blocked: true }))).toBe(false);
    expect(takesProjectFind(key({ metaKey: true }), place({ focusOwnsKey: true }))).toBe(false);
    expect(takesProjectFind(key({ metaKey: true, defaultPrevented: true }), place())).toBe(false);
  });
});

describe("Command-F with the workspace launcher open", () => {
  const command = { key: "f", metaKey: true, ctrlKey: false, shiftKey: false, altKey: false, defaultPrevented: false } as KeyboardEvent;
  /// A document stand-in: `visible` holds the ids of the shown surfaces, and `focus` is the focused control.
  function page(visible: string[], focus: { tag: string; inside?: string } | null) {
    const body = {};
    const element = focus && {
      isContentEditable: false,
      matches: (selector: string) => selector.split(",").map((part) => part.trim()).includes(focus.tag),
      closest: (selector: string) => (focus.inside && selector.includes(focus.inside) ? {} : null),
    };
    return {
      body,
      activeElement: element ?? body,
      // The launcher is a `div` with `role="dialog"` in the veil, not a native dialog.
      querySelector: (selector: string) => (visible.includes("veil") && selector.includes("#veil:not([hidden])") ? {} : null),
    };
  }

  it("leaves the key to the launcher when one of its buttons has focus", () => {
    const place = findPlace(page(["veil"], { tag: "button", inside: "#veil" }), true, true);
    expect(place.blocked).toBe(true);
    expect(takesProjectFind(command, place)).toBe(false);
  });

  it("leaves the key to the launcher when nothing has focus", () => {
    expect(takesProjectFind(command, findPlace(page(["veil"], null), true, true))).toBe(false);
  });

  it("searches the project files again after the launcher closes", () => {
    expect(takesProjectFind(command, findPlace(page([], { tag: "button" }), true, true))).toBe(true);
    expect(takesProjectFind(command, findPlace(page([], null), true, true))).toBe(true);
  });

  it("keeps the key for a focused text field and the file viewer", () => {
    expect(takesProjectFind(command, findPlace(page([], { tag: "textarea" }), true, true))).toBe(false);
    expect(takesProjectFind(command, findPlace(page([], { tag: "div", inside: "#viewer" }), true, true))).toBe(false);
  });
});

describe("file search of the selected root", () => {
  it("asks for files of the given project and shows each name with its folder", async () => {
    mocks.invoke.mockResolvedValue([file("src/app/main.ts"), file("README.md")]);
    openQuickOpen(anchor, "project-a", ["src/app/main.ts"], { open: vi.fn() });
    await flush();
    expect(mocks.invoke).toHaveBeenCalledWith("find_paths", { id: "project-a", query: "", recent: ["src/app/main.ts"], files: true });
    expect(mocks.pickers[0].rows[mocks.pickers[0].rows.length - 1]).toEqual([
      { key: "src/app/main.ts", label: "main.ts", detail: "src/app" },
      { key: "README.md", label: "README.md", detail: undefined },
    ]);
  });

  it("opens the selected relative path", async () => {
    mocks.invoke.mockResolvedValue([file("src/main.ts")]);
    const open = vi.fn();
    openQuickOpen(anchor, "project-a", [], { open });
    await flush();
    mocks.pickers[0].options.select("src/main.ts");
    expect(open).toHaveBeenCalledWith("src/main.ts");
  });

  it("keeps the newest query when answers arrive out of order", async () => {
    const first = deferred();
    const slow = deferred();
    const fast = deferred();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(slow.promise).mockReturnValueOnce(fast.promise);
    openQuickOpen(anchor, "project-a", [], { open: vi.fn() });
    const picker = mocks.pickers[0];
    picker.options.search("ma");
    picker.options.search(" main ");
    expect(mocks.invoke).toHaveBeenLastCalledWith("find_paths", { id: "project-a", query: "main", recent: [], files: true });
    fast.resolve([file("src/main.ts")]);
    await flush();
    slow.resolve([file("src/map.ts")]);
    first.resolve([file("old.ts")]);
    await flush();
    expect(picker.rows).toEqual([[{ key: "src/main.ts", label: "main.ts", detail: "src" }]]);
  });

  it("does not show a late answer of a project after a change to another project with the same file name", async () => {
    const late = deferred();
    mocks.invoke.mockReturnValueOnce(late.promise).mockResolvedValueOnce([file("src/index.ts")]);
    const closed = vi.fn();
    const openA = vi.fn();
    const openB = vi.fn();
    const a = openQuickOpen(anchor, "project-a", [], { open: openA, closed });
    // Leaving project A closes its search before project B opens its own.
    a.close();
    expect(closed).toHaveBeenCalledTimes(1);
    openQuickOpen(anchor, "project-b", [], { open: openB });
    await flush();
    late.resolve([file("src/index.ts"), file("lib/only-a.ts")]);
    await flush();
    const [pickerA, pickerB] = mocks.pickers;
    expect(pickerA.closed).toBe(true);
    expect(pickerA.rows).toEqual([]);
    expect(pickerB.rows).toEqual([[{ key: "src/index.ts", label: "index.ts", detail: "src" }]]);
    expect(mocks.invoke.mock.calls.map(([, args]) => args.id)).toEqual(["project-a", "project-b"]);
    pickerB.options.select("src/index.ts");
    expect(openB).toHaveBeenCalledWith("src/index.ts");
    expect(openA).not.toHaveBeenCalled();
  });

  it("drops answers that arrive after the person closes the search", async () => {
    const late = deferred();
    mocks.invoke.mockReturnValueOnce(late.promise);
    const closed = vi.fn();
    openQuickOpen(anchor, "project-a", [], { open: vi.fn(), closed });
    const picker = mocks.pickers[0];
    // Escape or a click outside closes the picker through the shared menu.
    picker.closed = true;
    picker.options.closed?.();
    late.resolve([file("src/main.ts")]);
    await flush();
    expect(picker.rows).toEqual([]);
    expect(closed).toHaveBeenCalledTimes(1);
  });
});
