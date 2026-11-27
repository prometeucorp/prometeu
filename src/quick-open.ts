import { t } from "./i18n";
import { invoke } from "./ipc";
import type { PathEntry } from "./paths";
import { searchablePicker, type SearchPickerItem } from "./ui";

export type QuickOpen = {
  /// Close the picker and drop every response that is still in flight.
  close: () => void;
};

/// Command-P palette, and Command-F in the project view: the backend's indexed fuzzy path search
/// ranks each query, so the picker shows its results as given. The backend leaves directories out
/// before its row limit because opening one has no viewer. `open` receives only a path that `id`
/// returned. `closed` runs once, for each way that the picker closes.
export function openQuickOpen(
  anchor: HTMLElement,
  id: string,
  recent: string[],
  hooks: { open: (path: string) => void; closed?: () => void },
): QuickOpen {
  // A slower response for an earlier query, or for a closed picker, must not replace newer results.
  let asked = 0;
  let alive = true;
  const rows = (list: PathEntry[]): SearchPickerItem[] =>
    list.map((entry) => {
      const cut = entry.path.lastIndexOf("/");
      return { key: entry.path, label: entry.name, detail: cut < 0 ? undefined : entry.path.slice(0, cut) };
    });
  const picker = searchablePicker(anchor, { track: "files.picker", area: "files",
    label: t("quickOpen.label"),
    searchPlaceholder: t("quickOpen.search"),
    empty: t("quickOpen.empty"),
    items: [],
    // The picker closes before it selects. Its rows always answer this root.
    select: hooks.open,
    search: (query) => void load(query),
    closed: () => { alive = false; hooks.closed?.(); },
  });
  async function load(query: string) {
    const mine = ++asked;
    let list: PathEntry[] = [];
    try {
      list = await invoke("find_paths", { id, query: query.trim(), recent, files: true });
    } catch {
      list = [];
    }
    if (alive && mine === asked) picker.update(rows(list));
  }
  void load("");
  return {
    close: () => {
      if (!alive) return;
      alive = false;
      picker.close();
    },
  };
}

type FindKey = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey" | "defaultPrevented">;

/// Where the person is when Command-F (Control-F outside macOS) arrives.
export type FindPlace = {
  mac: boolean;
  /// A local project view is on screen, with no workspace or terminal session.
  project: boolean;
  /// A dialog or a panel above the view owns the keyboard.
  blocked: boolean;
  /// The focused control has its own use for the key: a text field, the file viewer or a terminal.
  focusOwnsKey: boolean;
};

/// Surfaces above the view that own the keyboard: native dialogs, the feedback panel and the veil,
/// which holds the workspace launcher and other overlays with `role="dialog"` on a `div`.
export const FIND_BLOCKERS = "dialog[open], .ui-feedback-panel:not([hidden]), #veil:not([hidden])";

type FindFocus = { isContentEditable: boolean; matches(selector: string): boolean; closest(selector: string): unknown };
type FindDocument = { activeElement: unknown; body: unknown; querySelector(selector: string): unknown };

/// Read the place of Command-F from the document. While a blocker is visible, the key stays with it,
/// also when a button in it has focus.
export function findPlace(doc: FindDocument, mac: boolean, project: boolean): FindPlace {
  const active = doc.activeElement as FindFocus | null;
  const focused = active && active !== doc.body && typeof active.matches === "function" ? active : null;
  return {
    mac,
    project,
    blocked: !!doc.querySelector(FIND_BLOCKERS),
    focusOwnsKey: !!focused && (focused.isContentEditable || focused.matches("input, textarea, select") || !!focused.closest("#viewer, .xterm")),
  };
}

/// Return whether Command-F opens the file search of the project. Other views, and focused
/// controls that handle the key themselves, keep it.
export function takesProjectFind(e: FindKey, place: FindPlace): boolean {
  if (e.defaultPrevented || e.key.toLowerCase() !== "f" || e.shiftKey || e.altKey) return false;
  const primary = place.mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  return primary && place.project && !place.blocked && !place.focusOwnsKey;
}
