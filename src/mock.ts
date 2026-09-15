import type { IpcCommand, IpcHandlers } from "./ipc";
import { emptyCatalog, initializeDefaults, type Catalog, type Profile } from "./actions";
/// Browser backend for sample data. Loaded only when window.__TAURI_INTERNALS__ is absent; never loaded in Tauri.
import { simulatedSocket } from "./team-mock";
import * as browser from "./mock-browser";
import type { Share } from "../relay/src/protocol";
import { parseConversationEvent } from "./conversation";
import { LegacyConversationAdapter } from "./conversation-legacy";
import * as team from "./team";
import type { Accounts } from "./statusbar";
import type { CloudStatus } from "./cloud";
import type { CatalogState, Kind } from "./catalog";
import type { Skill } from "./skills";
import { hasWorktree, type Board, type Change, type Choice, type DockKind, type EffectiveItem, type GitBranch, type GitCommit, type GitConflict, type GitFile, type GitStatus, type Issue, type LinearStatus, type McpServer, type Plugin, type ProjectTools, type Pr, type Provenance, type Scripts, type Selection, type Tab, type Tools, type Workspace } from "./types";

type Handler = (e: { event: string; id: number; payload: unknown }) => void;
const handlers = new Map<string, Handler[]>();
let nextId = 1;
let cloudPending: string | null = null;
const emptyCloud = (): CloudStatus => ({ user: null, origin: "https://app.prometeu.co", offline: false });
const mockCloud = (): CloudStatus => JSON.parse(localStorage.getItem("mock:cloud") ?? "null") ?? emptyCloud();
let skillHub: Skill[] = JSON.parse(localStorage.getItem("mock:skills") ?? "[]");
const mockCatalog = (): CatalogState => JSON.parse(localStorage.getItem("mock:catalog") ?? "null") ?? {
  connected: true, revision: 0,
  plugins: [
    { id: "caveman", source: "https://github.com/JuliusBrussee/caveman", note: "fala curto e sem enfeite", local_id: "caveman", installed: true, source_changed: false },
    { id: "revisor", source: "https://github.com/prometeu/revisor", note: "revisa PR", local_id: "revisor", installed: false, source_changed: false },
  ],
  mcp: ["notion"],
  skills: [{ id: "revisao-cloud", description: "Revisar alterações", content: "Leia o diff e relate bugs.", local_id: "revisao-cloud", installed: false }],
  shared: { "plugins:caveman": "caveman", "plugins:revisor": "revisor", "mcp:notion": "notion", "skills:revisao-cloud": "revisao-cloud" },
};
type MockOrganizationCatalog = { id: string; name: string; revision?: number; plugins: Pick<Plugin, "id" | "source" | "note">[]; mcp: McpServer[]; skills: Skill[]; links: Record<string, string> };
const mockOrganizations = (): MockOrganizationCatalog[] => JSON.parse(localStorage.getItem("mock:organizationCatalogs") ?? "[]");
function cloudWrite() {
  if (!mockCloud().user) throw 'i18n:{"code":"err.catalog.disconnected"}';
  if (localStorage.getItem("mock:cloudOffline")) throw 'i18n:{"code":"err.cloud.network"}';
}
function saveMockCatalog(value: CatalogState) { value.revision = (value.revision ?? 0) + 1; localStorage.setItem("mock:catalog", JSON.stringify(value)); emit("catalog", null); }
function saveMockSkill(skill: Skill) {
  if (!/^[a-z0-9][a-z0-9-]{0,55}$/.test(skill.id) || !skill.description.trim() || skill.description.length > 2000 || !skill.content.trim() || new TextEncoder().encode(skill.content).length > 65536) throw 'i18n:{"code":"err.catalog.invalid"}';
  skillHub = [...skillHub.filter(s => s.id !== skill.id), skill];
  localStorage.setItem("mock:skills", JSON.stringify(skillHub));
  const id = `skill-${skill.id}`;
  pluginHub = [...pluginHub.filter(p => p.id !== id), { id, source: `~/.prometeu/skills-packages/${skill.id}`, note: skill.description, made: false }];
}
const w = window as unknown as Record<string, unknown>;

const accountDefaults: Accounts = {
  accounts: [
    { id: "claude", provider: "claude", email: "pessoal@exemplo.com", plan: "max", connected: true, revision: 0 },
    { id: "codex", provider: "codex", email: "pessoal@exemplo.com", plan: "pro", connected: true, revision: 0 },
    { id: "09317ab6-22c1-45bb-882e-f6fef6a44c09", provider: "claude", email: "trabalho@exemplo.com", plan: "max", connected: true, revision: 0 },
    { id: "086eb684-2c61-421b-a3e5-80e54bc26a53", provider: "codex", email: "trabalho@exemplo.com", plan: "pro", connected: true, revision: 0 },
  ],
  active: { claude: "claude", codex: "codex" },
  login: null,
};
const mockAccounts: Accounts = JSON.parse(localStorage.getItem("mock:accounts") ?? "null") ?? accountDefaults;
mockAccounts.login = null;
let cancelAccountLogin: (() => void) | null = null;
function accountSnapshot() {
  const snapshot = structuredClone(mockAccounts);
  localStorage.setItem("mock:accounts", JSON.stringify({ ...snapshot, login: null }));
  emit("accounts", snapshot);
  return snapshot;
}

const ws = (
  id: string,
  project: string,
  repo: string,
  title: string,
  stage: string,
  tabs: Workspace["tabs"],
): Workspace => ({
  id,
  title,
  project,
  repo: `/Users/gustavo/dev/${repo}`,
  repo_name: repo,
  branch: `prometeu/${id}`,
  worktree: `~/prometeu/worktrees/${repo}/prometeu-${id}`,
  repos: [{ path: `/Users/gustavo/dev/${repo}`, name: repo, worktree: `~/prometeu/worktrees/${repo}/prometeu-${id}`, base: "origin/main", pr: null }],
  stage,
  archived: false,
  pinned: false,
  unread: false,
  agent: "claude",
  // Sample model and effort keep the composer footer visible.
  model: "opus[1m]",
  effort: "high",
  mcp: null,
  plugins: null,
  skills: null,
  port: 3100,
  issue: null,
  cleaned: false,
  shared: false,
  audience: null,
  remote_control: false,
  preparing: false,
  failed: null,
  remote: null,
  tabs,
  active: tabs[0]?.id ?? null,
});

const board: Board = {
  actions: initializeDefaults(JSON.parse(localStorage.getItem("mock:actions") ?? "null") ?? emptyCatalog()),
  stages: ["Preparando", "Fazendo", "Code review", "Travado", "Feito"],
  projects: [
    { id: "p1", name: "njord", path: "/Users/gustavo/dev/njord" },
    { id: "p2", name: "prometeu", path: "/Users/gustavo/dev/prometeu" },
  ],
  workspaces: [
    ws("sessao-0929", "p1", "njord", "Ola", "Fazendo", [
      { id: "t1", title: "", status: "pronta", note: null, tokens: 57_000 },
      // This tab overrides the workspace model so its footer differs from sibling tabs.
      { id: "t2", title: "", status: "pronta", note: null, tokens: 112_400, pending_prompt: "O que tem nesse projeto aqui de legal?", choice: { agent: "claude", model: "sonnet", effort: "medium" } },
    ]),
    ws("ui-2231", "p2", "prometeu", "Tela igual ao Conductor", "Fazendo", [
      { id: "t3", title: "", status: "rodando", note: "Edit src/style.css", tokens: 23_800 },
    ]),
    // Two repositories on one branch exercise separate change sections.
    Object.assign(
      ws("portal-1217", "p2", "prometeu", "Contratação pelo portal", "Fazendo", [
        { id: "t9", title: "", status: "rodando", note: "Edit app/models/entry.rb", tokens: 31_000 },
      ]),
      {
        worktree: "~/prometeu/worktrees/prometeu+njord/prometeu-portal-1217",
        // Only one repository's PR is merged, so the workspace cannot be marked complete.
        repos: [
          {
            path: "/Users/gustavo/dev/prometeu",
            name: "prometeu",
            worktree: "~/prometeu/worktrees/prometeu+njord/prometeu-portal-1217/prometeu",
            base: "origin/main",
            pr: { number: 51, title: "feat(portal): contratação pelo portal", isDraft: false, state: "OPEN" },
          },
          {
            path: "/Users/gustavo/dev/njord",
            name: "njord",
            worktree: "~/prometeu/worktrees/prometeu+njord/prometeu-portal-1217/njord",
            base: "origin/develop",
            pr: { number: 12, title: "feat: origem da entrada", isDraft: false, state: "MERGED" },
          },
        ],
      },
    ),
    // An unanswered question marks this conversation unread.
    Object.assign(
      ws("icone-2140", "p2", "prometeu", "Ícone do app", "Code review", [
        { id: "t4", title: "", status: "querendo", note: "Qual tamanho de ícone você quer gerar?", tokens: 8_100 },
      ]),
      { unread: true, pr: { number: 42, title: "feat(quadro): ícone do app", isDraft: false, state: "OPEN" } },
    ),
    // A merged PR exposes the card badge and completion action.
    Object.assign(
      ws("dock-1130", "p2", "prometeu", "Porta do dock por worktree", "Code review", [
        { id: "t5", title: "", status: "pronta", note: null, tokens: 44_200 },
      ]),
      { pr: { number: 40, title: "feat(dock): porta por worktree", isDraft: false, state: "MERGED" } },
    ),
    // This archived workspace still occupies disk space and appears in cleanup.
    Object.assign(
      ws("linear-0912", "p1", "njord", "Conectar o Linear", "Feito", [
        { id: "t7", title: "", status: "desligada", note: null, tokens: 66_000 },
      ]),
      { archived: true, pr: { number: 8, title: "feat: conectar o Linear", isDraft: false, state: "MERGED" } },
    ),
    Object.assign(
      ws("porta-1751", "p1", "njord", "Porta ocupada no setup", "Travado", [
        { id: "t8", title: "", status: "desligada", note: null, tokens: 12_000 },
      ]),
      { archived: true },
    ),
    // Keep the completed workspace card after its worktree is removed.
    Object.assign(
      ws("idioma-1348", "p2", "prometeu", "O app fala inglês", "Feito", [
        { id: "t6", title: "", status: "desligada", note: null, tokens: 91_000 },
      ]),
      {
        archived: true,
        cleaned: true,
        pr: { number: 17, title: "feat(idioma): o app fala inglês", isDraft: false, state: "MERGED" },
      },
    ),
  ],
};

// Normalize sample PRs onto the primary repository, matching backend legacy loading.
for (const w of board.workspaces as (Workspace & { pr?: Pr | null })[]) {
  if (w.pr) w.repos[0].pr = w.pr;
  delete w.pr;
}

const tree: Record<string, { name: string; path: string; dir: boolean }[]> = {
  "": [
    ...[".github", "app", "bin", "config", "db", "docs", "lib", "log", "public", "script", "spec", "storage", "tmp", "vendor"].map(
      (name) => ({ name, path: name, dir: true }),
    ),
    ...[".dockerignore", ".env.example", ".gitignore", ".rspec", ".rubocop.yml", ".ruby-version", "CLAUDE.md", "Dockerfile", "Gemfile", "Gemfile.lock", "README.md", "config.ru"].map(
      (name) => ({ name, path: name, dir: false }),
    ),
  ],
  app: ["adapters", "assets", "channels", "controllers", "helpers", "javascript", "jobs", "models", "views"].map((name) => ({
    name,
    path: `app/${name}`,
    dir: true,
  })),
  "app/adapters": ["transcriber.rb", "waha.rb"].map((name) => ({ name, path: `app/adapters/${name}`, dir: false })),
  bin: ["brakeman", "ci", "dev", "rails", "rake", "rubocop", "setup"].map((name) => ({ name, path: `bin/${name}`, dir: false })),
  docs: [
    { name: "clientes.csv", path: "docs/clientes.csv", dir: false },
    { name: "regras.pdf", path: "docs/regras.pdf", dir: false },
  ],
};

