import { current as language, t } from "./i18n";
import type { CodeMapLabels } from "./components/code-map";

export function codeMapLabels(): CodeMapLabels {
  return {
    search: t("map.search"), clearSearch: t("map.clear"), root: t("map.root"), overview: t("map.overview"),
    results: t("map.results"), empty: t("map.empty"), noResults: t("map.noResults"),
    directories: t("map.directories"), back: t("map.back"), previous: t("map.previous"), next: t("map.next"),
    directoryCount: (groups, files) => t("map.directoryCount", { groups, files }),
    fileCount: count => t("map.fileCount", { count }),
    range: (start, end, total) => t("map.range", { start, end, total }),
    selectFile: t("map.selectFile"), selectSymbol: t("map.selectSymbol"), loading: t("map.loading"),
    sync: t("map.sync"), syncing: t("map.syncing"), retry: t("map.retry"), expand: t("map.expand"),
    collapse: t("map.collapse"), zoomIn: t("map.zoomIn"), zoomOut: t("map.zoomOut"), fit: t("map.fit"),
    openSource: t("map.openSource"), truncated: t("map.truncated"), uncertain: t("map.uncertain"),
    excerptTruncated: t("map.excerptTruncated"), syntaxErrors: t("map.syntaxErrors"), unresolved: t("map.unresolved"),
    graphCount: (files, edges, total) => t("map.graphCount", { files, edges, total }),
    resultCount: count => t("map.resultCount", { count }),
    lastSync: timestamp => timestamp === null ? t("map.notIndexed") : t("map.lastSync", {
      time: new Date(timestamp * 1000).toLocaleTimeString(language(), { hour: "2-digit", minute: "2-digit", second: "2-digit" }),
    }),
    generation: value => t("map.generation", { value }),
    relationship: (kind, resolution, confidence, ambiguous) => t("map.relationship", {
      kind, resolution, confidence, ambiguity: ambiguous ? t("map.uncertain") : "",
    }),
  };
}
