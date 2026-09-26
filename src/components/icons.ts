import { icon } from "../../packages/design-system/src/icons";
export { icon, iconNames, wave, type IconName } from "../../packages/design-system/src/icons";
const CLAUDE_MARK = new URL("../claude.svg", import.meta.url).href;

/* Stage icons. */

/// Derive the progress ring from stage position: dashed initially, partially filled between stages, and a check at the end. No stage-name mapping is required.
export function stageIcon(at: number, total: number, size = 16): string {
  const svg = (inner: string, color = "currentColor") =>
    `<svg class="ic" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" ` +
    `stroke="${color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">` +
    inner +
    "</svg>";

  const ring = '<circle cx="12" cy="12" r="9"/>';
  if (at <= 0) return svg('<circle cx="12" cy="12" r="9" stroke-dasharray="2.6 2.6"/>');
  if (at >= total - 1) return svg(`${ring}<path d="m8.5 12 2.5 2.5 4.5-5"/>`, "var(--done)");

  // Use a radius-4 circle with circumference about 25.1 for the filled arc. Butt line caps prevent the thick stroke from exaggerating partial progress.
  const fill = (25.1 * at) / (total - 1);
  return svg(
    `${ring}<circle cx="12" cy="12" r="4" stroke-width="8" stroke-linecap="butt" ` +
      `stroke-dasharray="${fill.toFixed(1)} 25.1" transform="rotate(-90 12 12)"/>`,
  );
}

/// An initial in a square with a stable name-derived color.
const HUES = ["#6525c9", "#c9552a", "#2a7fc9", "#2a9d6e", "#c9a02a", "#c92a6a"];
export function avatar(name: string): string {
  const el = document.createElement("span");
  el.className = "avatar";
  el.style.background = hue(name);
  el.textContent = initial(name);
  return el.outerHTML;
}

const hue = (name: string) => {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return HUES[h % HUES.length];
};
const initial = (name: string) => (name.trim()[0] ?? "?").toUpperCase();

/// Split workspace avatars by repository, retaining each project's color and initial. Show up to four slices; additional repositories become a count.
export function avatars(names: string[]): string {
  if (names.length < 2) return avatar(names[0] ?? "?");
  const slices = names.length > 4 ? [...names.slice(0, 3), `+${names.length - 3}`] : names;
  const el = document.createElement("span");
  el.className = `avatar multi n${slices.length}`;
  for (const [i, name] of slices.entries()) {
    const slice = document.createElement("i");
    slice.style.background = hue(names[i]);
    slice.textContent = name.startsWith("+") ? name : initial(name);
    el.append(slice);
  }
  return el.outerHTML;
}

/* File-type icons. */

/// Compact file-type glyphs remain legible at 16px; unknown types use the neutral file icon.
const S = 'stroke-linecap="round" stroke-linejoin="round" fill="none"';
const badge = (bg: string, fg: string, text: string) =>
  `<rect x="2" y="2" width="20" height="20" rx="4" fill="${bg}"/>` +
  `<text x="12" y="16.5" text-anchor="middle" font-family="var(--font)" font-size="10" font-weight="700" fill="${fg}">${text}</text>`;

