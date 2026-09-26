import { t } from "./i18n";
import { h } from "./util";
import { button, input, field, select } from "./ui";
import { sectionHeader, toolbar, itemRow, overflowAction, listState, type ListState } from "./components/compositions";

/** Independent building blocks with local callbacks; no resource or Actions adapters. */
export function compositionExamples(initialState = "ready") {
  const root = h("section", "ui-gallery-section composition-example"); root.id = "components-preview";
  root.append(h("h2", "", t("ui.compositionPreview")));
  const feedback = h("p", "ui-hint"); feedback.setAttribute("role", "status");
  const report = (name: string) => { feedback.textContent = t("ui.resourceAction", { name }); };
  const heading = sectionHeader({ title: t("actions.commands"), count: 2,
    description: t("actions.intro"), actions: [button(t("actions.newCommand"), () => report(t("actions.newCommand")))],
  });
  const query = input(""); query.type = "search"; query.setAttribute("aria-label", t("ui.componentSearch"));
  query.placeholder = t("ui.componentSearch");
  const state = select(initialState, [["ready", t("ui.resourceReady")], ["empty", t("ui.resourceEmpty")],
    ["loading", t("ui.resourceLoading")], ["error", t("ui.resourceError")], ["busy", t("ui.resourceBusy")]]);
  const tools = toolbar([query], [field(t("ui.resourceState"), state.control)]);
  const content = h("div", "composition-items");
  function paint() {
    const message: ListState = state.value === "error"
      ? { kind: "error", text: t("ui.resourceFailure"), retry: { label: t("settings.resourceRetry"), run: () => {
        state.value = "ready"; paint(); query.focus();
      } } }
      : state.value === "loading" ? { kind: "loading", text: t("settings.resourceLoading") }
      : state.value === "empty" ? { kind: "empty", text: t("actions.noCommands") } : { kind: "ready" };
    content.replaceChildren(listState(message));
    content.setAttribute("aria-busy", String(state.value === "loading"));
    if (message.kind !== "ready") return;
    const examples = [{ title: "/review", description: "Review changes before merging", glyph: "terminal" as const },
      { title: "Review assistant", description: "An independent example assembled from the same reusable components", glyph: "eye" as const }];
    const matches = examples.filter(item => `${item.title} ${item.description}`.toLowerCase().includes(query.value.toLowerCase()));
    for (const item of matches) {
      const busy = state.value === "busy";
      const edit = button(t("actions.edit"), () => report(item.title), "ghost"); edit.disabled = busy;
      const row = itemRow({ ...item, busy, status: busy ? t("settings.resourceLoading") : undefined,
        actions: [edit, overflowAction({ label: `${t("actions.more")} · ${item.title}`, key: item.title, busy,
          items: [{ label: t("actions.remove"), run: () => report(item.title) }] })],
      });
      content.append(row.root);
    }
    if (!matches.length) content.append(listState({ kind: "empty", text: t("settings.resourceNoResults") }));
  }
  query.oninput = paint; state.onchange = paint;
  paint(); root.append(heading, tools, content, feedback);
  return root;
}
