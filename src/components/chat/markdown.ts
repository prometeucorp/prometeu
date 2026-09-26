import { marked, type Tokens } from "marked";
import { diffHtml, highlight } from "../../highlight";
import { icon } from "../icons";
import { t } from "../../i18n";
import "./markdown.css";

/// Use marked for agent markdown, escaping raw HTML, sharing syntax highlighting with the viewer, and intercepting links to preserve the application window.

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const safeHref = (href: string) => {
  try {
    const url = new URL(href);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : "#";
  } catch {
    return "#";
  }
};

/// Normalize fenced-code language names for highlight().
const LANG: Record<string, string> = {
  typescript: "ts", javascript: "js", rust: "rs", python: "py", shell: "sh", bash: "sh", zsh: "sh",
  ruby: "rb", yml: "yaml", jsonc: "json", console: "sh", text: "txt", plaintext: "txt",
};

export function codeBlock(text: string, lang = ""): string {
  const raw = (lang ?? "").trim().split(/\s+/)[0].toLowerCase();
  const diff = raw === "diff" || raw === "patch";
  const ext = LANG[raw] ?? raw ?? "txt";
  const code = diff ? diffHtml(text) : highlight(text, `x.${ext || "txt"}`);
  return `<div class="md-code-block"><div class="md-code-toolbar"><button type="button" class="ui-button ghost sm md-code-copy" title="${esc(t("chat.copyCode"))}" aria-label="${esc(t("chat.copyCode"))}" data-code="${esc(text)}">${icon("copy", 14)}</button></div><pre class="code${diff ? " tdiff" : ""}"><code>${code}</code></pre></div>\n`;
}

marked.use({
  gfm: true,
  renderer: {
    html({ text }: Tokens.HTML | Tokens.Tag) {
      return esc(text);
    },
    code({ text, lang }: Tokens.Code) {
      return codeBlock(text, lang);
    },
    link({ href, tokens }: Tokens.Link) {
      return `<a class="lnk" href="${esc(safeHref(href))}" rel="noreferrer noopener">${this.parser.parseInline(tokens)}</a>`;
    },
    image({ href, text }: Tokens.Image) {
      return `<span class="img">${esc(text || href)}</span>`;
    },
  },
});

export function md(src: string): string {
  return marked.parse(src, { async: false }) as string;
}

/// Delegate clicks so streamed markdown replacements need no new listeners.
export function initCodeCopy(createReport: () => (message: string, isError?: boolean) => void) {
  document.addEventListener("click", async (event) => {
    const control = (event.target as Element | null)?.closest<HTMLButtonElement>("button.md-code-copy");
    if (!control || control.disabled) return;
    const report = createReport();
    control.disabled = true;
    try {
      await navigator.clipboard.writeText(control.dataset.code ?? "");
      report(t("chat.codeCopied"));
      control.innerHTML = icon("check", 14);
      control.title = t("chat.codeCopied");
      control.setAttribute("aria-label", control.title);
      setTimeout(() => {
        control.innerHTML = icon("copy", 14);
        control.title = t("chat.copyCode");
        control.setAttribute("aria-label", control.title);
      }, 1200);
    } catch {
      report(t("chat.copyCodeFailed"), true);
    } finally {
      control.disabled = false;
    }
  });
}