const files: Record<string, string> = {
  "docs/regras.pdf":
    "%PDF-1.1\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R>>",
  // Brazilian Excel format: semicolon delimiters, decimal commas, and a multiline field.
  "docs/clientes.csv": [
    "id;nome;cidade;total",
    ...Array.from({ length: 3000 }, (_, i) => `${i + 1};"Cliente ${i + 1}";São Paulo;${i * 7},50`),
    '3001;"Nome, com vírgula";"Rio de\nJaneiro";0,00',
  ].join("\n"),
  "app/adapters/transcriber.rb": `class Transcriber
  MODEL = "gemini-3.6-flash".freeze

  class << self
    def call(audio, extension:)
      Tempfile.create([ "audio", extension ]) do |file|
        file.binmode
        file.write(audio)
        file.flush

        transcription(file.path)
      end
    end

    private
      def transcription(path)
        RubyLLM.transcribe(
          path, model: MODEL, provider: :gemini, assume_model_exists: true, language: "portuguese"
        ).text
      end
  end
end
`,
  "CLAUDE.md": "# Njord\n\nControle financeiro pessoal em Rails.\n\n## Regras\n\n- Competência é o dia em que o dinheiro **saiu**.\n- Rodar `bin/ci` antes de abrir PR. Ver [docs](docs/README.md).\n",
  ".gitignore": "# Ignore bundler config.\n/.bundle\n/log/*\n!/log/.keep\n/tmp/*\n",
  "src/style.css": ".card {\n  display: flex;\n  gap: 8px;\n  padding: 12px;\n}\n",
  Dockerfile: "# syntax=docker/dockerfile:1\nFROM ruby:3.4-slim AS base\nWORKDIR /rails\nENV RAILS_ENV=production\nRUN apt-get update -qq && apt-get install -y curl\nCMD [\"bin/rails\", \"server\"]\n",
  ".rubocop.yml": "# Omakase Ruby styling for Rails\ninherit_gem: { rubocop-rails-omakase: rubocop.yml }\n\nAllCops:\n  TargetRubyVersion: 3.4\n  NewCops: enable\n",
};

/// Changes in the second repository exercise independent history for the same feature.
const changes2 = [
  {
    path: "app/models/entry.rb",
    added: 5,
    removed: 1,
    new_file: false, deleted: false, dirty: true,
    patch: [
      "@@ -4,7 +4,11 @@ class Entry < ApplicationRecord",
      "   belongs_to :category",
      "-  validates :amount, presence: true",
      "+  validates :amount, presence: true, numericality: { greater_than: 0 }",
      "+",
      "+  def portal?",
      '+    source == "portal"',
      "+  end",
      " ",
      "   scope :month, ->(m) { where(date: m.beginning_of_month..m.end_of_month) }",
      " end",
    ].join("\n"),
  },
  {
    path: "db/migrate/20260829120000_add_source_to_entries.rb",
    added: 5,
    removed: 0,
    new_file: true, deleted: false, dirty: false,
    patch: [
      "@@ -0,0 +1,5 @@",
      "+class AddSourceToEntries < ActiveRecord::Migration[7.1]",
      "+  def change",
      "+    add_column :entries, :source, :string",
      "+  end",
      "+end",
    ].join("\n"),
  },
];

const changes = [
  {
    path: "src/style.css",
    added: 6,
    removed: 2,
    new_file: false, deleted: false, dirty: true,
    patch: [
      "@@ -212,7 +212,11 @@ .card {",
      "   display: flex;",
      "-  gap: 4px;",
      "-  padding: 8px;",
      "+  gap: 8px;",
      "+  padding: 12px;",
      "+  border-radius: var(--r-lg);",
      " }",
      "@@ -318,3 +322,6 @@ .foot {",
      " .foot .x:hover { color: var(--err); }",
      "+.foot .chip { padding: 0; }",
      "+.foot .chip .dot { width: 7px; }",
      "+",
    ].join("\n"),
  },
  {
    path: "src/icons.ts",
    added: 4,
    removed: 0,
    new_file: true, deleted: false, dirty: false,
    patch: [
      "@@ -0,0 +1,4 @@",
      '+export function icon(name: string, size = 16): string {',
      '+  const PATHS: Record<string, string> = { plus: "M5 12h14" };',
      "+  return `<svg width=\"${size}\">${PATHS[name]}</svg>`;",
      "+}",
    ].join("\n"),
  },
  { path: "public/logo.png", added: 0, removed: 0, new_file: true, deleted: false, dirty: true, patch: "" },
];

type MockGit = {
  status: GitStatus;
  staged: Change[];
  changes: Change[];
  compare: Change[];
  commits: (GitCommit & { files: Change[] })[];
  conflicts: Record<string, GitConflict>;
  version: number;
};
const gitStates = new Map<string, MockGit>();
const gitError = (code: string): never => { throw `i18n:${JSON.stringify({ code })}`; };
const gitPatch = (file: Change, patch: string): Change => ({
  ...file, patch,
  added: patch.split("\n").filter((line) => line.startsWith("+")).length,
  removed: patch.split("\n").filter((line) => line.startsWith("-")).length,
});
// Sample hunks are independent. The mock combines them when staging; arbitrary patch application requires real Git tests.
function mergeGitFile(target: Change[], file: Change, prepend = false) {
  const at = target.findIndex((current) => current.path === file.path);
  if (at < 0) target.push({ ...file });
  else target[at] = gitPatch(file, (prepend ? [file.patch, target[at].patch] : [target[at].patch, file.patch]).filter(Boolean).join("\n"));
}
function gitState(id: string, index = 0): MockGit {
  const workspace = board.workspaces.find((workspace) => workspace.id === id && !workspace.cleaned);
  const repo = workspace?.repos[index];
  if (!workspace || !repo) return gitError("err.session.noWorkspace");
  const key = `${id}:${index}`;
  let value = gitStates.get(key);
  if (!value) {
    const sample = structuredClone(index === 0 ? changes : changes2);
    const local = sample.filter((file) => file.dirty);
    const staged: Change[] = [];
    if (local[0]?.patch) {
      const hunks = local[0].patch.split(/(?=^@@)/m);
      staged.push(gitPatch(local[0], hunks[0]));
      if (hunks.length > 1) local[0] = gitPatch(local[0], hunks.slice(1).join(""));
      else local.shift();
    }
    const upstream = id === "ui-2231" ? null : `origin/${workspace.branch}`;
    const committed = sample.filter((file) => !file.dirty);
    value = {
      status: {
        repo: index, name: repo.name, branch: workspace.branch, base: repo.base,
        upstream, remotes: ["origin"], ahead: upstream && index === 0 ? 1 : 0,
        behind: 0, has_head: true, merging: false, index: "mock-0", staged: [], changes: [], conflicts: [], error: null,
      },
      staged, changes: local, compare: committed,
      commits: [
        { oid: "2".padStart(40, "0"), subject: "feat: atualiza projeto", author: "Gustavo", date: "2026-09-05T09:00:00-03:00", outgoing: !!upstream && index === 0, files: structuredClone(committed) },
        { oid: "1".padStart(40, "0"), subject: "chore: inicia projeto", author: "Gustavo", date: "2026-09-04T09:00:00-03:00", outgoing: false, files: [] },
      ],
      conflicts: {}, version: 0,
    };
    if (id === "icone-2140" && index === 0) {
      value.status.merging = true;
      const path = sample[0].path;
      value.staged = value.staged.filter((file) => file.path !== path);
      value.changes = value.changes.filter((file) => file.path !== path);
      value.conflicts[path] = {
        current: ".card {\n<<<<<<< HEAD\n  gap: 8px;\n=======\n  gap: 12px;\n>>>>>>> origin/main\n}\n",
        ours: ".card {\n  gap: 8px;\n}\n",
        theirs: ".card {\n  gap: 12px;\n}\n",
      };
    }
    gitStates.set(key, value);
  }
  return value;
}
function gitStatus(value: MockGit): GitStatus {
  const file = (file: Change, staged: boolean): GitFile => ({ path: file.path, status: file.deleted ? "D" : file.new_file ? staged ? "A" : "?" : "M" });
  return structuredClone({
    ...value.status,
    staged: value.staged.map((entry) => file(entry, true)),
    changes: value.changes.map((entry) => file(entry, false)),
    conflicts: Object.keys(value.conflicts).map((path) => ({ path, status: "U" })),
  });
}

/// Legacy transcript fixture for compatibility. New mock events become V1 before reaching the UI or relay.
const line = (o: unknown) => JSON.stringify(o);
const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();
const SAMPLE =
  [
    // Simulate initialization and first-message metadata; terminal-only commands are filtered out.
    line({
      type: "control_response",
      response: {
        subtype: "success",
        request_id: "initialize",
        response: {
          commands: [
            { name: "compact", description: "Free up context by summarizing the conversation so far", argumentHint: "<optional custom summarization instructions>" },
            { name: "context", description: "Show the context usage of the current session", argumentHint: "" },
            { name: "clear", description: "Clear conversation history and free up context", argumentHint: "[name]" },
            { name: "cost", description: "Show the total cost and duration of the current session", argumentHint: "" },
            { name: "color", description: "Set the color of the session", argumentHint: "" },
            { name: "open-pr", description: "Abre um PR da branch atual — empurra, escreve título e corpo e acompanha os checks até o fim (project)", argumentHint: "" },
            { name: "release", description: "Solta uma versão nova do Prometeu — confere os commits, corta a tag, acompanha o CI e publica a draft (project)", argumentHint: "" },
            { name: "caveman:caveman", description: "(caveman) Ultra-compressed communication mode. Cuts token usage ~75% by speaking like caveman while keeping full technical accuracy.", argumentHint: "" },
            { name: "caveman:caveman-commit", description: "(caveman) Ultra-compressed commit message generator. Cuts noise from commit messages while preserving intent and reasoning.", argumentHint: "" },
          ],
        },
      },
    }),
    line({ type: "system", subtype: "init", slash_commands: ["compact", "context", "clear", "cost", "color"], terminal_slash_commands: ["color"] }),
    line({ type: "user", message: { role: "user", content: "Me pergunte quais são as minhas 3 cores preferidas" }, timestamp: ago(12) }),
    line({ type: "assistant", message: { id: "m0", role: "assistant", content: [{ type: "thinking", thinking: "Pergunta simples. Vou perguntar direto." }] }, timestamp: ago(12) }),
    line({ type: "assistant", message: { id: "m0", role: "assistant", content: [{ type: "text", text: "Quais são as suas **três** cores preferidas?" }] }, timestamp: ago(12) }),
    line({ type: "user", message: { role: "user", content: "verde" }, timestamp: ago(10) }),
    line({ type: "assistant", message: { id: "m1", role: "assistant", content: [{ type: "tool_use", id: "tu1", name: "Bash", input: { command: "ls -la", description: "Lista os arquivos" } }] }, timestamp: ago(10) }),
    line({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "tu1", content: "total 0\n.env\nREADME.md\napp/" }] }, timestamp: ago(10) }),
    line({ type: "assistant", message: { id: "m1", role: "assistant", content: [{ type: "tool_use", id: "tu2", name: "Edit", input: { file_path: "app/models/todo.rb", old_string: "  def complete!\n    destroy\n  end", new_string: "  def complete!\n    update!(completed_at: Time.current)\n  end" } }] }, timestamp: ago(9) }),
    line({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "tu2", content: "The file app/models/todo.rb has been updated." }] }, timestamp: ago(9) }),
    line({ type: "assistant", message: { id: "m1", role: "assistant", content: [{ type: "text", text: "Verde anotado. Só uma das três — quer dizer as outras duas?\n\n```ruby\ndef complete!\n  update!(completed_at: Time.current)\nend\n```" }] }, timestamp: ago(9) }),
    line({ type: "result", subtype: "success", is_error: false, duration_ms: 5000 }),
  ].join("\n") + "\n";

/// One workspace has setup and two run scripts; another has none to exercise the add-script prompt.
const scripts: Record<string, Scripts> = {
  "sessao-0929": {
    file: ".conductor/settings.toml",
    inherited: false,
    setup: "bin/setup",
    runs: [
      { name: "web", command: "bin/dev --port $PROMETEU_PORT" },
      { name: "worker", command: "bin/jobs" },
    ],
    archive: null,
    copy: [".env", "config/master.key"],
    port: 3100,
  },
  "ui-2231": {
    file: ".prometeu/settings.toml",
    inherited: true,
    setup: "npm install",
    runs: [{ name: "run", command: "npm run dev -- --port $PROMETEU_PORT" }],
    archive: null,
    copy: [".env"],
    port: 3110,
  },
};
const noScripts: Scripts = { file: null, inherited: false, setup: null, runs: [], archive: null, copy: [], port: 3120 };

/// Representative project `[tools]` declarations for the browser, keyed by workspace or project id.
/// The browser cannot read a repository, so this stands in for the primary repository's declaration;
/// `pending` and `decision` are recomputed at call time from `board.tool_trust`.
const projectTools: Record<string, ProjectTools> = {
  "ui-2231": {
    repo: "https://github.com/prometeucorp/prometeu.git",
    file: ".prometeu/settings.toml",
    hash: "9f1c-demo-hash",
    tools: { mcp: null, plugins: { base: "none", add: ["ponytail"], remove: [] }, skills: null },
    pending: true,
    decision: null,
  },
};

/// The empty declaration returned for a repository that declares no `[tools]`.
const noProjectTools: ProjectTools = {
  repo: "",
  file: null,
  hash: "",
  tools: { mcp: null, plugins: null, skills: null },
  pending: false,
  decision: null,
};

/// Mirror of selection::compose: apply the layers in order, keeping only universe IDs. Within a
/// layer `add` comes first and `remove` has the last word. See src-tauri/src/selection.rs.
function composeAxis(layers: (Selection | null)[], universe: string[]): string[] {
  let ids: string[] = [];
  for (const layer of layers) {
    if (!layer) continue;
    if (layer.base === "none") ids = [];
    for (const id of layer.add) if (!ids.includes(id)) ids.push(id);
    ids = ids.filter((id) => !layer.remove.includes(id));
  }
  return ids.filter((id) => universe.includes(id));
}

