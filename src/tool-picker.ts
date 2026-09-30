import { invoke } from "./ipc";
import { t } from "./i18n";
import * as menu from "./menu";
import { toggleSelection, type EffectiveItem, type Provenance, type ProviderId, type Selection } from "./types";

/// One provenance-aware tool picker shared by the MCP, plugin and standalone-skill axes (ADR 0045).
/// It reads the resolved effective set from `workspace_tools`, labels each row with where it came
/// from, and writes the workspace layer as inherit-plus-deltas so the global and project layers keep
/// flowing through. A change applies at the next spawn; the running session keeps its born set, so the
/// picker only states that instead of restarting anything.

export type Axis = "mcp" | "plugins" | "skills";

/// One selectable row. `id` is the hub identity used by the selection (skills ride `skill-<id>`).
/// `section` groups rows under a disabled header; headers render only when more than one group exists.
/// `implied` is the provenance that header already states, so a grouped row does not repeat its badge.
export type Row = { id: string; label: string; hint?: string; section?: string; implied?: Provenance };

export type Pick = {
  workspace: string;
  agent?: ProviderId;
  axis: Axis;
  rows: Row[];
  /// The workspace layer as it stands; null inherits everything from the layers above.
  current: () => Selection | null;
  /// Persist the new workspace layer, or null to return the axis to inherit.
  set: (sel: Selection | null) => Promise<void> | void;
  at: () => menu.Where;
  /// Shown when the hub for this axis is empty.
  noneLabel: string;
  /// Opens the project-trust prompt; offered while the project declaration awaits a decision.
  trust?: () => void;
};

/// Provenance values that mean the item is currently injected.
const isOn = (p: Provenance | undefined) => p === "inherited" || p === "added" || p === "cli";

function badgeFor(p: Provenance | undefined): string | undefined {
  if (p === "inherited") return t("tools.prov.inherited");
  if (p === "cli") return t("tools.prov.cli");
  if (p === "removed") return t("tools.prov.removed");
  if (p === "pending") return t("tools.prov.pending");
  if (p === "rejected") return t("tools.prov.rejected");
  // An item the person added needs no badge; the checkmark already says it is on.
  return undefined;
}

/// `scroll` is the list offset to restore when a choice reopens the picker.
export async function open(p: Pick, scroll = 0) {
  let items0: EffectiveItem[] = [];
  let pending = false;
  try {
    const [tools, project] = await Promise.all([
      invoke("workspace_tools", { id: p.workspace, agent: p.agent }),
      invoke("project_tools", { id: p.workspace }),
    ]);
    items0 = tools[p.axis];
    pending = project.pending;
  } catch {
    // An older or unavailable backend resolves nothing; fall back to the workspace's own adds.
  }
  const provenance = new Map(items0.map((e) => [e.id, e.provenance]));
  const current = p.current();
  // Each choice persists and reopens the picker where the person left the list.
  let offset = scroll;
  const choose = (sel: Selection | null) => {
    void Promise.resolve(p.set(sel)).then(() => open(p, offset));
  };
  const items: menu.Item[] = [];
  items.push({ label: t("tools.appliesNext"), disabled: true }, "sep");
  if (!p.rows.length) items.push({ label: p.noneLabel, disabled: true });
  // Group headers appear only when the list spans more than one origin, so single-group menus stay flat.
  const grouped = new Set(p.rows.map((r) => r.section).filter((s) => s !== undefined)).size > 1;
  let lastSection: string | undefined;
  let listed = false;
  for (const row of p.rows) {
    if (grouped && row.section !== lastSection) {
      lastSection = row.section;
      if (row.section !== undefined) {
        if (listed) items.push("sep");
        items.push({ label: row.section, disabled: true });
      }
    }
    listed = true;
    const seen = provenance.get(row.id);
    // Hub rows trust the resolved provenance; a row the hub no longer has falls back to the layer's add.
    const on = seen !== undefined ? isOn(seen) : (current?.add.includes(row.id) ?? false);
    items.push({
      label: row.label,
      hint: row.hint,
      checked: on,
      // The header already names this provenance; repeating it on every row buries the exceptions.
      badge: grouped && seen === row.implied ? undefined : badgeFor(seen),
      run: () => choose(toggleSelection(current, row.id, !on)),
    });
  }
  if (p.rows.length) {
    // Explicitly start from nothing: unlike reset (which returns the axis to inherit), this writes
    // an empty `none` layer so the global, project and CLI contributions are all switched off.
    const empty = current?.base === "none" && !current.add.length;
    items.push("sep", {
      label: t("tools.selectNone"),
      checked: empty,
      run: () => choose({ base: "none", add: [], remove: [] }),
    });
    if (current) {
      items.push({
        label: t("tools.reset"),
        run: () => choose(null),
      });
    }
  }
  // A pending project declaration resolves but is not injected until the person approves its hash.
  if (p.trust && pending) {
    items.push("sep", {
      label: t("tools.trust.prompt"),
      run: () => p.trust!(),
    });
  }
  show(p.at(), items, scroll, (top) => (offset = top));
}

/// Open the panel at `scroll` and report its offset when a choice is clicked, so a long list reopened
/// after every choice stays where the person was instead of jumping back to its top. The capture phase
/// reads it before the row's handler removes the panel, which resets the offset; a scroll listener
/// would miss a scroll made in the same frame as the click.
function show(at: menu.Where, items: menu.Item[], scroll: number, chosen: (top: number) => void) {
  const panel = menu.openAt(at, items, "tools");
  panel.scrollTop = scroll;
  panel.addEventListener("click", () => chosen(panel.scrollTop), true);
}

export type FlatPick = {
  rows: Row[];
  /// The layer as it stands; null selects nothing on its own.
  current: () => Selection | null;
  set: (sel: Selection | null) => Promise<void> | void;
  at: () => { x: number; y: number };
  noneLabel: string;
};

/// Picker for the base layer (the board's global tools), which has nothing above it to inherit, so an
/// item is on exactly when this layer adds it and does not remove it. No provenance fetch is needed.
export function openFlat(p: FlatPick, scroll = 0) {
  const current = p.current();
  const on = (id: string) => (current?.add.includes(id) ?? false) && !(current?.remove.includes(id) ?? false);
  let offset = scroll;
  const choose = (sel: Selection | null) => {
    void Promise.resolve(p.set(sel)).then(() => openFlat(p, offset));
  };
  const items: menu.Item[] = [];
  if (!p.rows.length) items.push({ label: p.noneLabel, disabled: true });
  for (const row of p.rows) {
    const isOn = on(row.id);
    items.push({
      label: row.label,
      hint: row.hint,
      checked: isOn,
      run: () => choose(toggleSelection(current, row.id, !isOn)),
    });
  }
  if (p.rows.length && current) {
    items.push("sep", {
      label: t("tools.reset"),
      run: () => choose(null),
    });
  }
  show(p.at(), items, scroll, (top) => (offset = top));
}
