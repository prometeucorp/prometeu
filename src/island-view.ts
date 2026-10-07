import { toolLabel } from "./components/chat/content";
import type { ConversationCommandV1, RequestResponse } from "./conversation";
import { t } from "./i18n";
import { brand } from "./icons";
import type { IslandRequest, IslandSnapshot, IslandTab, IslandUsage } from "./island";
import type { Notice } from "./notifications";
import { button, input } from "./ui";
import { h, span, template } from "./util";

export type IslandLayout = { expanded: boolean; top: number };
export type IslandActions = {
  open: (tab: string) => void;
  panel: (tab: IslandTab) => HTMLElement;
  noticeOpen: () => void;
};
type Tone = "run" | "wait" | "done" | "idle";

/// Pixel-art flame: a shared base and two alternating tips. o/m/c/w run from the outer edge to the
/// core; the status class recolors them.
const BASE = [".oommmoo.", ".omccmmo.", "oomcccmoo", "omccwccmo", "omcwwwcmo", ".omcwcmo.", "..ooooo.."];
const TIPS = [
  ["....o....", "...oo..o.", "..omo.oo.", "..ommomo."],
  [".....o...", ".o..oo...", ".oo.omo..", ".omoommo."],
];
const pixels = (rows: string[], top: number) => rows.flatMap((row, y) => [...row].flatMap((cell, x) =>
  cell === "." ? [] : [`<rect class="${cell}" x="${x}" y="${y + top}" width="1" height="1"/>`])).join("");
const FLAME = `<svg viewBox="0 0 9 11" shape-rendering="crispEdges" aria-hidden="true">${pixels(BASE, 4)}${
  TIPS.map((tip, i) => `<g class="tip${i}">${pixels(tip, 0)}</g>`).join("")}</svg>`;
const flame = (tone: Tone) => template("span", `island-flame ${tone}`, FLAME);

const toneOf = (tab: IslandTab): Tone =>
  tab.status === "querendo" ? "wait" : tab.status === "rodando" ? "run" : tab.unseen ? "done" : "idle";

/// Native overlay and browser mock render the same escaped content.
export function islandView(snapshot: IslandSnapshot, layout: IslandLayout, notice: Notice | null, actions: IslandActions) {
  const root = h("div", "island");
  root.dataset.expanded = String(layout.expanded);
  root.style.setProperty("--island-top", `${layout.top}px`);
  const tones = new Set(snapshot.tabs.map(toneOf));
  const waiting = snapshot.tabs.filter(tab => tab.status === "querendo").length;
  const running = snapshot.tabs.filter(tab => tab.status === "rodando").length;
  const bar = h("div", "island-bar");
  bar.append(flame(waiting ? "wait" : running ? "run" : tones.has("done") ? "done" : "idle"));
  if (waiting) bar.append(h("span", "island-count wait", String(waiting)));
  else if (tones.has("done")) bar.append(h("span", "island-signal"));
  else if (running) bar.append(h("span", "island-count", String(running)));
  root.append(bar);
  if (!layout.expanded) return root;
  const body = h("div", "island-body");
  if (snapshot.usage.length) body.append(usage(snapshot.usage));
  // A notice about a conversation highlights its row; only notices without one need their own line.
  if (notice && !notice.tab) {
    const line = button("", actions.noticeOpen, "ghost");
    line.classList.add("island-notice");
    line.append(flame("run"), h("strong", "", notice.title), h("span", "", notice.body));
    body.append(line);
  }
  const list = h("div", "island-list");
  list.append(...snapshot.tabs.map(tab => row(tab, actions, tab.id === notice?.tab)));
  if (!snapshot.tabs.length) list.append(h("p", "island-empty", t("island.empty")));
  body.append(list);
  root.append(body);
  return root;
}

function usage(items: IslandUsage[]) {
  const line = h("div", "island-usage");
  const now = Date.now() / 1000;
  for (const item of items) {
    const group = template("span", "island-quota", brand(item.agent, 12));
    item.windows.forEach((quota, i) => {
      if (i) group.append(h("i", "", "|"));
      group.append(
        h("b", "", quota.label),
        h("span", quota.pct >= 90 ? "hot" : quota.pct >= 75 ? "warn" : "ok", `${Math.round(quota.pct)}%`),
        h("small", "", quota.resets > now ? span(quota.resets - now) : t("status.now")),
      );
    });
    line.append(group);
  }
  return line;
}

/// Conversations that need attention show their prompt and activity; idle ones collapse to one line.
function row(tab: IslandTab, actions: IslandActions, focus: boolean) {
  const tone = toneOf(tab);
  const detailed = tone !== "idle" || focus;
  const item = h("div", `island-row ${tone}${detailed ? " detailed" : ""}${focus ? " focus" : ""}`);
  const open = button("", () => actions.open(tab.id), "ghost");
  open.classList.add("island-open");
  const copy = h("span", "island-copy");
  const head = h("span", "island-head");
  const tag = template("span", "island-tag", brand(tab.agent, 11));
  tag.append(tab.model);
  head.append(h("strong", "", tab.title), tag);
  if (tab.since) head.append(h("time", "", span((Date.now() - tab.since) / 1000)));
  copy.append(head);
  if (detailed && tab.prompt) copy.append(h("span", "island-prompt", t("island.you", { text: tab.prompt })));
  if (detailed && !tab.request) copy.append(h("span", "island-activity", activity(tab)));
  open.append(flame(tone), copy);
  item.append(open);
  if (tab.request) item.append(actions.panel(tab));
  return item;
}