/// Mirror of selection::resolve_with_base: the CLI-inherited base (ADR 0044) seeds the chain as an
/// implicit inherit layer below the global one. An empty base reduces to plain resolution.
function resolveWithBase(
  base: string[],
  global: Selection | null,
  project: Selection | null,
  workspace: Selection | null,
  universe: string[],
): string[] {
  const seed: Selection | null = base.length ? { base: "inherit", add: [...base], remove: [] } : null;
  return composeAxis([seed, global, project, workspace], universe);
}

/// Mirror of the backend `Gate`: whether the declared project layer may activate (ADR 0043).
type Gate = "trusted" | "pending" | "rejected";

/// Mirror of tool_roots + project_declaration for the browser: fixtures are keyed by workspace id,
/// and a project id resolves through the declaration stored under one of its workspaces.
function declaredFor(id: string): ProjectTools {
  if (projectTools[id]) return projectTools[id];
  const viaProject = board.workspaces.find((w) => w.project === id && projectTools[w.id]);
  return viaProject ? projectTools[viaProject.id] : noProjectTools;
}

/// Mirror of gate_of: a decision binds to the current hash; a rejection quiets the prompt but keeps
/// the declaration gated, and a changed hash re-pends until the person decides again.
function gateOf(declared: ProjectTools): Gate {
  if (!declared.hash) return "trusted";
  const decision = (board.tool_trust ?? []).find((t) => t.repo === declared.repo && t.hash === declared.hash);
  if (!decision) return "pending";
  return decision.approved ? "trusted" : "rejected";
}

/// The shared layer-resolution inputs for one workspace: the declared project layer (ungated), the
/// global layer, the workspace's own triple, and the mcp base/universe (ADR 0044).
function toolLayers(ws: Workspace) {
  const declared = projectTools[ws.id] ?? noProjectTools;
  const gate = gateOf(declared);
  const declaredTools = declared.hash ? declared.tools : { mcp: null, plugins: null, skills: null };
  const global = board.tools ?? { mcp: null, plugins: null, skills: null };
  const own = workspaceTools(ws);
  const mcpIds = mcpHub.map((s) => s.id);
  // The Claude CLI base joins the universe below the hub; other agents have no inherited servers.
  const base = ws.agent === "claude" ? cliServers.map((s) => s.id).filter((id) => !mcpIds.includes(id)) : [];
  const pluginIds = pluginHub.map((p) => p.id);
  return { gate, declaredTools, global, own, base, mcpUniverse: [...mcpIds, ...base], pluginIds };
}

/// Mirror of axis_provenance: label every universe ID with where its effective state came from, so
/// the picker shows the resolved result without reading the layers. Off and untouched IDs are
/// omitted; CLI-base IDs that stay on are labeled "cli"; gated project declarations show as
/// "pending" or "rejected".
function axisProvenance(
  global: Selection | null,
  project: Selection | null,
  gate: Gate,
  workspace: Selection | null,
  base: string[],
  universe: string[],
): EffectiveItem[] {
  const gated = gate === "trusted" ? project : null;
  const effective = resolveWithBase(base, global, gated, workspace, universe);
  const declared = composeAxis([null, project, null], universe);
  const items: EffectiveItem[] = [];
  for (const id of universe) {
    const on = effective.includes(id);
    const added = workspace?.add.includes(id) ?? false;
    const removed = workspace?.remove.includes(id) ?? false;
    let provenance: Provenance;
    if (on && added) provenance = "added";
    else if (on && base.includes(id)) provenance = "cli";
    else if (on) provenance = "inherited";
    else if (removed) provenance = "removed";
    else if (gate !== "trusted" && declared.includes(id)) provenance = gate === "rejected" ? "rejected" : "pending";
    else continue;
    items.push({ id, provenance });
  }
  return items;
}

/// The workspace triple as a `Tools` layer, matching Workspace::tools() in the backend.
function workspaceTools(ws: Workspace): Tools {
  return { mcp: ws.mcp, plugins: ws.plugins, skills: ws.skills };
}

/// Dock keys match Rust: <workspace>:<kind>. Setup finishes on a timer so retained output is available in the browser.
const docks = new Map<string, boolean>();
const DONE = "\r\n\x1b[32m✓ terminou\x1b[0m\r\n";

const SCRIPT_OUT =
  "\x1b[2m$ npm run dev -- --port 3110\x1b[0m\r\n\r\n" +
  "  \x1b[32m➜\x1b[0m  Local:   \x1b[36mhttp://localhost:3110/\x1b[0m\r\n" +
  "  \x1b[32m➜\x1b[0m  ready in 231 ms\r\n\r\n";

let linear: LinearStatus = { connected: false, who: null, busy: false };

const issue = (
  identifier: string,
  title: string,
  priority: number,
  state: [string, string, string],
  project: string | null,
  hours: number,
  description: string | null = null,
): Issue => ({
  id: `id-${identifier}`,
  identifier,
  title,
  description,
  url: `https://linear.app/moabi/issue/${identifier}/${title.toLowerCase().replace(/\W+/g, "-")}`,
  branch_name: `gustavo/${identifier.toLowerCase()}-${title.toLowerCase().replace(/\W+/g, "-").slice(0, 40)}`,
  priority,
  priority_label: ["Sem prioridade", "Urgente", "Alta", "Média", "Baixa"][priority],
  state: { name: state[0], kind: state[1], color: state[2] },
  team: identifier.split("-")[0],
  project,
  labels: [],
  updated_at: new Date(Date.now() - hours * 3_600_000).toISOString(),
});
const DOING: [string, string, string] = ["In Progress", "started", "#f2c94c"];
const TODO: [string, string, string] = ["Todo", "unstarted", "#e2e2e2"];
const BACKLOG: [string, string, string] = ["Backlog", "backlog", "#bec2c8"];
const ISSUES: Issue[] = [
  issue("MOA-142", "Conectar o Linear ao Prometeu", 2, DOING, "Integrações", 1, "Aba de issues e criar workspace a partir de uma delas."),
  issue("MOA-137", "[Quadro] Card arrastado entre colunas perde a etapa quando o mouse solta fora da coluna (drop + reordenar)", 1, DOING, "Quadro", 5),
  issue("MOA-151", "Mostrar tokens de contexto no card", 3, TODO, "Quadro", 26),
  issue("MOA-149", "Atalho ⌘, para configurações", 4, TODO, null, 30),
  issue("MOA-120", "Explorar sync com Notion", 0, BACKLOG, "Integrações", 240),
  // A second team makes the team filter visible.
  issue("INF-88", "Runner self-hosted cai depois de duas horas ocioso", 1, DOING, "Infra", 3),
  issue("INF-72", "Assinar o .dmg no CI sem pedir a senha do Keychain", 3, TODO, "Infra", 52),
];

/// Number conversation lines like the backend. Live events and snapshots share that sequence for real-relay browser tests.
const scrolls = new Map<string, { text: string; seq: number }>();
const conversationAdapters = new Map<string, LegacyConversationAdapter>();
const scrollOf = (tab: string) => {
  let s = scrolls.get(tab);
  if (!s) {
    s = { text: SAMPLE, seq: 1 };
    scrolls.set(tab, s);
  }
  return s;
};
function pushLine(tab: string, o: unknown, keep = true) {
  const s = scrollOf(tab);
  let adapter = conversationAdapters.get(tab);
  if (!adapter) conversationAdapters.set(tab, (adapter = new LegacyConversationAdapter()));
  const canonical = parseConversationEvent(o);
  for (const event of canonical ? [canonical] : adapter.translate(o)) {
    const text = line(event);
    if (keep) s.text += text + "\n";
    s.seq += 1;
    emit("chat", [tab, text, s.seq]);
  }
}

/// Echo input and stream a sample reply. The fixture keywords select plan approval or multiple-choice question cards.
let msgN = 0;
/// The browser MCP hub supports settings edits without a backend.
let mcpHub: McpServer[] = [
  { id: "capim-ds", config: { type: "stdio", command: "npx", args: ["-y", "@capim/ds-mcp"], env: {} }, note: "design system" },
  { id: "notion", config: { type: "http", url: "https://mcp.notion.com/mcp" }, note: "" },
  { id: "linear-server", config: { type: "http", url: "https://mcp.linear.app/mcp" }, note: "capim-backend" },
];

/// Servers discoverable from the user's Claude configuration; they form the CLI-inherited base of
/// the workspace picker (ADR 0044) and the import menu.
const cliServers: McpServer[] = [
  { id: "metabase", config: { type: "http", url: "https://metabase.exemplo/mcp" }, note: "capim-backend" },
  { id: "n8n", config: { type: "stdio", command: "npx", args: ["-y", "n8n-mcp"], env: {} }, note: "" },
];

/// Count MCP/plugin writes so tests can verify that rapid selections are coalesced.
let writes = 0;

/// One progress step from the simulated plugin author.
type Step = { kind: string; text: string };

/// The sample hub includes a marketplace plugin and a plugin being authored locally.
let pluginHub: Plugin[] = [
  { id: "caveman", source: "~/.prometeu/plugins/caveman", note: "fala curto e sem enfeite", made: true, from: "https://github.com/JuliusBrussee/caveman" },
  { id: "ponytail", source: "~/dev/ponytail", note: "em construção" },
];


/// Plugin creation progress uses timers instead of an agent.
let pluginRun = 0;

/// MCP servers authenticated in this browser session.
let mcpLogins: string[] = [];

