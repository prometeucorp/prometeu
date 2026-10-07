import { listen } from "@tauri-apps/api/event";
import { invoke } from "./ipc";
import type { IslandSnapshot } from "./island";
import { islandView, redraw, requestPanels, type IslandLayout } from "./island-view";
import type { Notice } from "./notifications";
import "./ui.css";
import "./notifications.css";

/// The notch overlay renders the main window's island snapshot and answers requests directly.
let snapshot: IslandSnapshot = { tabs: [], usage: [] };
let layout: IslandLayout = { expanded: false, top: 32 };
let notice: Notice | null = null;
const open = (tab: string) => void invoke("island_open", { tab }).catch(console.error);
const panels = requestPanels((session, frame) => invoke("chat_control", { session, frame }), open);
const actions = {
  open,
  panel: panels.panel,
  noticeOpen: () => void invoke("notification_open").catch(console.error),
};

function draw() {
  panels.prune(snapshot);
  redraw(document.body, () => islandView(snapshot, layout, notice, actions));
}
new ResizeObserver(() => {
  if (layout.expanded) void invoke("island_resize", { height: Math.ceil(document.body.getBoundingClientRect().height) }).catch(console.error);
}).observe(document.body);
// Elapsed times and quota resets move without new events.
setInterval(draw, 30_000);

const seen = new Set<string>();
await listen<IslandSnapshot>("island", ({ payload }) => { seen.add("snapshot"); snapshot = payload; draw(); });
await listen<IslandLayout>("island-layout", ({ payload }) => { seen.add("layout"); layout = payload; draw(); });
await listen<Notice | null>("notification", ({ payload }) => { seen.add("notice"); notice = payload; draw(); });
const current = await invoke("island_current");
if (!seen.has("snapshot") && current.snapshot) snapshot = current.snapshot;
if (!seen.has("layout")) layout = current.layout;
if (!seen.has("notice")) notice = current.notice;
draw();