function activity(tab: IslandTab) {
  if (tab.status === "querendo") return tab.note ?? t("island.waiting");
  if (tab.status === "pronta") return t("island.done");
  return tab.activity ? `${toolLabel(tab.activity.tool)} ${tab.activity.target}`.trim() : t("island.working");
}

/// Panels keep their own progress and typing, so each one survives snapshot redraws.
export function requestPanels(control: (tab: string, frame: ConversationCommandV1) => Promise<unknown>, open: (tab: string) => void) {
  const panels = new Map<string, HTMLElement>();
  return {
    panel(tab: IslandTab) {
      const request = tab.request!;
      const known = panels.get(request.id);
      if (known) return known;
      // Hold the panel while its answer travels; the next snapshot removes it once the request closes.
      const respond = (response: RequestResponse) => {
        panel.inert = true;
        void control(tab.id, { v: 1, type: "request.respond", requestId: request.id, response })
          .catch(error => { panel.inert = false; console.error(error); });
      };
      const panel = h("div", `island-request ${request.requestKind}`);
      if (request.requestKind === "question") question(panel, tab, request, respond, () => open(tab.id));
      else approval(panel, request, respond, () => open(tab.id));
      panels.set(request.id, panel);
      return panel;
    },
    prune(snapshot: IslandSnapshot) {
      const live = new Set(snapshot.tabs.flatMap(tab => tab.request ? [tab.request.id] : []));
      for (const id of panels.keys()) if (!live.has(id)) panels.delete(id);
    },
  };
}

const label = (text: string, extra = "") => {
  const line = h("div", "island-label", text);
  if (extra) line.append(h("small", "", extra));
  return line;
};

/// Tool approvals allow or deny in place; plans may need edits, so refusing opens the conversation.
function approval(panel: HTMLElement, request: IslandRequest, respond: (response: RequestResponse) => void, open: () => void) {
  const plan = request.requestKind === "plan";
  const input = request.input;
  const text = (key: string) => typeof input[key] === "string" ? input[key] as string : null;
  panel.append(label(t(plan ? "chat.plan.title" : "island.permission")));
  if (!plan) {
    const tool = h("div", "island-tool");
    tool.append(h("b", "", toolLabel(request.tool)), h("code", "", text("file_path") ?? text("notebook_path") ?? text("path") ?? text("url") ?? ""));
    panel.append(tool);
  }
  const removed = text("old_string")?.split("\n") ?? [];
  const added = (text("new_string") ?? text("content"))?.split("\n") ?? [];
  const plain = (text("command") ?? text("plan"))?.split("\n") ?? [];
  const lines = [...removed.map(line => ["del", `- ${line}`]), ...added.map(line => ["add", `+ ${line}`]), ...plain.map(line => ["", line])];
  if (lines.length) {
    const code = h("pre", "island-code");
    code.append(...lines.slice(0, 8).map(([kind, line]) => h("span", kind, line)));
    if (lines.length > 8) code.append(h("span", "more", "…"));
    panel.append(code);
  }
  if (removed.length || added.length) panel.append(h("small", "island-stat", `+${added.length} −${removed.length}`));
  const choices = h("div", "island-actions");
  choices.append(
    plan ? button(t("island.openApp"), open) : button(t("chat.perm.no"), () => respond({ outcome: "deny", message: t("chat.perm.denied") })),
    button(t(plan ? "island.approve" : "chat.perm.yes"), () => respond({ outcome: "allow" }), "pri"),
  );
  panel.append(choices);
}

type Question = { question: string; multiSelect?: boolean; options?: { label: string; description?: string }[] };

/// One question at a time; a single choice answers it, and the last answer sends them all.
function question(panel: HTMLElement, tab: IslandTab, request: IslandRequest, respond: (response: RequestResponse) => void, open: () => void) {
  const questions = (Array.isArray(request.input.questions) ? request.input.questions : []) as Question[];
  const answers: Record<string, string> = {};
  let index = 0;
  const paint = () => {
    const current = questions[index];
    panel.replaceChildren(label(t("island.asks", { agent: t(`model.${tab.agent}`) }), questions.length > 1 ? `${index + 1}/${questions.length}` : ""));
    if (!current) {
      panel.append(button(t("island.openApp"), open));
      return;
    }
    const picked = new Set<string>();
    const other = input();
    other.classList.add("island-other");
    other.placeholder = t("chat.ask.other");
    const answer = () => {
      const value = [...picked, other.value.trim()].filter(Boolean).join(", ");
      if (!value) return;
      answers[current.question] = value;
      if (++index < questions.length) paint();
      else respond({ outcome: "answer", answers });
    };
    const options = h("div", "island-options");
    (current.options ?? []).forEach((option, i) => {
      const choice = button("", () => {
        if (!current.multiSelect) {
          picked.add(option.label);
          return answer();
        }
        if (!picked.delete(option.label)) picked.add(option.label);
        choice.classList.toggle("on", picked.has(option.label));
      }, "ghost");
      choice.classList.add("island-option");
      choice.append(h("kbd", "", String(i + 1)), h("b", "", option.label));
      if (option.description) choice.append(h("small", "", option.description));
      options.append(choice);
    });
    other.addEventListener("keydown", event => {
      if (event.key === "Enter") { event.preventDefault(); answer(); }
    });
    panel.append(h("p", "island-question", current.question), options, other);
    if (current.multiSelect) {
      const send = button(t("chat.ask.go"), answer, "pri");
      send.classList.add("island-send");
      panel.append(send);
    }
  };
  paint();
}
