import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";

const root = process.cwd();
const errors = [];
const types = {
  architecture: "Architecture",
  contracts: "Contract",
  decisions: "Decision",
  operations: "Operations",
  quality: "Quality",
};

function checkMetadata(file) {
  const name = relative(root, file);
  const text = readFileSync(file, "utf8");
  const header = text.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  if (!header) {
    errors.push(`${name}: missing YAML frontmatter`);
    return;
  }
  const fields = new Map();
  // ponytail: four single-line YAML strings only; use a YAML parser if richer metadata becomes necessary.
  for (const line of header[1].split(/\r?\n/)) {
    const field = line.match(/^(type|title|description|status): +(.+)$/);
    if (!field || fields.has(field[1])) {
      errors.push(`${name}: unknown, malformed or duplicate metadata field: ${line}`);
      continue;
    }
    const raw = field[2].trim();
    let value;
    try {
      if (raw.startsWith('"')) value = JSON.parse(raw);
      else if (/^'(?:[^']|'')*'$/.test(raw)) value = raw.slice(1, -1).replaceAll("''", "'");
      else if (/^\p{L}/u.test(raw) && !/:(?:\s|$)|\s#/.test(raw)
        && !/^(true|false|null|yes|no|on|off)$/i.test(raw)) value = raw;
    } catch {
      // Invalid quoted values are reported by the same string check below.
    }
    if (typeof value !== "string" || !value.trim() || /[\r\n]/.test(value)) {
      errors.push(`${name}: ${field[1]} must be a nonempty single-line YAML string`);
    }
    fields.set(field[1], value);
  }
  for (const key of ["type", "title", "description", "status"]) {
    if (!fields.has(key)) errors.push(`${name}: missing metadata field: ${key}`);
  }
  const area = relative(join(root, "docs"), file).split(sep)[0];
  if (!Object.hasOwn(types, area) || fields.get("type") !== types[area]) {
    errors.push(`${name}: type must match its bundle area (${types[area] ?? "unknown area"})`);
  }
  if (!["draft", "stable", "deprecated"].includes(fields.get("status"))) {
    errors.push(`${name}: status must be draft, stable or deprecated`);
  }
  if (/^Status:/m.test(text.slice(header[0].length))) {
    errors.push(`${name}: use only YAML status, not a second Status field in the body`);
  }
}

function markdownUnder(dir) {
  const files = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) files.push(...markdownUnder(path));
    else if (name.endsWith(".md")) files.push(path);
  }
  return files;
}

const roots = ["README.md", "CONTRIBUTING.md", "AGENTS.md", "CLAUDE.md", "ARCHITECTURE.md"]
  .map((file) => join(root, file))
  .filter(existsSync);
const docs = markdownUnder(join(root, "docs"));
const files = [...roots, ...docs];
const links = new Map();

for (const file of files) {
  const text = readFileSync(file, "utf8");
  const targets = new Set();
  links.set(file, targets);
  for (const match of text.matchAll(/!?\[[^\]]*\]\(([^)]+)\)/g)) {
    let target = match[1].trim();
    if (target.startsWith("<") && target.endsWith(">")) target = target.slice(1, -1);
    if (!target || target.startsWith("#") || /^[a-z][a-z+.-]*:/i.test(target)) continue;

    target = target.split("#", 1)[0];
    try {
      target = decodeURIComponent(target);
    } catch {
      errors.push(`${relative(root, file)}: invalid link escape: ${match[1]}`);
      continue;
    }

    const path = resolve(dirname(file), target);
    if (!match[0].startsWith("!")) targets.add(path);
    if (!existsSync(path)) {
      const line = text.slice(0, match.index).split("\n").length;
      errors.push(`${relative(root, file)}:${line}: target does not exist: ${match[1]}`);
    }
  }
}

const bundleIndex = join(root, "docs", "README.md");
for (const file of docs) {
  if (file === bundleIndex) continue;
  const index = basename(file) === "README.md" ? bundleIndex : join(dirname(file), "README.md");
  if (!links.get(index)?.has(file)) {
    errors.push(`${relative(root, index)}: does not index ${relative(root, file)}`);
  }
  if (basename(file) !== "README.md") checkMetadata(file);
}

const agents = readFileSync(join(root, "AGENTS.md"), "utf8");
const agentLines = agents.split("\n").length;
if (agentLines > 200) errors.push(`AGENTS.md: ${agentLines} lines; the short-map limit is 200`);

const claude = readFileSync(join(root, "CLAUDE.md"), "utf8");
if (!claude.includes("@AGENTS.md")) errors.push("CLAUDE.md: does not import @AGENTS.md");

if (errors.length) {
  console.error(errors.join("\n"));
  process.exitCode = 1;
} else {
  console.log(`${files.length} valid documents and ${docs.length - 1} indexed pages`);
}