const CONTEXT_MD = "## Context Usage\n\n**Model:** claude-fable-5  \n**Tokens:** 20.2k / 1m (2%)\n\n### Estimated usage by category\n\n| Category | Tokens | Percentage |\n|----------|--------|------------|\n| System prompt | 4k | 0.4% |\n| System tools | 6.5k | 0.7% |\n| MCP tools (deferred) | 14.3k | 1.4% |\n| System tools (deferred) | 14k | 1.4% |\n| Custom agents | 368 | 0.0% |\n| Skills | 3k | 0.3% |\n| Messages | 6.3k | 0.6% |\n| Compact buffer | 3k | 0.3% |\n| Free space | 976.8k | 97.7% |\n\n### MCP Tools\n\n| Tool | Server | Tokens |\n|------|--------|--------|\n| mcp__capim-ds__get_components | capim-ds | 250 |\n| mcp__capim-ds__get_foundations | capim-ds | 209 |\n| mcp__capim-ds__get_icon_details | capim-ds | 168 |\n| mcp__capim-ds__get_illustration_details | capim-ds | 194 |\n| mcp__capim-ds__get_logo_details | capim-ds | 171 |\n| mcp__capim-ds__list_components | capim-ds | 130 |\n| mcp__capim-ds__list_icons | capim-ds | 107 |\n| mcp__capim-ds__list_illustrations | capim-ds | 120 |\n| mcp__capim-ds__list_logos | capim-ds | 112 |\n| mcp__claude_ai_Google_Drive__copy_file | claude_ai_Google_Drive | 444 |\n| mcp__claude_ai_Google_Drive__create_file | claude_ai_Google_Drive | 965 |\n| mcp__claude_ai_Google_Drive__download_file_content | claude_ai_Google_Drive | 433 |\n| mcp__claude_ai_Google_Drive__get_file_metadata | claude_ai_Google_Drive | 237 |\n| mcp__claude_ai_Google_Drive__get_file_permissions | claude_ai_Google_Drive | 143 |\n\n### Custom Agents\n\n| Agent Type | Source | Tokens |\n|------------|--------|--------|\n| caveman:cavecrew-builder | Plugin | 134 |\n| caveman:cavecrew-investigator | Plugin | 112 |\n| caveman:cavecrew-reviewer | Plugin | 122 |\n\n### Skills\n\n| Skill | Source | Tokens |\n|-------|--------|--------|\n| para-memory-files | User | ~190 |\n| caveman:cavecrew | Plugin (caveman) | ~190 |\n| caveman:caveman | Plugin (caveman) | ~140 |\n| caveman:caveman-commit | Plugin (caveman) | ~120 |\n| caveman:caveman-compress | Plugin (caveman) | ~120 |\n| caveman:caveman-help | Plugin (caveman) | ~70 |\n| caveman:caveman-review | Plugin (caveman) | ~110 |\n| caveman:caveman-stats | Plugin (caveman) | ~90 |\n| dataviz | Built-in | ~380 |\n| update-config | Built-in | ~240 |\n| keybindings-help | Built-in | ~80 |\n| code-review | Built-in | ~270 |\n| simplify | Built-in | ~60 |\n| fewer-permission-prompts | Built-in | ~60 |\n| loop | Built-in | ~120 |\n| schedule | Built-in | ~130 |\n| claude-api | Built-in | ~360 |\n| workflow-authoring | Built-in | ~80 |\n| run | Built-in | ~120 |\n| init | Built-in | ~20 |\n| security-review | Built-in | ~30 |";
function sayInto(tab: string, text: string) {
  pushLine(tab, { v: 1, type: "session.state", at: Date.now(), state: "busy" }, false);
  pushLine(tab, { type: "user", message: { role: "user", content: text }, ts: Date.now() });
  const id = `mm${++msgN}`;
  if (text.trim() === "/context") {
    pushLine(tab, { type: "assistant", message: { id: `${id}x`, model: "<synthetic>", role: "assistant", content: [{ type: "text", text: CONTEXT_MD }] } });
    pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 0 });
    return;
  }
  if (text.trim() === "/compact") {
    // Keep compaction progress visible until the result, summary, and command echo arrive.
    pushLine(tab, { type: "system", subtype: "status", status: "compacting" });
    setTimeout(() => {
      pushLine(tab, { type: "system", subtype: "status", status: null, compact_result: "success" });
      pushLine(tab, { type: "system", subtype: "compact_boundary", compact_metadata: { trigger: "manual", pre_tokens: 23978, post_tokens: 3132 } });
      pushLine(tab, { type: "user", isCompactSummary: true, message: { role: "user", content: "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\nSummary:\n1. **Primary Request**: trocar o `destroy` por `completed_at`.\n2. **Files**: `app/models/todo.rb`." } });
      pushLine(tab, { type: "user", message: { role: "user", content: "<local-command-stdout>Compacted </local-command-stdout>" } });
      pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 4000 });
    }, 4000);
    return;
  }
  const words = (text.includes("plano")
    ? "Li o pedido. Segue o plano — aprove para eu começar."
    : text.includes("pergunta")
      ? "Antes de mexer, uma pergunta."
      : `Entendi: **${text.slice(0, 40)}**. Vou olhar o código e volto com o que achei.`
  ).split(" ");
  let i = 0;
  pushLine(tab, { type: "stream_event", event: { type: "message_start", message: { id } } }, false);
  pushLine(tab, { type: "stream_event", event: { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } } }, false);
  const tick = setInterval(() => {
    if (i < words.length) {
      pushLine(tab, { type: "stream_event", event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: (i ? " " : "") + words[i++] } } }, false);
      return;
    }
    clearInterval(tick);
    pushLine(tab, { type: "assistant", message: { id, role: "assistant", content: [{ type: "text", text: words.join(" ") }] } });
    if (text.includes("plano")) {
      pushLine(tab, { type: "assistant", message: { id, role: "assistant", content: [{ type: "tool_use", id: `tu-${id}`, name: "ExitPlanMode", input: { plan: "# Plano\n\n1. Ler `app/models/todo.rb`\n2. Trocar o `destroy` por `completed_at`\n3. Rodar os testes" } }] } });
      pushLine(tab, { type: "control_request", request_id: `req-${id}`, request: { subtype: "can_use_tool", tool_name: "ExitPlanMode", input: { plan: "# Plano\n\n1. Ler `app/models/todo.rb`\n2. Trocar o `destroy` por `completed_at`\n3. Rodar os testes" }, tool_use_id: `tu-${id}` } });
      return;
    }
    if (text.includes("background")) {
      // Two background tasks exercise progress counts, completion notices, and automatic agent replies.
      const tasks = [
        { task: `bg-${id}a`, tool: `tu-${id}a`, desc: "Mapear lacunas de teste" },
        { task: `bg-${id}b`, tool: `tu-${id}b`, desc: "Auditar qualidade do repositório" },
      ];
      for (const k of tasks) {
        pushLine(tab, { type: "assistant", message: { id, role: "assistant", content: [{ type: "tool_use", id: k.tool, name: "Agent", input: { description: k.desc, subagent_type: "Explore", run_in_background: true, prompt: "…" } }] } });
        pushLine(tab, { type: "system", subtype: "task_started", task_id: k.task, tool_use_id: k.tool, description: k.desc, is_backgrounded: true, task_type: "local_agent" });
        pushLine(tab, { type: "user", message: { role: "user", content: [{ tool_use_id: k.tool, type: "tool_result", content: `Agent started with ID: ${k.task}. You will be notified when it completes.`, is_error: false }] } });
      }
      pushLine(tab, { type: "system", subtype: "background_tasks_changed", tasks: tasks.map((k) => ({ task_id: k.task, task_type: "local_agent", description: k.desc })) });
      pushLine(tab, { type: "assistant", message: { id: `${id}c`, role: "assistant", content: [{ type: "text", text: "Dois agentes rodando. Aviso quando terminarem." }] } });
      pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 1200 });
      tasks.forEach((k, n) =>
        setTimeout(() => {
          pushLine(tab, { type: "system", subtype: "background_tasks_changed", tasks: tasks.slice(n + 1).map((j) => ({ task_id: j.task, task_type: "local_agent", description: j.desc })) });
          pushLine(tab, { type: "system", subtype: "task_notification", task_id: k.task, tool_use_id: k.tool, status: "completed", summary: `Agent "${k.desc}" finished` });
          pushLine(tab, { type: "assistant", message: { id: `${id}d${n}`, role: "assistant", content: [{ type: "text", text: `Terminou: ${k.desc}.` }] } });
          pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 300 });
        }, 5000 * (n + 1)),
      );
      return;
    }
    if (text.includes("diff")) {
      const diff = "diff --git a/src/lib/token.ts b/src/lib/token.ts\nindex 4354763..0458d9c 100644\n--- a/src/lib/token.ts\n+++ b/src/lib/token.ts\n@@ -1,4 +1,5 @@\n /**\n- * Shape of the token\n+ * Shape of the token that authenticates\n+ * the public landings\n  */\n const PATTERN = /^[1-9A-Z]{36}$/;";
      pushLine(tab, { type: "assistant", message: { id, role: "assistant", content: [{ type: "tool_use", id: `tu-${id}`, name: "Bash", input: { command: "git diff -- src/lib/token.ts", description: "Diff do arquivo" } }] } });
      pushLine(tab, { type: "user", message: { role: "user", content: [{ tool_use_id: `tu-${id}`, type: "tool_result", content: diff, is_error: false }] } });
      pushLine(tab, { type: "assistant", message: { id: `${id}c`, role: "assistant", content: [{ type: "text", text: "O mesmo diff, num bloco:\n\n```diff\n" + diff + "\n```\n\n1 arquivo, +2 −1." }] } });
      pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 1200 });
      return;
    }
    if (text.includes("pergunta")) {
      pushLine(tab, { type: "assistant", message: { id, role: "assistant", content: [{ type: "tool_use", id: `tu-${id}`, name: "AskUserQuestion", input: { questions: [{ header: "Histórico", question: "Onde guardar os concluídos?", options: [{ label: "Coluna", description: "completed_at na tabela de todos" }, { label: "Tabela", description: "uma tabela só deles" }] }, { header: "Migração", question: "Rodar a migração agora?", options: [{ label: "Sim", description: "no banco de dev" }, { label: "Depois", description: "só escrever o arquivo" }] }] } }] } });
      pushLine(tab, { type: "control_request", request_id: `req-${id}`, request: { subtype: "can_use_tool", tool_name: "AskUserQuestion", input: { questions: [{ header: "Histórico", question: "Onde guardar os concluídos?", options: [{ label: "Coluna", description: "completed_at na tabela de todos" }, { label: "Tabela", description: "uma tabela só deles" }] }, { header: "Migração", question: "Rodar a migração agora?", options: [{ label: "Sim", description: "no banco de dev" }, { label: "Depois", description: "só escrever o arquivo" }] }] }, tool_use_id: `tu-${id}` } });
      return;
    }
    pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 1200 });
  }, 60);
}

/// A card response completes the simulated tool and turn.
function controlInto(tab: string, frame: Record<string, any>) {
  if (frame.v !== 1 || frame.type !== "request.respond") return;
  const req = String(frame.requestId ?? "");
  const id = req.replace(/^req-/, "");
  const denied = frame.response?.outcome === "deny";
  pushLine(tab, { type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: `tu-${id}`, content: denied ? String(frame.response.message) : "ok", is_error: denied }] } });
  pushLine(tab, { type: "assistant", message: { id: `${id}b`, role: "assistant", content: [{ type: "text", text: denied ? "Certo, vou mudar o plano." : "Combinado. Seguindo." }] } });
  pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 400 });
}

/// Persist shared workspaces and their access choices across browser reloads, mirroring board.json.
const SHARED = "mock:shared";
for (const [id, audience, shareTeam, remoteControl] of JSON.parse(localStorage.getItem(SHARED) ?? "[]") as [string, string[] | null, string?, boolean?][]) {
  const ws = board.workspaces.find((x) => x.id === id);
  if (ws) {
    ws.shared = true;
    ws.audience = audience;
    ws.share_team = shareTeam;
    ws.remote_control = remoteControl ?? false;
  }
}

function emit(event: string, payload: unknown) {
  handlers.get(event)?.forEach((h) => h({ event, id: nextId++, payload }));
}

