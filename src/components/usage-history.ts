import { h } from "../util";
import { t } from "../i18n";
import { button, formDialog } from "./primitives";
import { sectionHeader } from "./compositions";
import "./usage-history.css";

type Metric = { label: string; value: string; note?: string };
export type HistorySession = {
  id: string; title: string; workspace: string; provider: string; turns: string; tokens: string; cost: string;
  coverage: string; status: "complete" | "partial" | "unknown";
  details: Metric[]; notes: string[];
};
export type UsageHistoryData = {
  metrics: Metric[]; inputShare: number | null;
  tokens: (Metric & { kind: "input" | "output" | "subset" })[];
  sources: (Metric & { share: number | null })[];
  sessions: HistorySession[]; models: Metric[];
};

/** Local usage presentation. Its host supplies formatted data and owns queries and subscriptions. */
export function usageHistoryView() {
  const root = h("div", "usage-history");
  const metrics = h("div", "history-metrics");
  const overview = h("div", "history-overview");
  const tokens = h("section", "history-panel"), sources = h("section", "history-panel");
  const sessions = h("section", "history-sessions"), models = h("section", "history-panel history-models");
  overview.append(tokens, sources); root.append(metrics, overview, sessions, models);
  let data: UsageHistoryData | null = null;
  let detail: { id: string; dialog: ReturnType<typeof formDialog> } | null = null;

  function detailBody(session: HistorySession) {
    if (!detail) return;
    const list = h("dl", "history-breakdown");
    for (const metric of session.details) list.append(h("dt", "", metric.label), h("dd", "", metric.value));
    detail.dialog.body.replaceChildren(h("p", "ui-hint", [session.workspace, session.provider].filter(Boolean).join(" · ")),
      list, ...session.notes.map(note => h("p", "ui-hint", note)));
  }
  function openDetail(id: string) {
    const session = data?.sessions.find(row => row.id === id); if (!session) return;
    detail?.dialog.close();
    const dialog = formDialog({ title: session.title, save: t("review.close"), cancel: t("review.close"),
      submit: async () => {}, error: String, track: "usage_history.details", area: "usage_history", closed: () => { detail = null; } });
    dialog.save.remove(); dialog.root.classList.add("history-dialog");
    detail = { id, dialog }; detailBody(session); dialog.open();
  }
  return {
    root,
    clear() { detail?.dialog.close(); data = null; root.hidden = true; },
    destroy() { detail?.dialog.close(); root.remove(); },
    update(next: UsageHistoryData) {
      const focused = root.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.focus : undefined;
      const scroll = sessions.querySelector(".history-table-scroll")?.scrollLeft ?? 0;
      data = next; root.hidden = false;
      metrics.replaceChildren(...next.metrics.map(metric => {
        const card = h("div", "history-metric");
        card.append(h("div", "history-metric-label", metric.label), h("div", "history-metric-value", metric.value));
        if (metric.note) card.append(h("p", "ui-hint", metric.note));
        return card;
      }));
      const bar = h("div", "history-token-bar"); bar.setAttribute("aria-hidden", "true");
      if (next.inputShare !== null) {
        const input = h("span", "history-input"), output = h("span", "history-output");
        input.style.width = `${next.inputShare}%`; output.style.width = `${100 - next.inputShare}%`; bar.append(input, output);
      }
      const breakdown = h("dl", "history-breakdown");
      for (const metric of next.tokens) breakdown.append(h("dt", `history-token-${metric.kind}`, metric.label), h("dd", "", metric.value));
      tokens.replaceChildren(sectionHeader({ title: t("history.breakdown") }), bar, breakdown);
      sources.replaceChildren(sectionHeader({ title: t("history.sources") }));
      for (const metric of next.sources) {
        const row = h("div", "history-source"), line = h("div", "history-source-label"), track = h("div", "history-source-track"), fill = h("span", "");
        line.append(h("span", "", metric.label), h("span", "", metric.value));
        fill.style.width = `${metric.share ?? 0}%`; track.append(fill); row.append(line, track); sources.append(row);
      }
      if (!next.sources.length) sources.append(h("p", "ui-hint", t("history.empty")));
      sessions.replaceChildren(sectionHeader({ title: t("history.sessions"), supporting: [h("p", "ui-hint", t("history.sorted"))] }));
      const region = h("div", "history-table-scroll"); region.tabIndex = 0; region.dataset.focus = "history-table";
      region.setAttribute("role", "region"); region.setAttribute("aria-label", t("history.sessions"));
      region.onkeydown = event => {
        if (event.target !== region || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
        event.preventDefault(); region.scrollLeft += event.key === "ArrowRight" ? 120 : -120;
      };
      const table = h("table", "history-table"), head = h("thead", ""), headings = h("tr", ""), body = h("tbody", "");
      for (const [index, label] of [t("history.session"), t("history.provider"), t("history.turns"), t("history.tokens"), t("history.cost"), t("history.coverage")].entries()) {
        const cell = h("th", index >= 2 && index <= 4 ? "history-numeric" : "", label); cell.setAttribute("scope", "col"); headings.append(cell);
      }
      head.append(headings); table.append(head, body); region.append(table);
      for (const session of next.sessions) {
        const row = h("tr", ""), name = h("td", "");
        const open = button(session.title, () => openDetail(session.id), "ghost", "usage_history.session");
        open.classList.add("history-session"); open.dataset.focus = `history-session-${session.id}`; open.setAttribute("aria-haspopup", "dialog");
        name.append(open, h("div", "history-session-meta", session.workspace));
        const coverage = h("td", ""); coverage.append(h("span", `history-status ${session.status}`, session.coverage));
        row.append(name, h("td", "", session.provider), h("td", "history-numeric", session.turns), h("td", "history-numeric", session.tokens), h("td", "history-numeric", session.cost), coverage); body.append(row);
      }
      if (next.sessions.length) sessions.append(region);
      else sessions.append(h("p", "history-empty", t("history.empty")));
      sessions.append(h("p", "ui-hint", t("history.sessionHint")));
      models.replaceChildren(sectionHeader({ title: t("usage.models") }));
      for (const metric of next.models) { const row = h("div", "history-model"); row.append(h("span", "", metric.label), h("span", "", metric.value)); models.append(row); }
      if (!next.models.length) models.append(h("p", "ui-hint", t("history.noModels")));
      models.append(h("p", "ui-hint", t("history.modelHint")));
      region.scrollLeft = scroll;
      if (focused) (root.querySelector<HTMLElement>(`[data-focus="${CSS.escape(focused)}"]`) ?? region).focus({ preventScroll: true });
      if (detail) {
        const session = next.sessions.find(row => row.id === detail!.id);
        if (session) detailBody(session); else detail.dialog.close();
      }
    },
  };
}