const FILE_ICONS: Record<string, string> = {
  ruby:
    '<path d="M7 3h10l4 6-9 12L3 9z" fill="#d63d38"/>' +
    `<path d="M3 9h18M7 3l5 6 5-6M12 9v12" stroke="#fff" stroke-opacity=".4" stroke-width="1.2" ${S}/>`,
  yaml:
    '<text x="12" y="11.5" text-anchor="middle" font-family="var(--font)" font-size="11" font-weight="900" letter-spacing="-.5" fill="#d63d38">YA</text>' +
    '<text x="12" y="22" text-anchor="middle" font-family="var(--font)" font-size="11" font-weight="900" letter-spacing="-.5" fill="#d63d38">ML</text>',
  md:
    '<rect x="1.5" y="5" width="21" height="14" rx="2" fill="#a4a09d"/>' +
    '<text x="8" y="16.2" text-anchor="middle" font-family="var(--font)" font-size="10" font-weight="800" fill="#141110">M</text>' +
    `<path d="M16.5 9v6m-2.5-2.5 2.5 2.5 2.5-2.5" stroke="#141110" stroke-width="1.8" ${S}/>`,
  docker:
    '<path d="M2 12.5h18.5c.5 0 1.5-.3 2-.8-.8-1.3-2.3-1.2-3-.9-.2-1.4-1-2.2-1.8-2.6-.9 1-1 2.4-.5 3.3H2c0 4.5 3 8.5 8.5 8.5 4.5 0 8-2.5 9.5-6.5" fill="#2496ed"/>' +
    '<g fill="#2496ed"><rect x="4.5" y="8.5" width="3" height="3"/><rect x="8.5" y="8.5" width="3" height="3"/><rect x="12.5" y="8.5" width="3" height="3"/><rect x="8.5" y="4.5" width="3" height="3"/><rect x="12.5" y="4.5" width="3" height="3"/></g>',
  env:
    `<path d="M4 6h16M4 12h16M4 18h16" stroke="#f5c542" stroke-width="1.8" ${S}/>` +
    '<g fill="#f5c542"><circle cx="9" cy="6" r="2.4"/><circle cx="15" cy="12" r="2.4"/><circle cx="8" cy="18" r="2.4"/></g>',
  git:
    '<path d="M12 1.5 22.5 12 12 22.5 1.5 12z" fill="#f0573f"/>' +
    `<path d="M9 9v6.5M9.5 9.5l4.5 4.2" stroke="#fff" stroke-width="1.5" ${S}/>` +
    '<g fill="#fff"><circle cx="9" cy="8" r="1.7"/><circle cx="9" cy="16.5" r="1.7"/><circle cx="15" cy="14.5" r="1.7"/></g>',
  rspec:
    '<circle cx="12" cy="12" r="10" fill="#d63d38"/>' +
    '<path d="M12 17.5s-5.5-3.3-5.5-7a2.8 2.8 0 0 1 5.5-.9 2.8 2.8 0 0 1 5.5.9c0 3.7-5.5 7-5.5 7z" fill="#fff"/>',
  ts: badge("#3178c6", "#fff", "TS"),
  js: badge("#f0db4f", "#2b2826", "JS"),
  json:
    '<text x="12" y="18" text-anchor="middle" font-family="var(--mono)" font-size="17" font-weight="700" fill="#f5c542">{ }</text>',
  css: `<path d="M9.5 3 7.5 21M16.5 3l-2 18M4 9h17M3 15h17" stroke="#42a5f5" stroke-width="2.2" ${S}/>`,
  html: `<path d="m8 7-5 5 5 5M16 7l5 5-5 5" stroke="#e44d26" stroke-width="2.2" ${S}/>`,
  rust:
    `<circle cx="12" cy="12" r="4" stroke="#dea584" stroke-width="2.2" ${S}/>` +
    `<path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1" stroke="#dea584" stroke-width="2.4" ${S}/>`,
  toml:
    `<path d="M4 4h4M4 4v16M20 4h-4M20 4v16" stroke="#c99a6e" stroke-width="2" ${S}/>` +
    '<text x="12" y="17" text-anchor="middle" font-family="var(--font)" font-size="12" font-weight="800" fill="#c99a6e">T</text>',
  lock:
    `<rect x="5" y="11" width="14" height="10" rx="2" stroke="#a4a09d" stroke-width="1.8" ${S}/>` +
    `<path d="M8 11V7a4 4 0 0 1 8 0v4" stroke="#a4a09d" stroke-width="1.8" ${S}/>`,
  image:
    `<rect x="3" y="3" width="18" height="18" rx="2" stroke="#b47aea" stroke-width="1.8" ${S}/>` +
    `<circle cx="9" cy="9" r="2" stroke="#b47aea" stroke-width="1.8" ${S}/>` +
    `<path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21" stroke="#b47aea" stroke-width="1.8" ${S}/>`,
  sh: `<path d="m4 7 5 5-5 5M12 17h8" stroke="#4caf50" stroke-width="2.2" ${S}/>`,
  py: badge("#3572a5", "#ffd43b", "Py"),
};