const mockCommands: IpcHandlers = {
  accounts() {
    return structuredClone(mockAccounts);
  },
  account_select(args) {
    const account = mockAccounts.accounts.find((account) => account.id === args.id);
    if (!account) throw 'i18n:{"code":"err.account.missing"}';
    if (!account.connected && account.id !== account.provider) throw 'i18n:{"code":"err.account.disconnected"}';
    mockAccounts.active[account.provider] = account.id;
    return accountSnapshot();
  },
  account_remove(args) {
    if (mockAccounts.login) throw 'i18n:{"code":"err.account.busy"}';
    const account = mockAccounts.accounts.find((account) => account.id === args.id);
    if (!account) throw 'i18n:{"code":"err.account.missing"}';
    if (mockAccounts.active[account.provider] === account.id) delete mockAccounts.active[account.provider];
    mockAccounts.accounts = mockAccounts.accounts.filter((entry) => entry.id !== account.id);
    const snapshot = accountSnapshot();
    emit("usage", call("usage"));
    return snapshot;
  },
  account_login(args) {
    if (mockAccounts.login) throw 'i18n:{"code":"err.account.busy"}';
    if (!["claude", "codex"].includes(args.provider)) throw 'i18n:{"code":"err.account.provider"}';
    let account = mockAccounts.accounts.find((account) => account.id === args.id);
    if (!account) {
      account = { id: crypto.randomUUID(), provider: args.provider, email: null, plan: null, connected: false, revision: 0 };
      mockAccounts.accounts.push(account);
    }
    const connecting = account;
    mockAccounts.login = { id: connecting.id, provider: connecting.provider };
    accountSnapshot();
    return new Promise((resolve, reject) => {
      const finish = (code?: string) => {
        mockAccounts.login = null;
        cancelAccountLogin = null;
        if (!code) {
          connecting.connected = true;
          connecting.email = "nova@exemplo.com";
          connecting.plan = "pro";
          connecting.revision++;
        }
        const snapshot = accountSnapshot();
        emit("usage", call("usage"));
        if (code) reject(`i18n:${JSON.stringify({ code })}`);
        else resolve(snapshot);
      };
      const timer = setTimeout(() => finish(localStorage.getItem("mock:accountLoginError") ? "err.account.login" : undefined), 1000);
      cancelAccountLogin = () => { clearTimeout(timer); finish("err.account.cancelled"); };
    });
  },
  account_login_cancel(args) {
    if (mockAccounts.login?.id === args.id) cancelAccountLogin?.();
    return;
  },
  actions_save(args) {
    const catalog = args.catalog as Catalog;
    if (new Set(catalog.commands.map(c => c.name)).size !== catalog.commands.length || catalog.commands.some(c => !/^[a-z0-9-]{1,64}$/.test(c.name) || ["context", "compact"].includes(c.name) || (c.kind === "prompt" ? !c.prompt.trim() : !catalog.profiles.some(p => p.id === c.profile))) || catalog.profiles.some(p => !p.name.trim() || !p.prompt.trim() || (p.watch && (p.watch.interval_seconds < 30 || p.watch.max_turns < 1 || p.watch.max_turns > 100)))) {
      throw `i18n:${JSON.stringify({ code: "err.actions.invalid" })}`;
    }
    board.actions = structuredClone(catalog);
    localStorage.setItem("mock:actions", JSON.stringify(catalog));
    emit("board", board);
    return;
  },
  action_start(args) {
    const workspace = board.workspaces.find(w => w.id === args.workspace);
    const catalog = board.actions ?? emptyCatalog();
    const action = catalog.commands.find(c => c.name === args.name && c.kind === "agent");
    if (!workspace || !action?.profile || workspace.cleaned || workspace.archived) throw `i18n:${JSON.stringify({ code: "err.actions.unavailable" })}`;
    const existing = workspace.tabs.find(t => t.task?.command === action.name && !t.task.done);
    if (existing) {
      if (String(args.context ?? "").trim()) throw `i18n:${JSON.stringify({ code: "err.actions.active" })}`;
      return existing;
    }
    if (workspace.tabs.some(t => t.status === "rodando" || t.status === "querendo" || t.pending_prompt)) throw `i18n:${JSON.stringify({ code: "err.actions.busy" })}`;
    const profile = structuredClone(catalog.overrides[workspace.project]?.[action.profile] ?? catalog.profiles.find(p => p.id === action.profile)) as Profile;
    // Mirror of resolve_workspace_tools: a profile that leaves an axis unset inherits the resolved
    // global, project and workspace layers instead of only the workspace's own adds.
    const l = toolLayers(workspace);
    const project = l.gate === "trusted" ? l.declaredTools : { mcp: null, plugins: null, skills: null };
    // Mirror of resolve_axis: an axis nobody declared stays null, preserving the provider's own set.
    const axis = (g: Selection | null, p: Selection | null, w: Selection | null, base: string[], universe: string[]) =>
      g === null && p === null && w === null ? null : resolveWithBase(base, g, p, w, universe);
    const plugins = axis(l.global.plugins, project.plugins, l.own.plugins, [], l.pluginIds);
    const skills = axis(l.global.skills, project.skills, l.own.skills, [], l.pluginIds);
    profile.mcp ??= axis(l.global.mcp, project.mcp, l.own.mcp, l.base, l.mcpUniverse);
    // Mirror of plugin_packages: plugins and standalone skills materialize together.
    profile.plugins ??= plugins === null && skills === null ? null : [...(plugins ?? []), ...(skills ?? [])];
    const tab: Tab = { id: crypto.randomUUID(), title: profile.name, choice: profile.choice, status: "pronta", note: null, tokens: null,
      task: { command: action.name, profile, paused: false, done: !profile.watch, turns: 0, checked_at: 0, error: null, seen: {}, prs: {} } };
    scrolls.set(tab.id, { text: line({ v: 1, type: "user.message", at: Date.now(), content: [{ kind: "text", text: [action.prompt, args.context].filter(Boolean).join("\n\n") || profile.prompt }] }) + "\n", seq: 1 });
    workspace.tabs.push(tab); workspace.active = tab.id;
    emit("board", board);
    return tab;
  },
  action_pause(args) {
    const run = board.workspaces.flatMap(w => w.tabs).find(t => t.id === args.session)?.task;
    if (!run) throw `i18n:${JSON.stringify({ code: "err.actions.missing" })}`;
    run.paused = args.paused; run.error = null;
    if (!run.paused) { run.turns = 0; run.checked_at = 0; }
    emit("board", board); return;
  },
  load_board() {
    return board;
  },
  cloud_status(args) {
    if (args.refresh && localStorage.getItem("mock:cloudExpired")) localStorage.removeItem("mock:cloud");
    return { ...mockCloud(), offline: !!localStorage.getItem("mock:cloudOffline") };
  },
  cloud_organizations() {
    const cloud = mockCloud();
    if (localStorage.getItem("mock:cloudOffline")) throw 'i18n:{"code":"err.cloud.network"}';
    return { user: cloud.user, origin: cloud.origin, organizations: cloud.user ? JSON.parse(localStorage.getItem("mock:organizations") ?? "[]") : [] };
  },
  cloud_relay_ticket(args) {
    cloudWrite();
    const cloud = mockCloud();
    const org = JSON.parse(localStorage.getItem("mock:organizations") ?? "[]").find((org: team.Organization) => org.id === args.organization);
    if (!org || cloud.user?.id !== args.user || cloud.origin !== args.expectedOrigin) throw 'i18n:{"code":"err.cloud.response"}';
    return `ws://mock/organization/${org.id}?p=4&ticket=${"t".repeat(43)}&m=${org.member}&n=${encodeURIComponent(cloud.user!.name)}`;
  },
  cloud_login_start() {
    if (localStorage.getItem("mock:cloudOffline")) throw 'i18n:{"code":"err.cloud.network"}';
    cloudPending = crypto.randomUUID();
    return { id: cloudPending, user_code: "ABCD-EFGH", url: "https://app.prometeu.co/device?user_code=ABCD-EFGH&mode=signup", interval: 5 };
  },
  cloud_login_poll(args) {
    if (!cloudPending || cloudPending !== args.id) throw 'i18n:{"code":"err.cloud.expired"}';
    if (!localStorage.getItem("mock:cloudApproved")) return null;
    const value = { ...emptyCloud(), user: { id: "cloud-user", name: "Gustavo Brancaglione", email: "gustavo@example.com" } };
    localStorage.setItem("mock:cloud", JSON.stringify(value));
    localStorage.removeItem("mock:cloudApproved"); cloudPending = null; emit("catalog", null);
    return value;
  },
  catalog_state() {
    if (!mockCloud().user) return { connected: false, revision: null, plugins: [], mcp: [], skills: [], shared: {} };
    const state = mockCatalog();
    state.plugins = state.plugins.map(p => ({ ...p, installed: pluginHub.some(local => local.id === p.local_id) }));
    state.skills = state.skills.map(s => ({ ...s, installed: skillHub.some(local => local.id === s.local_id) }));
    state.organization_items = mockOrganizations().flatMap(org => (["plugins", "mcp", "skills"] as Kind[]).flatMap(kind => org[kind].map(item => ({
      organization: org.id, organization_name: org.name, revision: org.revision ?? 0, kind, id: item.id,
      description: "description" in item ? item.description : "source" in item ? `${item.source} · ${item.note}` : item.note,
      installed: (kind === "plugins" ? pluginHub : kind === "mcp" ? mcpHub : skillHub).some(local => local.id === org.links[`${kind}:${item.id}`]),
    }))));
    return state;
  },
  catalog_refresh() {
    cloudWrite(); emit("catalog", null); return;
  },
  catalog_share(args) {
    cloudWrite();
    const kind = String(args.kind), id = String(args.id), state = mockCatalog();
    if (state.shared[`${kind}:${id}`]) throw 'i18n:{"code":"err.catalog.conflict"}';
    if (kind === "plugins") {
      const plugin = pluginHub.find(p => p.id === id);
      const source = plugin?.from || plugin?.source || "";
      if (!/^(https?:\/\/|git@|[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$)/.test(source)) throw 'i18n:{"code":"err.catalog.portable"}';
      state.plugins.push({ id, source, note: plugin!.note, local_id: id, installed: true, source_changed: false });
    } else if (kind === "mcp") state.mcp.push(id);
    else if (kind === "skills") {
      const skill = skillHub.find(s => s.id === id); if (!skill) throw 'i18n:{"code":"err.catalog.invalid"}';
      state.skills.push({ ...skill, local_id: id, installed: true });
    } else throw 'i18n:{"code":"err.catalog.invalid"}';
    state.shared[`${kind}:${id}`] = id; saveMockCatalog(state); return;
  },
  catalog_copy(args) {
    const kind = String(args.kind), id = String(args.id), newId = String(args.newId).trim();
    if (!newId) throw 'i18n:{"code":"err.catalog.invalid"}';
    if (kind === "skills") {
      if (skillHub.some(s => s.id === newId)) throw 'i18n:{"code":"err.catalog.conflict"}';
      const skill = skillHub.find(s => s.id === id); if (!skill) throw 'i18n:{"code":"err.catalog.invalid"}';
      saveMockSkill({ ...skill, id: newId });
    } else if (kind === "mcp") {
      if (mcpHub.some(s => s.id === newId)) throw 'i18n:{"code":"err.catalog.conflict"}';
      const server = mcpHub.find(s => s.id === id); if (!server) throw 'i18n:{"code":"err.catalog.invalid"}';
      mcpHub.push({ ...structuredClone(server), id: newId });
    } else {
      if (pluginHub.some(p => p.id === newId)) throw 'i18n:{"code":"err.catalog.conflict"}';
      const plugin = pluginHub.find(p => p.id === id); if (!plugin) throw 'i18n:{"code":"err.catalog.invalid"}';
      pluginHub.push({ ...plugin, id: newId, made: false });
    }
    emit("catalog", null); return;
  },
  catalog_install_plugin(args) {
    const item = mockCatalog().plugins.find(p => p.id === args.id); if (!item) throw 'i18n:{"code":"err.catalog.invalid"}';
    pluginHub = [...pluginHub.filter(p => p.id !== item.local_id), { id: item.local_id, source: `~/.prometeu/plugins/${item.local_id}`, from: item.source, note: item.note, made: false }];
    emit("catalog", null); return;
  },
  catalog_install_skill(args) {
    const item = mockCatalog().skills.find(s => s.id === args.id); if (!item) throw 'i18n:{"code":"err.catalog.invalid"}';
    saveMockSkill({ id: item.local_id, description: item.description, content: item.content }); emit("catalog", null); return;
  },
  catalog_install_organization_item({ organization, kind, id, revision }) {
    cloudWrite();
    const organizations = mockOrganizations();
    const org = organizations.find(org => org.id === organization);
    if (!org || !org[kind].some(item => item.id === id)) throw 'i18n:{"code":"err.catalog.invalid"}';
    if (revision !== (org.revision ?? 0)) { emit("catalog", null); throw 'i18n:{"code":"err.catalog.conflict"}'; }
    const hub = kind === "plugins" ? pluginHub : kind === "mcp" ? mcpHub : skillHub;
    const key = `${kind}:${id}`;
    if (hub.some(item => item.id === org.links[key])) return;
    const personal = mockCatalog();
    const used = new Set([...hub.map(item => item.id), ...Object.entries(personal.shared).filter(([key]) => key.startsWith(`${kind}:`)).map(([key]) => key.slice(kind.length + 1)),
      ...organizations.flatMap(org => Object.entries(org.links).filter(([key]) => key.startsWith(`${kind}:`)).map(([, local]) => local))]);
    let local = org.links[key] ?? id;
    if (!org.links[key]) {
      const stem = id.replace(/[^a-z0-9-]/g, "").slice(0, 40) || "item";
      for (let n = 1; used.has(local); n++) local = `cloud-${stem}-${n}`;
    }
    if (kind === "plugins") {
      const item = org.plugins.find(item => item.id === id)!;
      pluginHub.push({ id: local, source: `~/.prometeu/plugins/${local}`, from: item.source, note: item.note, made: false });
    } else if (kind === "mcp") mcpHub.push({ ...structuredClone(org.mcp.find(item => item.id === id)!), id: local });
    else saveMockSkill({ ...org.skills.find(item => item.id === id)!, id: local });
    org.links[key] = local;
    localStorage.setItem("mock:organizationCatalogs", JSON.stringify(organizations));
    emit("catalog", null); return;
  },
  skill_hub() {
    for (const skill of [...skillHub]) saveMockSkill(skill);
    return skillHub;
  },
  skill_save(args) {
    const skill = args.skill as Skill;
    const state = mockCatalog();
    if (mockCloud().user && state.shared[`skills:${skill.id}`]) {
      cloudWrite();
      if (args.revision !== state.revision) throw 'i18n:{"code":"err.catalog.conflict"}';
      state.skills = state.skills.map(s => s.local_id === skill.id ? { ...s, description: skill.description, content: skill.content } : s);
      saveMockCatalog(state);
    }
    saveMockSkill(skill); return skillHub;
  },
  skill_remove(args) {
    skillHub = skillHub.filter(s => s.id !== args.id);
    localStorage.setItem("mock:skills", JSON.stringify(skillHub));
    pluginHub = pluginHub.filter(p => p.id !== `skill-${args.id}`);
    return skillHub;
  },
  cloud_login_cancel(args) {
    if (cloudPending === args.id) cloudPending = null;
    return;
  },
  cloud_logout() {
    if (localStorage.getItem("mock:cloudOffline")) throw 'i18n:{"code":"err.cloud.network"}';
    localStorage.removeItem("mock:cloud");
    return emptyCloud();
  },
  remove_project(args) {
    board.projects = board.projects.filter((project) => project.id !== args.id);
    emit("board", board);
    return;
  },
  // Use localStorage for browser team state; Tauri stores it in team.json.
  team_config() {
    return { config: JSON.parse(localStorage.getItem("mock:team") ?? "null"), default_name: "Você" };
  },
  team_security() {
    return JSON.parse(localStorage.getItem("mock:team-security") ?? "null");
  },
  team_security_set({ state }) {
    localStorage.setItem("mock:team-security", JSON.stringify(state));
  },
  team_config_set(args) {
    if (args.config) localStorage.setItem("mock:team", JSON.stringify(args.config));
    else localStorage.removeItem("mock:team");
    return;
  },
  pty_buffer(args) {
    const s = String(args.session);
    const text = docks.get(s) === false ? SCRIPT_OUT + DONE : SCRIPT_OUT;
    return [...new TextEncoder().encode(text)];
  },
  // Append runtime turn state after the numbered transcript; it is not a conversation line.
  chat_snapshot(args) {
    const s = scrollOf(String(args.session));
    return { text: s.text + line({ v: 1, type: "session.state", at: Date.now(), state: "ready" }) + "\n", seq: s.seq };
  },
  // Respond to incoming messages so remote input and replies can be tested.
  chat_send(args) {
    sayInto(String(args.session), String(args.text));
    return;
  },
  chat_control(args) {
    controlInto(String(args.session), args.frame as Record<string, any>);
    return;
  },
  chat_control_remote(args) {
    controlInto(String(args.session), args.frame as Record<string, any>);
    return;
  },
  pty_write() {
    return;
  },
  // Return one change group per repository, including the second repository's independent changes.
  workspace_diff(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    return (target?.repos ?? []).map((r, i) => {
      const files = i === 0 ? changes : changes2;
      const base = i === 0 ? "origin/main" : "origin/develop";
      // Only the first repository has unpushed commits, exercising both summary states.
      const ahead = i === 0 ? 3 : 1;
      return { name: r.name, base, ahead, unpushed: i === 0 ? 1 : 0, dirty: files.filter((f) => f.dirty).length, files };
    });
  },
  workspace_git_status(args) {
    const target = board.workspaces.find((workspace) => workspace.id === args.id && !workspace.cleaned);
    if (!target) return gitError("err.session.noWorkspace");
    return target.repos.map((_, index) => gitStatus(gitState(target.id, index)));
  },
  workspace_git_diff(args) {
    const value = gitState(args.id, args.repo ?? 0);
    const committed = args.scope === "commit";
    const comparison = args.scope === "compare";
    const position = committed && args.reference && args.reference !== "HEAD"
      ? value.commits.findIndex((commit) => commit.oid === args.reference) : 0;
    if (committed && position < 0) return gitError("err.git.changed");
    const source = args.scope === "staged" ? value.staged : args.scope === "changes" ? value.changes
      : comparison ? value.compare : value.commits[position].files;
    return structuredClone({
      base: committed ? value.commits[position + 1]?.oid ?? "0".repeat(40) : comparison ? "1".padStart(40, "0") : "",
      head: committed || comparison ? value.commits[committed ? position : 0].oid : "",
      files: source.filter((file) => args.path == null || file.path === args.path),
    });
  },
  workspace_git_action(args) {
    const value = gitState(args.id, args.repo ?? 0);
    const status = gitStatus(value);
    const operation = args.operation;
    const selected: string[] = args.paths ?? [];
    if (operation === "stage" || operation === "unstage") {
      if (!selected.length) return gitError("err.git.selection");
      const source = operation === "stage" ? value.changes : value.staged;
      const target = operation === "stage" ? value.staged : value.changes;
      if (selected.some((path) => !source.some((file) => file.path === path) && !value.conflicts[path])) return gitError("err.git.changed");
      for (const path of new Set(selected)) {
        const at = source.findIndex((file) => file.path === path);
        if (at >= 0) mergeGitFile(target, source.splice(at, 1)[0], operation === "unstage");
        else {
          const conflict = value.conflicts[path];
          const file = { path, added: 0, removed: 0, new_file: false, deleted: false, dirty: true, patch: "" };
          mergeGitFile(target, gitPatch(file, `@@ -0,0 +1,${conflict.current.split("\n").length} @@\n${conflict.current.split("\n").map((line) => `+${line}`).join("\n")}`));
        }
        delete value.conflicts[path];
      }
      value.status.index = `mock-${++value.version}`;
    } else if (operation === "commit") {
      if (!status.branch) return gitError("err.git.detached");
      if (status.conflicts.length) return gitError("err.git.conflicts");
      if ((!value.staged.length && !status.merging) || !String(args.message ?? "").trim()) return gitError("err.git.selection");
      if (args.expected !== status.index) return gitError("err.git.changed");
      const committed = value.staged.map((file) => ({ ...file, dirty: false }));
      for (const file of committed) mergeGitFile(value.compare, file);
      value.commits.unshift({
        oid: (++nextId + 100).toString(16).padStart(40, "0"),
        subject: String(args.message).trim().split("\n")[0], author: "Gustavo",
        date: new Date().toISOString(), outgoing: !!status.upstream, files: committed,
      });
      value.staged = [];
      value.status.merging = false;
      value.status.ahead += status.upstream ? 1 : 0;
      value.status.index = `mock-${++value.version}`;
    } else if (operation === "fetch") {
      // Simulate a remote commit without network access.
      value.status.behind = status.upstream ? Math.max(1, status.behind) : 0;
    } else if (operation === "pull") {
      if (!status.branch) return gitError("err.git.detached");
      if (!status.upstream) return gitError("err.git.upstream");
      if (status.staged.length || status.changes.length || status.conflicts.length) return gitError("err.git.dirtyPull");
      if (status.ahead && status.behind) return gitError("err.git.changed");
      value.status.behind = 0;
      value.status.index = `mock-${++value.version}`;
    } else if (operation === "push" || operation === "publish") {
      if (!status.branch) return gitError("err.git.detached");
      if (status.behind) return gitError("err.git.changed");
      if (operation === "publish") {
        if (!status.remotes.includes(args.remote ?? "")) return gitError("err.git.upstream");
        value.status.upstream = `${args.remote}/${status.branch}`;
      } else if (!status.upstream) return gitError("err.git.upstream");
      value.status.ahead = 0;
      value.commits.forEach((commit) => { commit.outgoing = false; });
    } else return gitError("err.git.selection");
    return;
  },
  workspace_git_history(args) {
    return gitState(args.id, args.repo ?? 0).commits.map(({ files: _, ...commit }) => ({ ...commit }));
  },
  workspace_git_branches(args) {
    const value = gitState(args.id, args.repo ?? 0);
    const current = board.workspaces.find((workspace) => workspace.id === args.id)!;
    const repo = current.repos[args.repo ?? 0];
    const branches: GitBranch[] = board.workspaces.filter((workspace) => !workspace.cleaned && workspace.repos.some((entry) => entry.path === repo.path)).map((workspace) => ({
      name: workspace.branch, current: workspace.branch === value.status.branch, remote: false,
      worktree: workspace.repos.find((entry) => entry.path === repo.path)!.worktree, workspace: workspace.id,
    }));
    branches.push(
      { name: "main", current: value.status.branch === "main", remote: false, worktree: repo.path, workspace: null },
      { name: "feature/local-work", current: false, remote: false, worktree: null, workspace: null },
      ...[...new Set([repo.base, value.status.upstream, "origin/feature/review"].filter((name): name is string => !!name))].map((name) => ({ name, current: false, remote: true, worktree: null, workspace: null })),
    );
    return branches.filter((branch, index) => branches.findIndex((other) => other.name === branch.name) === index);
  },
  workspace_git_conflict(args) {
    const conflict = gitState(args.id, args.repo ?? 0).conflicts[args.path];
    if (!conflict) return gitError("err.git.changed");
    return { ...conflict };
  },
  workspace_git_resolve(args) {
    const value = gitState(args.id, args.repo ?? 0);
    const conflict = value.conflicts[args.path];
    if (!conflict) return gitError("err.git.changed");
    if (conflict.current !== args.was) return gitError("err.session.changed");
    const text = String(args.text ?? "");
    if (new TextEncoder().encode(text).length > 400_000 || /^(<<<<<<< |=======|>>>>>>> )/m.test(text)) return gitError("err.git.conflicts");
    const before = (conflict.ours ?? "").split("\n");
    const after = text.split("\n");
    const patch = `@@ -1,${before.length} +1,${after.length} @@\n${before.map((line) => `-${line}`).join("\n")}\n${after.map((line) => `+${line}`).join("\n")}`;
    if (text !== conflict.ours) mergeGitFile(value.staged, gitPatch({ path: args.path, added: 0, removed: 0, new_file: false, deleted: false, dirty: true, patch: "" }, patch));
    delete value.conflicts[args.path];
    value.status.index = `mock-${++value.version}`;
    return;
  },
  list_dir(args) {
    return tree[args.rel ?? ""] ?? [];
  },
  // Viewer saves replace mock file contents. Concurrent disk-writer detection remains a Rust test.
  write_file(args) {
    files[args.rel] = args.text;
    return;
  },
  // The shallow sample tree needs only substring matching; return shallower paths first.
  find_paths(args) {
    const q = String(args.query ?? "").toLowerCase();
    const recent: string[] = args.recent ?? [];
    const points = (p: string) => {
      const at = recent.indexOf(p);
      return at < 0 ? 0 : 100 - at;
    };
    return Object.values(tree)
      .flat()
      .filter((e) => e.path.toLowerCase().includes(q))
      .sort(
        (a, b) =>
          points(b.path) - points(a.path) ||
          a.path.split("/").length - b.path.split("/").length ||
          a.path.localeCompare(b.path),
      )
      .slice(0, 40);
  },
  file_stamp() {
    return "0";
  },
  read_bytes(args) {
    if (args.rel in files) return new TextEncoder().encode(files[args.rel]).buffer;
    throw `i18n:${JSON.stringify({ code: "err.session.binary" })}`;
  },
  read_file(args) {
    if (args.rel in files) return files[args.rel];
    // Return backend error codes for frontend translation.
    throw args.rel.endsWith(".lock")
      ? `i18n:${JSON.stringify({ code: "err.session.tooBig", args: { kb: 2140 } })}`
      : `i18n:${JSON.stringify({ code: "err.session.binary" })}`;
  },
  rename_workspace(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) target.title = args.title;
    emit("board", board);
    return;
  },
  rename_tab(args) {
    const target = board.workspaces.find((x) => x.id === args.workspace);
    const tab = target?.tabs.find((t) => t.id === args.tab);
    if (tab) tab.title = args.title;
    emit("board", board);
    return;
  },
  focus_tab(args) {
    const target = board.workspaces.find((x) => x.id === args.workspace);
    if (target?.tabs.some((tab) => tab.id === args.tab)) target.active = args.tab;
    emit("board", board);
    return;
  },
  // A removed worktree has no branch to read, matching Rust.
  workspace_branch(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    return target && !target.cleaned ? target.branch : null;
  },
  set_stage(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) target.stage = args.stage;
    emit("board", board);
    return;
  },
  set_shared(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) {
      target.shared = args.shared;
      target.share_team = args.shared ? args.team : null;
      target.audience = args.shared ? args.audience ?? null : null;
      target.remote_control = args.shared && args.remoteControl;
    }
    // Persist sharing choices across page reloads, matching board.json.
    localStorage.setItem(SHARED, JSON.stringify(board.workspaces.filter((x) => x.shared).map((x) => [x.id, x.audience, x.share_team, x.remote_control])));
    emit("board", board);
    return;
  },
  pin_workspace(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) target.pinned = args.pinned;
    emit("board", board);
    return;
  },
  set_unread(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) target.unread = args.unread;
    emit("board", board);
    return;
  },
  look_at(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target?.unread) {
      target.unread = false;
      emit("board", board);
    }
    return;
  },
  archive_workspace(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) target.archived = args.archived;
    // Archiving stops tab processes, matching Rust.
    if (target && args.archived) target.tabs.forEach((t) => (t.status = "desligada"));
    emit("board", board);
    return;
  },
  // A fixed Codex catalog exercises provider selection without an installed CLI.
  agents() {
    return {
      providers: [
        {
          id: "claude",
          label: "Claude",
          installed: true,
          models: [],
          capabilities: {
            initialPlanMode: true,
            workspaceMcpSelection: true,
            workspacePluginSelection: true,
            resume: true,
            compact: true,
            contextReport: true,
            approvals: true,
            userQuestions: true,
            attachments: true,
          },
        },
        {
          id: "codex",
          label: "Codex",
          installed: true,
          models: [
            { id: "gpt-5.6-sol", label: "GPT-5.6-Sol", efforts: ["low", "medium", "high", "xhigh", "max", "ultra"] },
            { id: "gpt-5.6-terra", label: "GPT-5.6-Terra", efforts: ["low", "medium", "high", "xhigh", "max", "ultra"] },
            { id: "gpt-5.4", label: "GPT-5.4", efforts: ["low", "medium", "high", "xhigh"] },
          ],
          capabilities: {
            initialPlanMode: false,
            workspaceMcpSelection: true,
            workspacePluginSelection: true,
            resume: true,
            compact: true,
            contextReport: true,
            approvals: true,
            userQuestions: true,
            attachments: true,
          },
        },
      ],
    };
  },
  // Use the filtered Claude catalog shape. Haiku exposes no effort levels.
  claude_models() {
    return [
      { id: "opus[1m]", label: "Opus (1M context)", efforts: ["low", "medium", "high", "xhigh", "max"] },
      { id: "claude-fable-5[1m]", label: "Fable", efforts: ["low", "medium", "high", "xhigh", "max"] },
      { id: "sonnet", label: "Sonnet", efforts: ["low", "medium", "high", "xhigh", "max"] },
      { id: "haiku", label: "Haiku", efforts: [] },
    ];
  },
  // Sample provider quotas keep usage indicators visible in the browser.
  usage() {
    const now = Math.floor(Date.now() / 1000);
    return {
      ...Object.fromEntries(mockAccounts.accounts.filter((account) => account.id !== account.provider && account.connected).map((account, index) => [account.id, {
        windows: [{ kind: "session", pct: 9 + index, resets: now + 2 * 3600 }, { kind: "weekly", pct: 25 + index, resets: now + 4 * 86400 }],
        at: now,
      }])),
      claude: {
        windows: [
          { kind: "session", pct: 16, resets: now + 3 * 3600 + 14 * 60 },
          { kind: "weekly", pct: 78, resets: now + 3 * 86400 + 4 * 3600 },
          { kind: "fable", pct: 72, resets: now + 3 * 86400 + 4 * 3600 },
        ],
        at: now - 4 * 60,
      },
      codex: {
        windows: [
          { kind: "weekly", pct: 6, resets: now + 6 * 86400 + 20 * 3600, scope: "general" },
          {
            kind: "session",
            pct: 1,
            resets: now + 4 * 3600 + 55 * 60,
            scope: "codex_bengalfox",
            label: "GPT-5.3-Codex-Spark",
          },
          {
            kind: "weekly",
            pct: 0,
            resets: now + 6 * 86400 + 23 * 3600,
            scope: "codex_bengalfox",
            label: "GPT-5.3-Codex-Spark",
          },
        ],
        at: now - 96 * 60,
      },
    };
  },
  // The browser cannot control macOS sleep; accept the command without a platform effect.
  set_awake() {
    return;
  },
  // Sample local and remote MCP servers exercise settings, selection, and composer controls.
  mcp_hub() {
    return mcpHub;
  },
  mcp_save(args) {
    const server = args.server as (typeof mcpHub)[number];
    if (mockCloud().user && mockCatalog().shared[`mcp:${server.id}`]) cloudWrite();
    const at = mcpHub.findIndex((s) => s.id === server.id);
    if (at < 0) mcpHub.push(server);
    else mcpHub[at] = server;
    return mcpHub;
  },
  mcp_remove(args) {
    const state = mockCatalog();
    if (mockCloud().user && state.shared[`mcp:${args.id}`]) {
      cloudWrite(); state.mcp = state.mcp.filter(id => id !== args.id); delete state.shared[`mcp:${args.id}`]; saveMockCatalog(state);
    }
    mcpHub = mcpHub.filter((s) => s.id !== args.id);
    return mcpHub;
  },
  // Simulate probe steps, including 401 responses that expose authentication actions.
  mcp_check(args) {
    const server = args.server as McpServer;
    const url = String(server.config.url ?? "");
    const step = (key: string, ok: boolean, note = "", detail = "") => ({ key, ok, note, detail });
    if (url.includes("notion") || url.includes("capim"))
      return {
        steps: [step("connect", true, "401"), step("oauth", true), step("client", true)],
        probe: { ok: false, auth: true, tools: 0, name: "", detail: "" },
      };
    if (url.includes("quebrado"))
      return {
        steps: [step("connect", false, "", "connection refused")],
        probe: { ok: false, auth: false, tools: 0, name: "", detail: "connection refused" },
      };
    const first = url ? "connect" : "spawn";
    return {
      steps: [
        step(first, true, url ? "200" : ""),
        step("handshake", true, server.id),
        step("tools", true, "9"),
      ],
      probe: { ok: true, auth: false, tools: 9, name: server.id, detail: "" },
    };
  },
  // Browser login marks the sample server connected without launching an OAuth flow.
  mcp_logins() {
    return mcpLogins;
  },
  mcp_login(args) {
    mcpLogins = [...new Set([...mcpLogins, (args.server as McpServer).id])];
    return;
  },
  mcp_logout(args) {
    mcpLogins = mcpLogins.filter((id) => id !== args.id);
    return;
  },
  // Sample servers discoverable from the user's Claude configuration.
  mcp_found() {
    return cliServers;
  },
  // The CLI-inherited base for a Claude workspace, minus servers the hub already has (ADR 0044).
  mcp_inherited(args) {
    const ws = board.workspaces.find((x) => x.id === args.id);
    if (!ws || ws.agent !== "claude") return [];
    return cliServers.filter((s) => !mcpHub.some((h) => h.id === s.id));
  },
  // Plugin hub behavior mirrors MCP hub editing.
  plugin_hub() {
    return pluginHub;
  },
  plugin_save(args) {
    const plugin = args.plugin as Plugin;
    if (mockCloud().user && mockCatalog().shared[`plugins:${plugin.id}`]) cloudWrite();
    const at = pluginHub.findIndex((p) => p.id === plugin.id);
    if (at < 0) pluginHub.push(plugin);
    else pluginHub[at] = plugin;
    return pluginHub;
  },
  plugin_remove(args) {
    const state = mockCatalog();
    if (mockCloud().user && state.shared[`plugins:${args.id}`]) {
      cloudWrite(); state.plugins = state.plugins.filter(p => p.local_id !== args.id); delete state.shared[`plugins:${args.id}`]; saveMockCatalog(state);
    }
    pluginHub = pluginHub.filter((p) => p.id !== args.id);
    return pluginHub;
  },
  // Without disk access, derive the sample plugin name from its source path.
  plugin_look(args) {
    const source = String(args.source ?? "").trim();
    const id = source.replace(/\/+$/, "").split("/").pop() ?? "";
    return { id: id.replace(/\.zip$/, ""), source, note: "" };
  },
  // Sources ending in -plugins simulate marketplaces; other sources install a single plugin.
  plugin_install(args) {
    const url = String(args.source ?? "").trim().replace(/\/+$/, "");
    const name = (url.split(/[/:]/).pop() ?? "").replace(/\.git$/, "");
    if (!name) throw "i18n:" + JSON.stringify({ code: "err.plugin.noSource" });
    const dir = `~/.prometeu/plugins/${name}`;
    const from = url.includes("://") || url.includes("@") ? url : `https://github.com/${url}`;
    if (name.endsWith("-plugins")) {
      return {
        dir,
        saved: false,
        plugins: [
          { id: `${name}-um`, source: `${dir}/plugins/um`, note: "o primeiro do repositório", made: true, from },
          { id: `${name}-dois`, source: `${dir}/plugins/dois`, note: "o segundo do repositório", made: true, from },
        ],
      };
    }
    const plugin: Plugin = { id: name, source: dir, note: `plugin de ${url}`, made: true, from };
    pluginHub = [...pluginHub.filter((p) => p.id !== name), plugin].sort((a, b) => a.id.localeCompare(b.id));
    return { dir, saved: true, plugins: [plugin] };
  },
  plugin_update() {
    return pluginHub;
  },
  plugin_scrap() {
    return;
  },
  // Simulate plugin authoring with timed progress and a final hub entry.
  plugin_make(args) {
    const slug = String(args.name ?? "")
      .toLowerCase()
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "")
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");
    const run = ++pluginRun;
    const steps: Step[] = [
      { kind: "say", text: "Vou começar pelo manifesto." },
      { kind: "file", text: ".claude-plugin/plugin.json" },
      { kind: "file", text: `skills/${slug}/SKILL.md` },
      { kind: "file", text: "hooks/hooks.json" },
    ];
    steps.forEach((step, i) => setTimeout(() => emit("plugin-make", [run, step]), 500 * (i + 1)));
    setTimeout(() => {
      pluginHub.push({ id: slug, source: `~/.prometeu/plugins/${slug}`, note: String(args.ask ?? "").slice(0, 60), made: true });
      emit("plugin-made", [run, ""]);
    }, 500 * (steps.length + 1));
    return { run, slug };
  },
  plugin_make_stop() {
    return;
  },
  set_workspace_plugins(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    // Mirror the backend `axis()`: an absent argument keeps the current selection, an explicit null
    // returns the axis to inherit, and an object replaces it. Standalone skills are their own axis.
    if (target && args.plugins !== undefined) target.plugins = args.plugins;
    writes++;
    // Publish board changes asynchronously so selection feedback cannot depend on an immediate backend echo.
    setTimeout(() => emit("board", board), 0);
    return;
  },
  set_workspace_skills(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target && args.skills !== undefined) target.skills = args.skills;
    writes++;
    setTimeout(() => emit("board", board), 0);
    return;
  },
  set_tools_global(args) {
    board.tools ??= { mcp: null, plugins: null, skills: null };
    if (args.mcp !== undefined) board.tools.mcp = args.mcp;
    if (args.plugins !== undefined) board.tools.plugins = args.plugins;
    if (args.skills !== undefined) board.tools.skills = args.skills;
    writes++;
    setTimeout(() => emit("board", board), 0);
    return;
  },
  // Model and effort changes stop the process. Matching workspace defaults clears the tab override.
  set_tab_choice(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    const tab = target?.tabs.find((t) => t.id === args.tab);
    if (target && tab) {
      const choice = args.choice as Choice;
      const follows =
        choice.agent === target.agent &&
        choice.model === target.model &&
        choice.effort === target.effort;
      tab.choice = follows ? null : choice;
      tab.status = "desligada";
    }
    emit("board", board);
    return;
  },
  set_workspace_mcp(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target && args.mcp !== undefined) target.mcp = args.mcp;
    writes++;
    setTimeout(() => emit("board", board), 0);
    return;
  },
  project_tools(args) {
    const declared = declaredFor(args.id);
    if (!declared.hash) return noProjectTools;
    // The stored decision for the repository, whatever its hash; `pending` follows the current one.
    const decision = (board.tool_trust ?? []).find((t) => t.repo === declared.repo) ?? null;
    const pending = gateOf(declared) === "pending";
    return { ...declared, pending, decision };
  },
  project_tools_trust(args) {
    const declared = declaredFor(args.id);
    // A repository without a declaration has nothing to trust; the command is a no-op.
    if (!declared.hash) return;
    board.tool_trust ??= [];
    const at = Math.floor(Date.now() / 1000);
    const existing = board.tool_trust.find((t) => t.repo === declared.repo);
    if (existing) {
      existing.hash = declared.hash;
      existing.approved = args.approved;
      existing.at = at;
    } else {
      board.tool_trust.push({ repo: declared.repo, hash: declared.hash, approved: args.approved, at });
    }
    emit("board", board);
    return;
  },
  workspace_tools(args) {
    const ws = board.workspaces.find((x) => x.id === args.id);
    if (!ws) throw `i18n:${JSON.stringify({ code: "err.session.noWorkspace" })}`;
    const l = toolLayers(ws);
    return {
      mcp: axisProvenance(l.global.mcp, l.declaredTools.mcp, l.gate, l.own.mcp, l.base, l.mcpUniverse),
      plugins: axisProvenance(l.global.plugins, l.declaredTools.plugins, l.gate, l.own.plugins, [], l.pluginIds),
      skills: axisProvenance(l.global.skills, l.declaredTools.skills, l.gate, l.own.skills, [], l.pluginIds),
    };
  },
  machine() {
    return {
      rss: 822 * 1024 * 1024,
      cpu: 3.4,
      procs: [
        { kind: "app", name: "Prometeu", detail: "", rss: 640 * 1024 * 1024, cpu: 0.8,
          hist: [0.4, 0.6, 1.2, 0.9, 0.7, 2.1, 1.4, 0.8, 0.6, 0.8] },
        { kind: "chat", name: "Tela igual ao Conductor", detail: "Conversa 1", rss: 128 * 1024 * 1024, cpu: 2.2,
          hist: [0, 0, 4.5, 8.2, 6.1, 3.3, 1.2, 2.8, 5.4, 2.2] },
        { kind: "term", name: "Ícone do app", detail: "run", rss: 54 * 1024 * 1024, cpu: 0.4,
          hist: [0.2, 0.3, 0.2, 0.5, 0.4, 0.3, 0.4, 0.4, 0.3, 0.4] },
      ],
      terms: 2,
      ports: [{ id: "0831-1714", title: "Ícone do app", port: 3100 }],
    };
  },
  list_branches() {
    return {
      all: [
        "origin/main", "main", "entire/checkpoints/v1", "manual-sleep-button",
        "dashboard-app-preview", "export-project-zip", "fix/deploy-build-cache",
        "password-reset-crud", "project-renaming", "refactor/railsway-specs-and-lint",
        "origin/entire/checkpoints/v1", "origin/manual-sleep-button",
      ],
      default: "origin/main",
      git: true,
    };
  },
  // Return a preparing workspace immediately, then finish setup on a timer to exercise optimistic creation without Git.
  create_workspace(args) {
    const draft = args.draft;
    const id = `nova-${nextId++}`;
    const repo = String(draft.project).split("/").pop() ?? "repo";
    const fresh = ws(id, draft.project, repo, draft.title || draft.branch, draft.stage, []);
    fresh.branch = draft.branch || "main";
    // Multiple repositories share a parent directory with one worktree each.
    const extras: string[] = draft.extras ?? [];
    if (extras.length) {
      const names = [repo, ...extras.map((p: string) => board.projects.find((x) => x.id === p)?.name ?? p)];
      fresh.worktree = `~/prometeu/worktrees/${names.join("+")}/prometeu-${id}`;
      fresh.repos = [draft.project, ...extras].map((p: string, i: number) => ({
        path: String(p),
        name: names[i],
        worktree: `${fresh.worktree}/${names[i]}`,
        base: "origin/main",
        pr: null,
      }));
    }
    fresh.agent = draft.agent;
    fresh.model = draft.model;
    fresh.effort = draft.effort;
    fresh.issue = draft.issue ?? null;
    fresh.preparing = true;
    board.workspaces.push(fresh);
    emit("board", board);
    // Delay setup like a large Git worktree operation. The fixture failure keyword selects the error state.
    setTimeout(() => {
      fresh.preparing = false;
      if (String(draft.prompt).includes("falha")) {
        fresh.failed = JSON.stringify({
          code: "err.git",
          args: {
            command: "git worktree add",
            cause: `fatal: '${fresh.branch}' is already checked out at '/Users/g/wt/outro'`,
          },
        }).replace(/^/, "i18n:");
        emit("board", board);
        return;
      }
      fresh.tabs = [{ id: `t-${id}`, title: "", status: "pronta", note: null, tokens: null }];
      fresh.active = fresh.tabs[0].id;
      emit("board", board);
    }, 1400);
    return fresh;
  },
  workspace_scripts(args) {
    return scripts[args.id] ?? noScripts;
  },
  dock_state(args) {
    return [...docks]
      .filter(([k]) => k.startsWith(`${args.id}:`))
      .map(([k, alive]) => ({ kind: k.split(":")[1] as DockKind, alive }));
  },
  create_scripts_file() {
    return ".prometeu/settings.toml";
  },
  scripts_prompt() {
    return "Descubra como preparar e como rodar este projeto, e escreva isso em `.prometeu/settings.toml`.";
  },
  /// The browser has no pasteboard; return a plausible attachment so the paste flow stays testable.
  paste_files() {
    return ["/Users/gustavo/.prometeu/attachments/pasted.png"];
  },
  feedback_capture() {
    return "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aHlcAAAAASUVORK5CYII=";
  },
  // The browser mock records the report; no issue is created and nothing leaves the machine.
  // mock:feedbackFailures makes that many attempts fail, as a recoverable delivery error does.
  feedback_send(args) {
    if (!mockCloud().user) throw 'i18n:{"code":"feedback.needAccount"}';
    const attempts = JSON.parse(localStorage.getItem("mock:feedbackAttempts") ?? "[]");
    attempts.push(args.report);
    localStorage.setItem("mock:feedbackAttempts", JSON.stringify(attempts));
    const failures = Number(localStorage.getItem("mock:feedbackFailures") ?? 0);
    if (failures > 0) {
      localStorage.setItem("mock:feedbackFailures", String(failures - 1));
      throw 'i18n:{"code":"feedback.sendError"}';
    }
    return;
  },
  // External links open a separate browser tab.
  open_external(args) {
    window.open(String(args.url), "_blank", "noreferrer");
    return;
  },
  open_run(args) {
    console.log("abrir no navegador: http://localhost:" + ((scripts[args.id] ?? noScripts).port ?? 0));
    return;
  },
  // The interactive fixture shares the inspector script with the native preview.
  browser_open(args) {
    return browser.open(args.id, (scripts[args.id] ?? noScripts).port ?? 3100, url => emit("browser:url", [args.id, url]));
  },
  browser_url(args) {
    return browser.url(args.id);
  },
  browser_navigate(args) {
    return browser.navigate(args.id, args.url);
  },
  browser_back(args) {
    return browser.back(args.id);
  },
  browser_forward(args) {
    return browser.forward(args.id);
  },
  browser_bounds(args) {
    return browser.bounds(args.id, args.w, args.h);
  },
  browser_hide(args) {
    return browser.hide(args.id);
  },
  browser_reload(args) {
    return browser.reload(args.id);
  },
  browser_close(args) {
    return browser.close(args.id);
  },
  browser_inspect(args) {
    return browser.inspect(args.id, args.enabled);
  },
  browser_selection(args) {
    return browser.selection(args.id);
  },
  browser_capture(args) {
    return browser.capture(args.id, args.rect);
  },
  // Use known PR state without invoking gh; only the review workspace exposes a PR.
  pr_open() {
    return;
  },
  // Refreshing PRs preserves sample state without invoking gh.
  refresh_prs() {
    return;
  },
  finish_workspace(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) {
      target.stage = board.stages[board.stages.length - 1];
      target.archived = true;
      target.tabs.forEach((t) => (t.status = "desligada"));
    }
    emit("board", board);
    return;
  },
  cleanup_list() {
    return board.workspaces
      .filter(hasWorktree)
      .map((x, i) => ({
        id: x.id,
        title: x.title,
        repoName: x.repo_name,
        branch: x.branch,
        worktree: x.worktree,
        sizeKb: 2_900_000 - i * 700_000,
        pr: x.repos.find((r) => r.pr)?.pr?.number ?? null,
        // Blocked rows remain selectable and explain their cleanup risk.
        blocked: i === 1 ? 'i18n:{"args":{"n":"3"},"code":"err.cleanup.dirty"}' : null,
      }));
  },
  cleanup_worktree(args) {
    const target = board.workspaces.find((x) => x.id === args.id);
    if (target) target.cleaned = true;
    emit("board", board);
    return;
  },
  open_pr(args) {
    console.log("abrir o PR de " + args.id + " no navegador");
    return;
  },
  pr_prompt() {
    return [
      "Quero abrir um PR deste worktree.",
      "",
      "Há 2 arquivos com mudanças fora de commit. A branch atual é `mock/ajuste`; o alvo é `origin/main`. Ainda não há branch upstream.",
    ].join("\n");
  },
  open_dock(args) {
    const key = `${args.id}:${args.kind}`;
    docks.set(key, true);
    if (args.kind === "setup") {
      setTimeout(() => {
        if (!docks.get(key)) return;
        docks.set(key, false);
        emit("pty", [key, [...new TextEncoder().encode(DONE)]]);
        emit("pty-closed", [key, 0]);
      }, 1500);
    }
    return key;
  },
  close_dock(args) {
    docks.delete(`${args.id}:${args.kind}`);
    return;
  },
  new_tab(args) {
    const workspace = board.workspaces.find(workspace => workspace.id === args.workspace);
    if (!workspace) throw 'i18n:{"code":"err.session.noWorkspace"}';
    if (workspace.cleaned) throw 'i18n:{"code":"err.session.cleaned"}';
    const tab: Tab = { id: crypto.randomUUID(), title: args.prompt.trim(), status: "pronta", note: null, tokens: null,
      choice: args.choice ?? null };
    workspace.tabs.push(tab);
    workspace.active = tab.id;
    emit("board", board);
    if (args.prompt.trim()) sayInto(tab.id, args.prompt.trim());
    return tab;
  },
  resume_tab() {
    return true;
  },
  // Simulate delayed Linear login and publish the same status event as the backend.
  linear_status() {
    return linear;
  },
  // Accept the language command; browser UI translation happens in the frontend.
  set_lang() {
    return undefined;
  },
  linear_connect() {
    linear = { ...linear, busy: true };
    emit("linear", linear);
    return new Promise((done) =>
      setTimeout(() => {
        linear = {
          connected: true,
          busy: false,
          who: { name: "Gustavo Brancaglione", email: "gustavo@exemplo.com", org: "Moabi", org_key: "moabi" },
        };
        emit("linear", linear);
        done(linear);
      }, 1200),
    );
  },
  linear_issues() {
    if (!linear.connected) {
      return Promise.reject(`i18n:${JSON.stringify({ code: "err.linear.off" })}`);
    }
    return new Promise((done) => setTimeout(() => done({ issues: ISSUES, fetched_at: Date.now() / 1000 }), 600));
  },
  linear_open(args) {
    console.log("abrir no Linear:", args.url);
    return;
  },
  linear_disconnect() {
    linear = { connected: false, who: null, busy: false };
    emit("linear", linear);
    return linear;
  },
  add_project(args) {
    const existing = board.projects.find(project => project.path === args.path);
    if (existing) return existing;
    const project = { id: args.path, name: args.path.split("/").pop() ?? args.path, path: args.path };
    board.projects.push(project);
    emit("board", board);
    return project;
  },
  // Mirrors session.rs: the closed tab leaves the board and the first survivor becomes active.
  close_tab(args) {
    const workspace = board.workspaces.find((x) => x.id === args.workspace);
    if (!workspace) return;
    workspace.tabs = workspace.tabs.filter((t) => t.id !== args.tab);
    if (workspace.active === args.tab) workspace.active = workspace.tabs[0]?.id ?? null;
    emit("board", board);
  },
  pty_resize() {},
  remove_workspace() {},
  reveal() {},
};

