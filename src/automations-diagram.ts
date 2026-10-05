import { h } from "./util";
import * as ui from "./ui";
import { t } from "./i18n";
import { toolbar, listState } from "./components/compositions";
import { workflowDiagram, type Workflow } from "./automations-model";

/** Read-only, local rendering. Disposed or superseded renders never replace the current view. */
export function automationDiagram(host: HTMLElement, workflow: Workflow, portLabel: (port: string) => string): () => void {
  let generation = 0, disposed = false, detailed = false, zoom = 1;
  const root = h("section", "automations-mermaid");
  const controls = h("div", "automations-mermaid-controls");
  const viewport = h("div", "automations-mermaid-viewport");
  viewport.tabIndex = 0; viewport.setAttribute("aria-label", t("automations.diagram"));
  root.append(controls, viewport); host.append(root);
  const draw = async () => {
    const current = ++generation;
    const graph = workflowDiagram(workflow, detailed, portLabel);
    const toggle = ui.checkbox(t("automations.detailedDiagram"), detailed);
    toggle.control.onchange = () => { detailed = toggle.control.checked; void draw(); };
    const size = ui.button(`${Math.round(zoom * 100)}%`, () => { zoom = 1; scale(); }, "ghost");
    const scale = () => {
      const svg = viewport.querySelector("svg");
      if (svg) { svg.style.width = `${Number(svg.dataset.width) * zoom}px`; svg.style.height = `${Number(svg.dataset.height) * zoom}px`; }
      size.textContent = `${Math.round(zoom * 100)}%`;
    };
    controls.replaceChildren(toolbar([toggle.label, h("span", "ui-hint", graph.hiddenEdges ? t("automations.hiddenDiagramPaths", { count: graph.hiddenEdges }) : t("automations.allDiagramPaths"))], [
      ui.button(t("automations.zoomOut"), () => { zoom = Math.max(.5, zoom - .1); scale(); }, "ghost"), size,
      ui.button(t("automations.zoomIn"), () => { zoom = Math.min(2, zoom + .1); scale(); }, "ghost"),
    ]));
    viewport.replaceChildren(listState({ kind: "loading", text: t("automations.loadingDiagram") }));
    try {
      const { default: mermaid } = await import("mermaid");
      if (disposed || current !== generation) return;
      const style = getComputedStyle(root);
      mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: "base", htmlLabels: false,
        flowchart: { htmlLabels: false, useMaxWidth: false, curve: "basis", nodeSpacing: 36, rankSpacing: 48 },
        fontFamily: style.fontFamily, themeVariables: { fontFamily: style.fontFamily, fontSize: "15px",
          primaryColor: style.getPropertyValue("--bg-side").trim(), primaryTextColor: style.getPropertyValue("--fg").trim(),
          primaryBorderColor: style.getPropertyValue("--line-raised").trim(), lineColor: style.getPropertyValue("--fg-2").trim(),
          edgeLabelBackground: style.getPropertyValue("--bg").trim() } });
      const staging = h("div", "automations-mermaid-staging"); root.append(staging);
      let svg: string;
      try { ({ svg } = await mermaid.render(`automation-diagram-${crypto.randomUUID()}`, graph.source, staging)); }
      finally { staging.remove(); }
      if (disposed || current !== generation) return;
      viewport.innerHTML = svg;
      const diagram = viewport.querySelector("svg")!;
      // SVG text labels retain Mermaid's numeric entities with htmlLabels disabled.
      // Decode only text nodes, once; never parse the resulting user text as markup.
      const text = document.createTreeWalker(diagram, NodeFilter.SHOW_TEXT);
      for (let node = text.nextNode(); node; node = text.nextNode()) if (node.parentElement?.closest("text")) {
        node.textContent = node.textContent!.replace(/&#(\d+);/g, (entity, code: string) => Number(code) <= 0x10ffff ? String.fromCodePoint(Number(code)) : entity);
      }
      diagram.setAttribute("role", "img"); diagram.setAttribute("aria-label", workflow.name);
      diagram.dataset.width = String(diagram.viewBox.baseVal.width); diagram.dataset.height = String(diagram.viewBox.baseVal.height);
      scale();
    } catch {
      if (!disposed && current === generation) viewport.replaceChildren(listState({ kind: "error", text: t("automations.diagramFailed"), retry: { label: t("automations.retry"), run: () => { void draw(); } } }));
    }
  };
  void draw();
  return () => { disposed = true; generation++; root.remove(); };
}