const BY_NAME: Record<string, string> = {
  gemfile: "ruby", rakefile: "ruby", "config.ru": "ruby", ".ruby-version": "ruby",
  dockerfile: "docker", ".dockerignore": "docker",
  ".rspec": "rspec",
  ".gitignore": "git", ".gitattributes": "git", ".gitmodules": "git",
  "package-lock.json": "lock", "gemfile.lock": "lock", "cargo.lock": "lock", "yarn.lock": "lock", "pnpm-lock.yaml": "lock",
};
const BY_EXT: Record<string, string> = {
  rb: "ruby", erb: "html", yml: "yaml", yaml: "yaml", md: "md", markdown: "md",
  ts: "ts", tsx: "ts", mts: "ts", js: "js", jsx: "js", mjs: "js", cjs: "js",
  json: "json", css: "css", html: "html", htm: "html", rs: "rust", toml: "toml", lock: "lock",
  png: "image", jpg: "image", jpeg: "image", gif: "image", webp: "image", svg: "image", ico: "image", icns: "image",
  sh: "sh", zsh: "sh", bash: "sh", py: "py",
};

export function fileKind(name: string): string | null {
  const lower = name.toLowerCase();
  if (BY_NAME[lower]) return BY_NAME[lower];
  if (lower.startsWith(".env")) return "env";
  const ext = lower.includes(".") ? lower.slice(lower.lastIndexOf(".") + 1) : "";
  return BY_EXT[ext] ?? null;
}

export function fileIcon(name: string, size = 16): string {
  const kind = fileKind(name);
  if (!kind) return icon("file", size);
  return `<svg class="ic" width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true">${FILE_ICONS[kind]}</svg>`;
}

/* Provider branding. */

/// Use official Claude and OpenAI marks so providers remain recognizable in workspace lists and usage indicators.
const BRANDS: Record<string, string> = {
  antigravity: '<path fill="#4285f4" d="M12 1C10.5 7.5 7.5 10.5 1 12c6.5 1.5 9.5 4.5 11 11 1.5-6.5 4.5-9.5 11-11-6.5-1.5-9.5-4.5-11-11Z"/>',
  claude: `<image href="${CLAUDE_MARK}" width="24" height="24"/>`,
  codex:
    '<path fill="#a4a09d" d="M22.282 9.821a5.985 5.985 0 0 0-.516-4.911 6.046 6.046 0 0 0-6.51-2.9A6.065 6.065 0 0 0 4.98 4.182a5.985 5.985 0 0 0-3.998 2.9 6.046 6.046 0 0 0 .743 7.097 5.98 5.98 0 0 0 .51 4.91 6.051 6.051 0 0 0 6.515 2.9A5.985 5.985 0 0 0 13.26 24a6.056 6.056 0 0 0 5.772-4.206 5.99 5.99 0 0 0 3.998-2.9 6.056 6.056 0 0 0-.748-7.073zm-9.022 12.608a4.476 4.476 0 0 1-2.876-1.04l.142-.081 4.778-2.758a.795.795 0 0 0 .393-.681v-6.737l2.02 1.168a.071.071 0 0 1 .038.052v5.583a4.504 4.504 0 0 1-4.495 4.494zM3.6 18.305a4.471 4.471 0 0 1-.535-3.014l.142.085 4.783 2.758a.771.771 0 0 0 .78 0l5.843-3.368v2.332a.08.08 0 0 1-.033.062L9.74 19.95a4.499 4.499 0 0 1-6.14-1.646zM2.34 7.896a4.485 4.485 0 0 1 2.366-1.973V11.6a.766.766 0 0 0 .388.677l5.814 3.354-2.02 1.168a.076.076 0 0 1-.071 0l-4.83-2.786A4.504 4.504 0 0 1 2.34 7.872zm16.597 3.856-5.834-3.388L15.119 7.2a.076.076 0 0 1 .071 0l4.83 2.791a4.494 4.494 0 0 1-.676 8.105v-5.678a.79.79 0 0 0-.407-.666zm2.01-3.023-.141-.085-4.774-2.782a.776.776 0 0 0-.785 0L9.41 9.23V6.897a.066.066 0 0 1 .028-.061l4.83-2.787a4.499 4.499 0 0 1 6.68 4.66zM8.307 12.863l-2.02-1.164a.08.08 0 0 1-.038-.057V6.074a4.499 4.499 0 0 1 7.375-3.454l-.142.08-4.778 2.76a.795.795 0 0 0-.393.68zm1.097-2.366 2.602-1.5 2.607 1.5v3l-2.597 1.5-2.607-1.5z"/>',
};

export function brand(name: string, size = 14): string {
  const art = BRANDS[name];
  if (!art) return icon("sparkles", size);
  return `<svg class="ic" width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true">${art}</svg>`;
}