function call(cmd: string, args: Record<string, any> = {}): unknown {
  if (Object.prototype.hasOwnProperty.call(mockCommands, cmd)) {
    // Tauri dispatches dynamically; each handler is checked against the shared contract above.
    const handler = mockCommands[cmd as IpcCommand] as (args: unknown) => unknown;
    return handler(args);
  }
  switch (cmd) {

    case "plugin:event|listen": {
      const h = w[`_${args.handler}`] as Handler;
      handlers.set(args.event, [...(handlers.get(args.event) ?? []), h]);
      return nextId++;
    }

    // Folder selection returns no sample project. Multiple-file selection supplies sample attachments.
    case "plugin:dialog|open":
      return args.options?.multiple
        ? ["/Users/gustavo/dev/njord/docs/spec.md", "/Users/gustavo/Desktop/tela.png"]
        : null;

    // There is no app bundle version in the browser. Version zero prevents release notes from opening automatically.
    case "plugin:app|version":
      return "0.0.0-mock";

    // No app updater runs in the browser; null means no update.
    case "plugin:updater|check":
      return null;
    default:
      return null;
  }
}

/* ---------- mock relay ---------- */

/// A fixed two-member team enables collaboration UI without a relay. mock.presence(false) simulates an offline member.
let marcusOnline = true;
const fakes: team.SocketLike[] = [];
/// Marcus's shared workspace provides a running remote conversation.
const marcusShare = (): Share => ({
  id: "ws-marcus",
  title: "Arquivar todos os concluídos",
  repo_name: "capim-backend",
  branch: "fix/archive-completed-todos",
  stage: "Fazendo",
  issue: { identifier: "CAP-218", title: "Digest semanal zera concluídos", url: "https://linear.app/x/issue/CAP-218" },
  active: "mt1",
  tabs: [
    { id: "mt1", title: "", status: "rodando", note: "Edit src/todos/complete.ts", tokens: 41_200 },
    { id: "mt2", title: "testes", status: "pronta", note: null, tokens: 8_300 },
  ],
  sizes: { mt1: [100, 30], mt2: [100, 30] },
  audience: null,
});
function fakeSocket(url: string): team.SocketLike {
  const socket = simulatedSocket(url, SAMPLE, marcusShare(), () => marcusOnline);
  fakes.push(socket);
  return socket;
}
// VITE_RELAY connects browsers to a real relay while retaining the mock backend for end-to-end sharing tests.
if (!(import.meta as unknown as { env?: Record<string, string | undefined> }).env?.VITE_RELAY) {
  team.useTransport({
    needsRelay: false,
    socket: fakeSocket,
    create: async () => ({
      team: "timeDeMentira",
      secret: "segredoDeMentira",
      member: "eu_mock",
      credential: "c".repeat(43),
    }),
    enroll: async () => ({ member: "eu_mock", credential: "c".repeat(43) }),
  });
}

