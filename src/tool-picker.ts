import { invoke } from "./ipc";
import { t } from "./i18n";
import * as menu from "./menu";
import { toggleSelection, type EffectiveItem, type Provenance, type Selection } from "./types";

/// One provenance-aware tool picker shared by the MCP, plugin and standalone-skill axes (ADR 0043).
/// It reads the resolved effective set from `workspace_tools`, labels each row with where it came
/// from, and writes the workspace layer as inherit-plus-deltas so the global and project layers keep
/// flowing through. A change applies at the next spawn; the running session keeps its born set, so the
/// picker only states that instead of restarting anything.

export type Axis = "mcp" | "plugins" | "skills";

/// One selectable row. `id` is the hub identity used by the selection (skills ride `skill-<id>`).
/// `section` groups rows under a disabled header; headers render only when more than one group exists.
export type Row = { id: string; label: string; hint?: string; section?: string };

export type Pick = {
  workspace: string;
  axis: Axis;
  rows: Row[];
  /// The workspace layer as it stands; null inherits everything from the layers above.
  current: () => Selection | null;
  /// Persist the new workspace layer, or null to return the axis to inherit.
  set: (sel: Selection | null) => Promise<void> | void;
  at: () => { x: number; y: number };
  /// True while the agent is working, to explain that the change lands on the next message.
  working?: () => boolean;
  /// Shown when the hub for this axis is empty.
  noneLabel: string;
  /// Opens the project-trust prompt; offered only while the project layer has pending items.
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

export async function open(p: Pick) {
  let items0: EffectiveItem[] = [];
  try {
    const tools = await invoke("workspace_tools", { id: p.workspace });
    items0 = tools[p.axis];
  } catch {
    // An older or unavailable backend resolves nothing; fall back to the workspace's own adds.
  }
  const provenance = new Map(items0.map((e) => [e.id, e.provenance]));
  const current = p.current();
  const items: menu.Item[] = [];
  if (p.working?.()) items.push({ label: t("tools.appliesNext"), disabled: true }, "sep");
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
      badge: badgeFor(seen),
      run: () => {
        void Promise.resolve(p.set(toggleSelection(current, row.id, !on))).then(() => open(p));
      },
    });
  }
  if (p.rows.length) {
    // Explicitly start from nothing: unlike reset (which returns the axis to inherit), this writes
    // an empty `none` layer so the global, project and CLI contributions are all switched off.
    const empty = current?.base === "none" && !current.add.length;
    items.push("sep", {
      label: t("tools.selectNone"),
      checked: empty,
      run: () => {
        void Promise.resolve(p.set({ base: "none", add: [], remove: [] })).then(() => open(p));
      },
    });
    if (current) {
      items.push({
        label: t("tools.reset"),
        run: () => {
          void Promise.resolve(p.set(null)).then(() => open(p));
        },
      });
    }
  }
  // A pending project declaration resolves but is not injected until the person approves its hash.
  if (p.trust && items0.some((e) => e.provenance === "pending")) {
    items.push("sep", {
      label: t("tools.trust.prompt"),
      run: () => p.trust!(),
    });
  }
  menu.openAt(p.at(), items, "tools");
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
export function openFlat(p: FlatPick) {
  const current = p.current();
  const on = (id: string) => (current?.add.includes(id) ?? false) && !(current?.remove.includes(id) ?? false);
  const items: menu.Item[] = [];
  if (!p.rows.length) items.push({ label: p.noneLabel, disabled: true });
  for (const row of p.rows) {
    const isOn = on(row.id);
    items.push({
      label: row.label,
      hint: row.hint,
      checked: isOn,
      run: () => {
        void Promise.resolve(p.set(toggleSelection(current, row.id, !isOn))).then(() => openFlat(p));
      },
    });
  }
  if (p.rows.length && current) {
    items.push("sep", {
      label: t("tools.reset"),
      run: () => {
        void Promise.resolve(p.set(null)).then(() => openFlat(p));
      },
    });
  }
  menu.openAt(p.at(), items, "tools");
}
