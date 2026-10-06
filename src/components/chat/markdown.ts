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

/// GitHub comments mix Markdown with a small HTML vocabulary; rebuild only these tags, keep only these attributes, and escape everything else.
const TAGS = new Set(["a", "b", "blockquote", "br", "code", "dd", "del", "details", "div", "dl", "dt", "em", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i", "img", "kbd", "li", "ol", "p", "pre", "s", "span", "strong", "sub", "summary", "sup", "table", "tbody", "td", "th", "thead", "tr", "ul"]);
const TAG = /<(\/?)([a-zA-Z][\w-]*)((?:\s+[^\s"'>/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'>]+))?)*)\s*\/?>/g;
const ATTR = /([^\s"'>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;
const escText = (s: string) => s.replace(/&(?!#?\w+;)/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

function safeHtml(src: string): string {
  let out = "", at = 0;
  src = src.replace(/<!--[\s\S]*?(?:-->|$)/g, "");
  for (const m of src.matchAll(TAG)) {
    out += escText(src.slice(at, m.index));
    at = m.index + m[0].length;
    const [, close, name, raw] = m;
    const tag = name.toLowerCase();
    if (!TAGS.has(tag)) continue;
    const attrs = new Map([...raw.matchAll(ATTR)].map(a => [a[1].toLowerCase(), a[2] ?? a[3] ?? a[4] ?? ""]));
    // The CSP blocks remote images, so badges read as their alt text like Markdown images do.
    if (tag === "img") out += `<span class="img">${esc(attrs.get("alt") || "")}</span>`;
    else if (close) out += `</${tag}>`;
    else if (tag === "a") out += `<a class="lnk" href="${esc(safeHref(attrs.get("href") ?? ""))}" rel="noreferrer noopener">`;
    else out += `<${tag}${tag === "details" && attrs.has("open") ? " open" : ""}>`;
  }
  return out + escText(src.slice(at));
}

let allowHtml = false;

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
      return allowHtml ? safeHtml(text) : esc(text);
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

/// `html` admits the sanitized GitHub subset; agent output keeps raw HTML as text.
export function md(src: string, { html = false } = {}): string {
  allowHtml = html;
  try {
    return marked.parse(src, { async: false }) as string;
  } finally {
    allowHtml = false;
  }
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