w.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  transformCallback(cb: Handler) {
    const id = nextId++;
    w[`_${id}`] = cb;
    return id;
  },
  unregisterCallback(id: number) {
    delete w[`_${id}`];
  },
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise.resolve(call(cmd, args)),
};

// Console shortcut for file-drop testing: mock.drop([...]).
w.mock = {
  usage: (payload: unknown) => emit("usage", payload),
  accountError: (error: string) => emit("account-error", error),
  /// Inject a conversation line as if the process emitted it.
  line: (tab: string, o: unknown) => pushLine(tab, o),
  /// Expose the number of MCP/plugin writes received by the mock.
  writes: () => writes,
  /// Expose team state for external UI assertions.
  team: () => ({ status: team.status(), remotes: team.remotes() }),
  presence: (online: boolean) => {
    marcusOnline = online;
    for (const s of fakes) (s as unknown as { presence: () => void }).presence();
  },
  /// Simulate Tauri file drops using macOS logical window coordinates; see dropTarget in main.ts.
  drop: (paths: string[], x = innerWidth / 2, y = innerHeight / 2, dropX = x, dropY = y) => {
    emit("file-drag", { type: "over", paths: [], position: { x, y } });
    emit("file-drag", { type: "drop", paths, position: { x: dropX, y: dropY } });
  },
  over: (position: { x: number; y: number }) => emit("file-drag", { type: "over", paths: [], position }),
  drag: (type: "enter" | "over" | "drop" | "leave" | "pending" | "received", payload: { paths?: string[]; position?: { x: number; y: number }; id?: string; error?: string } = {}) => emit("file-drag", { type, paths: [], ...payload }),
};
