/// Back falso para o navegador puro (`npm run dev` e abrir localhost:1420):
/// a UI inteira roda com dados de amostra, sem subir o Tauri. Só entra quando
/// `window.__TAURI_INTERNALS__` não existe — dentro do app não é carregado.
import { encodeLive, encodeSnapshot } from "../relay/src/protocol";
import { LegacyConversationAdapter } from "./conversation-legacy";
import * as team from "./team";
import { hasWorktree, type Board, type Choice, type Issue, type LinearStatus, type McpServer, type Plugin, type Pr, type Scripts, type Workspace } from "./types";

type Handler = (e: { event: string; id: number; payload: unknown }) => void;
const handlers = new Map<string, Handler[]>();
let nextId = 1;
const w = window as unknown as Record<string, unknown>;

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
  // Modelo e esforço de mentira: é o que a caixa de escrever mostra embaixo,
  // e sem eles o rodapé da conversa não teria o que desenhar.
  model: "opus[1m]",
  effort: "high",
  mcp: null,
  plugins: null,
  port: 3100,
  issue: null,
  cleaned: false,
  shared: false,
  audience: null,
  preparing: false,
  failed: null,
  remote: null,
  tabs,
  active: tabs[0]?.id ?? null,
});

const board: Board = {
  stages: ["Preparando", "Fazendo", "Code review", "Travado", "Feito"],
  projects: [
    { id: "p1", name: "njord", path: "/Users/gustavo/dev/njord" },
    { id: "p2", name: "prometeu", path: "/Users/gustavo/dev/prometeu" },
  ],
  workspaces: [
    ws("sessao-0929", "p1", "njord", "Ola", "Fazendo", [
      { id: "t1", title: "conversa 1", status: "pronta", note: null, tokens: 57_000 },
      // Aba que nasceu com outro modelo que o do workspace: é o rodapé da
      // conversa mostrando o dela, e não o das irmãs.
      { id: "t2", title: "conversa 2", status: "pronta", note: null, tokens: 112_400, pending_prompt: "O que tem nesse projeto aqui de legal?", choice: { agent: "claude", model: "sonnet", effort: "medium" } },
    ]),
    ws("ui-2231", "p2", "prometeu", "Tela igual ao Conductor", "Fazendo", [
      { id: "t3", title: "conversa 1", status: "rodando", note: "Edit src/style.css", tokens: 23_800 },
    ]),
    // Dois repositórios na mesma branch: é aqui que a lista de mudanças ganha
    // uma seção por repo.
    Object.assign(
      ws("portal-1217", "p2", "prometeu", "Contratação pelo portal", "Fazendo", [
        { id: "t9", title: "conversa 1", status: "rodando", note: "Edit app/models/entry.rb", tokens: 31_000 },
      ]),
      {
        worktree: "~/prometeu/worktrees/prometeu+njord/prometeu-portal-1217",
        // Um PR por repositório: o do njord já entrou, o do prometeu ainda
        // não — e é por isso que a barra não oferece "Concluir".
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
    // Uma pergunta esperando você é justamente o que vira novidade.
    Object.assign(
      ws("icone-2140", "p2", "prometeu", "Ícone do app", "Code review", [
        { id: "t4", title: "conversa 1", status: "querendo", note: "Qual tamanho de ícone você quer gerar?", tokens: 8_100 },
      ]),
      { unread: true, pr: { number: 42, title: "feat(quadro): ícone do app", isDraft: false, state: "OPEN" } },
    ),
    // PR mergeado: é este que mostra o selo no card e o "Concluir" na barra.
    Object.assign(
      ws("dock-1130", "p2", "prometeu", "Porta do dock por worktree", "Code review", [
        { id: "t5", title: "conversa 1", status: "pronta", note: null, tokens: 44_200 },
      ]),
      { pr: { number: 40, title: "feat(dock): porta por worktree", isDraft: false, state: "MERGED" } },
    ),
    // Arquivado que ainda ocupa disco: é ele que a folha de limpeza lista.
    Object.assign(
      ws("linear-0912", "p1", "njord", "Conectar o Linear", "Feito", [
        { id: "t7", title: "conversa 1", status: "desligada", note: null, tokens: 66_000 },
      ]),
      { archived: true, pr: { number: 8, title: "feat: conectar o Linear", isDraft: false, state: "MERGED" } },
    ),
    Object.assign(
      ws("porta-1751", "p1", "njord", "Porta ocupada no setup", "Travado", [
        { id: "t8", title: "conversa 1", status: "desligada", note: null, tokens: 12_000 },
      ]),
      { archived: true },
    ),
    // Worktree devolvido: o card que sobrou de um trabalho que acabou.
    Object.assign(
      ws("idioma-1348", "p2", "prometeu", "O app fala inglês", "Feito", [
        { id: "t6", title: "conversa 1", status: "desligada", note: null, tokens: 91_000 },
      ]),
      {
        archived: true,
        cleaned: true,
        pr: { number: 17, title: "feat(idioma): o app fala inglês", isDraft: false, state: "MERGED" },
      },
    ),
  ],
};

// O PR é do repositório, não do workspace: o que as amostras acima escrevem
// solto vai para o principal — o mesmo caminho do `revive` do back.
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
  // Excel em pt-BR: `;` de separador, vírgula decimal, campo com quebra dentro.
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

/// As mudanças do segundo repositório do workspace de dois: o outro lado da
/// mesma feature, com histórico próprio.
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

/// Um transcript legado de mentira, mantido como fixture de rollback. Eventos
/// novos do mock são normalizados para V1 antes de chegar à tela ou ao relay.
const line = (o: unknown) => JSON.stringify(o);
const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();
const SAMPLE =
  [
    // A resposta ao `initialize` que o back manda ao subir o processo, e o
    // `init` que vem depois da primeira fala (o `color` é de terminal, sai).
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

/// Os scripts de cada workspace. Um repo com tudo declarado e dois runs, para a
/// lista do botão ter o que mostrar; e um sem nada, que é o estado que o convite
/// de "Adicionar script" existe para cobrir.
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

/// Os docks que existem, pela mesma chave do Rust: `<workspace>:<tipo>`, e se
/// o processo está vivo. O setup "termina" sozinho pouco depois de subir, para
/// a tela do que já rodou existir no navegador.
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
  // Um segundo time: é o que faz a linha de filtro por time aparecer.
  issue("INF-88", "Runner self-hosted cai depois de duas horas ocioso", 1, DOING, "Infra", 3),
  issue("INF-72", "Assinar o .dmg no CI sem pedir a senha do Keychain", 3, TODO, "Infra", 52),
];

/// As linhas de cada conversa de mentira, numeradas como o back numera: o que
/// `chat_send` escreve entra aqui, sai pelo evento `chat` com o número, e o
/// `chat_snapshot` devolve o mesmo par — para o compartilhamento poder ser
/// testado contra um relay de verdade sem subir o Tauri.
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
  for (const event of adapter.translate(o)) {
    const text = line(event);
    if (keep) s.text += text + "\n";
    s.seq += 1;
    emit("chat", [tab, text, s.seq]);
  }
}

/// Uma fala: entra como o back a ecoa, e o agente de mentira responde
/// letra a letra. Fala com "plano" vira um plano esperando aprovação; com
/// "pergunta", uma pergunta com opções — os dois cards que existem para ver.
let msgN = 0;
/// O que o `/context` devolve (um de verdade, encurtado).
/// O hub de MCP do navegador. Muda com o que se cadastra e remove na tela —
/// é o que deixa a seção de Configurações ser usada de verdade sem back.
let mcpHub: McpServer[] = [
  { id: "capim-ds", config: { type: "stdio", command: "npx", args: ["-y", "@capim/ds-mcp"], env: {} }, note: "design system" },
  { id: "notion", config: { type: "http", url: "https://mcp.notion.com/mcp" }, note: "" },
  { id: "linear-server", config: { type: "http", url: "https://mcp.linear.app/mcp" }, note: "capim-backend" },
];

/// Quantas vezes a escolha de MCP ou de plugin foi gravada. É o que um teste
/// olha para saber se marcar três coisas seguidas virou uma gravação só.
let writes = 0;

/// Uma linha do que o agente que escreve o plugin está fazendo.
type Step = { kind: string; text: string };

/// O hub de plugins do navegador: um instalado por marketplace e um que
/// alguém está escrevendo, que são os dois casos que a lista desenha.
let pluginHub: Plugin[] = [
  { id: "caveman", source: "~/.prometeu/plugins/caveman", note: "fala curto e sem enfeite", made: true, from: "https://github.com/JuliusBrussee/caveman" },
  { id: "ponytail", source: "~/dev/ponytail", note: "em construção" },
];

/// A corrida da criação, no navegador: os passos saem de um relógio, e não de
/// um agente.
let pluginRun = 0;

/// Em quais servidores já se entrou, no navegador.
let mcpLogins: string[] = [];

const CONTEXT_MD = "## Context Usage\n\n**Model:** claude-fable-5  \n**Tokens:** 20.2k / 1m (2%)\n\n### Estimated usage by category\n\n| Category | Tokens | Percentage |\n|----------|--------|------------|\n| System prompt | 4k | 0.4% |\n| System tools | 6.5k | 0.7% |\n| MCP tools (deferred) | 14.3k | 1.4% |\n| System tools (deferred) | 14k | 1.4% |\n| Custom agents | 368 | 0.0% |\n| Skills | 3k | 0.3% |\n| Messages | 6.3k | 0.6% |\n| Compact buffer | 3k | 0.3% |\n| Free space | 976.8k | 97.7% |\n\n### MCP Tools\n\n| Tool | Server | Tokens |\n|------|--------|--------|\n| mcp__capim-ds__get_components | capim-ds | 250 |\n| mcp__capim-ds__get_foundations | capim-ds | 209 |\n| mcp__capim-ds__get_icon_details | capim-ds | 168 |\n| mcp__capim-ds__get_illustration_details | capim-ds | 194 |\n| mcp__capim-ds__get_logo_details | capim-ds | 171 |\n| mcp__capim-ds__list_components | capim-ds | 130 |\n| mcp__capim-ds__list_icons | capim-ds | 107 |\n| mcp__capim-ds__list_illustrations | capim-ds | 120 |\n| mcp__capim-ds__list_logos | capim-ds | 112 |\n| mcp__claude_ai_Google_Drive__copy_file | claude_ai_Google_Drive | 444 |\n| mcp__claude_ai_Google_Drive__create_file | claude_ai_Google_Drive | 965 |\n| mcp__claude_ai_Google_Drive__download_file_content | claude_ai_Google_Drive | 433 |\n| mcp__claude_ai_Google_Drive__get_file_metadata | claude_ai_Google_Drive | 237 |\n| mcp__claude_ai_Google_Drive__get_file_permissions | claude_ai_Google_Drive | 143 |\n\n### Custom Agents\n\n| Agent Type | Source | Tokens |\n|------------|--------|--------|\n| caveman:cavecrew-builder | Plugin | 134 |\n| caveman:cavecrew-investigator | Plugin | 112 |\n| caveman:cavecrew-reviewer | Plugin | 122 |\n\n### Skills\n\n| Skill | Source | Tokens |\n|-------|--------|--------|\n| para-memory-files | User | ~190 |\n| caveman:cavecrew | Plugin (caveman) | ~190 |\n| caveman:caveman | Plugin (caveman) | ~140 |\n| caveman:caveman-commit | Plugin (caveman) | ~120 |\n| caveman:caveman-compress | Plugin (caveman) | ~120 |\n| caveman:caveman-help | Plugin (caveman) | ~70 |\n| caveman:caveman-review | Plugin (caveman) | ~110 |\n| caveman:caveman-stats | Plugin (caveman) | ~90 |\n| dataviz | Built-in | ~380 |\n| update-config | Built-in | ~240 |\n| keybindings-help | Built-in | ~80 |\n| code-review | Built-in | ~270 |\n| simplify | Built-in | ~60 |\n| fewer-permission-prompts | Built-in | ~60 |\n| loop | Built-in | ~120 |\n| schedule | Built-in | ~130 |\n| claude-api | Built-in | ~360 |\n| workflow-authoring | Built-in | ~80 |\n| run | Built-in | ~120 |\n| init | Built-in | ~20 |\n| security-review | Built-in | ~30 |";
function sayInto(tab: string, text: string) {
  pushLine(tab, { type: "user", message: { role: "user", content: text }, ts: Date.now() });
  const id = `mm${++msgN}`;
  if (text.trim() === "/context") {
    pushLine(tab, { type: "assistant", message: { id: `${id}x`, model: "<synthetic>", role: "assistant", content: [{ type: "text", text: CONTEXT_MD }] } });
    pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 0 });
    return;
  }
  if (text.trim() === "/compact") {
    // Demora de verdade (um minuto, às vezes mais): a legenda fica o tempo
    // todo, e no fim vêm o tamanho, o resumo e o eco do comando.
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
      // Duas tarefas em segundo plano: a legenda diz quantas, o card gira
      // até o aviso de que acabou, e o agente reage sozinho ao aviso.
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

/// Uma resposta a card: a ferramenta "roda" e o turno termina.
function controlInto(tab: string, frame: Record<string, any>) {
  if (frame.v !== 1 || frame.type !== "request.respond") return;
  const req = String(frame.requestId ?? "");
  const id = req.replace(/^req-/, "");
  const denied = frame.response?.outcome === "deny";
  pushLine(tab, { type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: `tu-${id}`, content: denied ? String(frame.response.message) : "ok", is_error: denied }] } });
  pushLine(tab, { type: "assistant", message: { id: `${id}b`, role: "assistant", content: [{ type: "text", text: denied ? "Certo, vou mudar o plano." : "Combinado. Seguindo." }] } });
  pushLine(tab, { type: "result", subtype: "success", is_error: false, duration_ms: 400 });
}

/// Os workspaces compartilhados, entre recargas — o `shared` e a `audience`
/// do board.json.
const SHARED = "mock:shared";
for (const [id, audience] of JSON.parse(localStorage.getItem(SHARED) ?? "[]") as [string, string[] | null][]) {
  const ws = board.workspaces.find((x) => x.id === id);
  if (ws) {
    ws.shared = true;
    ws.audience = audience;
  }
}

function emit(event: string, payload: unknown) {
  handlers.get(event)?.forEach((h) => h({ event, id: nextId++, payload }));
}

function call(cmd: string, args: Record<string, any> = {}): unknown {
  switch (cmd) {
    case "plugin:event|listen": {
      const h = w[`_${args.handler}`] as Handler;
      handlers.set(args.event, [...(handlers.get(args.event) ?? []), h]);
      return nextId++;
    }
    case "load_board":
      return board;
    // O time fica no localStorage aqui, para sobreviver a recarregar a aba —
    // no app é o `team.json` do back.
    case "team_config":
      return { config: JSON.parse(localStorage.getItem("mock:team") ?? "null"), default_name: "Você" };
    case "team_config_set":
      if (args.config) localStorage.setItem("mock:team", JSON.stringify(args.config));
      else localStorage.removeItem("mock:team");
      return;
    case "pty_buffer": {
      const s = String(args.session);
      const text = docks.get(s) === false ? SCRIPT_OUT + DONE : SCRIPT_OUT;
      return [...new TextEncoder().encode(text)];
    }
    // Como o back: a última linha diz se há turno em andamento.
    case "chat_buffer":
      return scrollOf(String(args.session)).text + line({ v: 1, type: "session.state", at: Date.now(), state: "ready" }) + "\n";
    case "chat_snapshot": {
      const s = scrollOf(String(args.session));
      return { text: s.text, seq: s.seq };
    }
    // A conversa de mentira responde ao que recebe: é o que deixa ver a fala
    // de um colega chegar e voltar.
    case "chat_send":
      sayInto(String(args.session), String(args.text));
      return;
    case "chat_control":
    case "chat_control_remote":
      controlInto(String(args.session), args.frame as Record<string, any>);
      return;
    case "pty_write":
      return;
    // Um grupo por repositório, como o back: o primeiro leva as mudanças de
    // sempre, e o segundo (só no workspace de dois repos) as do outro lado.
    case "workspace_diff": {
      const target = board.workspaces.find((x) => x.id === args.id);
      return (target?.repos ?? []).map((r, i) => {
        const files = i === 0 ? changes : changes2;
        const base = i === 0 ? "origin/main" : "origin/develop";
        // O primeiro tem commit que ainda não foi para o remoto e o segundo
        // não: é o par que faz o resumo dizer as duas coisas.
        const ahead = i === 0 ? 3 : 1;
        return { name: r.name, base, ahead, unpushed: i === 0 ? 1 : 0, dirty: files.filter((f) => f.dirty).length, files };
      });
    }
    case "list_dir":
      return tree[args.rel ?? ""] ?? [];
    // Salvar do viewer: escreve por cima, e a próxima leitura já vê. A guarda
    // de corrida (`was`) é do back de verdade; aqui ninguém escreve por baixo.
    case "write_file":
      files[args.rel] = args.text;
      return;
    // Como o back: o que combina com o que foi digitado, do mais raso para o
    // mais fundo. A árvore do mock é rasa, então basta o caminho conter o que
    // se escreveu.
    case "find_paths": {
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
    }
    case "file_stamp":
      return "0";
    case "read_bytes":
      if (args.rel in files) return new TextEncoder().encode(files[args.rel]).buffer;
      throw `i18n:${JSON.stringify({ code: "err.session.binary" })}`;
    case "read_file":
      if (args.rel in files) return files[args.rel];
      // Como o back de verdade: código, e não frase. O front traduz.
      throw args.rel.endsWith(".lock")
        ? `i18n:${JSON.stringify({ code: "err.session.tooBig", args: { kb: 2140 } })}`
        : `i18n:${JSON.stringify({ code: "err.session.binary" })}`;
    case "rename_workspace": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) target.title = args.title;
      emit("board", board);
      return;
    }
    case "rename_tab": {
      const target = board.workspaces.find((x) => x.id === args.workspace);
      const tab = target?.tabs.find((t) => t.id === args.tab);
      if (tab) tab.title = args.title;
      emit("board", board);
      return;
    }
    // Como no Rust: worktree devolvido não tem branch para ler.
    case "workspace_branch": {
      const target = board.workspaces.find((x) => x.id === args.id);
      return target && !target.cleaned ? target.branch : null;
    }
    case "set_stage": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) target.stage = args.stage;
      emit("board", board);
      return;
    }
    case "set_shared": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) {
        target.shared = args.shared;
        target.audience = args.shared ? args.audience : null;
      }
      // Como o `board.json` do back: recarregar a página não desfaz o que foi
      // compartilhado, senão o dono que volta volta sem nada compartilhado.
      localStorage.setItem(SHARED, JSON.stringify(board.workspaces.filter((x) => x.shared).map((x) => [x.id, x.audience])));
      emit("board", board);
      return;
    }
    case "pin_workspace": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) target.pinned = args.pinned;
      emit("board", board);
      return;
    }
    case "set_unread": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) target.unread = args.unread;
      emit("board", board);
      return;
    }
    case "look_at": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target?.unread) {
        target.unread = false;
        emit("board", board);
      }
      return;
    }
    case "archive_workspace": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) target.archived = args.archived;
      // Como no Rust: arquivar derruba os processos das abas.
      if (target && args.archived) target.tabs.forEach((t) => (t.status = "desligada"));
      emit("board", board);
      return;
    }
    // O catálogo do Codex, como o CLI o entrega. Fixo aqui: no navegador não há
    // `codex` para perguntar, e o dropdown com os dois agentes é justamente o
    // que se quer ver.
    case "agents":
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
    // O catálogo vivo do Claude Code, como o `list_models` o entrega depois do
    // filtro do back. O Haiku sem escada é de verdade: o CLI não publica
    // esforço para ele.
    case "claude_models":
      return [
        { id: "opus[1m]", label: "Opus (1M context)", efforts: ["low", "medium", "high", "xhigh", "max"] },
        { id: "claude-fable-5[1m]", label: "Fable", efforts: ["low", "medium", "high", "xhigh", "max"] },
        { id: "sonnet", label: "Sonnet", efforts: ["low", "medium", "high", "xhigh", "max"] },
        { id: "haiku", label: "Haiku", efforts: [] },
      ];
    // A cota dos dois agentes, com números parecidos com os de um dia de
    // trabalho: é o que faz a faixa de baixo aparecer no navegador.
    case "usage": {
      const now = Math.floor(Date.now() / 1000);
      return {
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
    }
    // O que o app custaria à máquina num dia comum: ele mesmo, uma conversa e
    // um `npm run dev` de pé.
    // No navegador não há Mac para segurar acordado: guarda e devolve.
    case "set_awake":
      return null;
    // O hub de MCP com o que uma máquina de trabalho costuma ter: um servidor
    // que roda aqui e dois remotos. É o bastante para ver o seletor com lista,
    // a linha de cada tipo em Configurações e o botão da conversa.
    case "mcp_hub":
      return mcpHub;
    case "mcp_save": {
      const server = args.server as (typeof mcpHub)[number];
      const at = mcpHub.findIndex((s) => s.id === server.id);
      if (at < 0) mcpHub.push(server);
      else mcpHub[at] = server;
      return mcpHub;
    }
    case "mcp_remove":
      mcpHub = mcpHub.filter((s) => s.id !== args.id);
      return mcpHub;
    // O exame do servidor. No navegador não há servidor para apertar a mão:
    // devolve os passos que cada caso daria — inclusive o 401, que é o que
    // muda o que a tela oferece depois.
    case "mcp_check": {
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
    }
    // No navegador não há navegador para abrir dentro do navegador: entrar
    // marca o servidor como conectado e pronto.
    case "mcp_logins":
      return mcpLogins;
    case "mcp_login":
      mcpLogins = [...new Set([...mcpLogins, (args.server as McpServer).id])];
      return null;
    case "mcp_logout":
      mcpLogins = mcpLogins.filter((id) => id !== args.id);
      return null;
    // O que haveria para importar do `~/.claude.json` desta máquina.
    case "mcp_found":
      return [
        { id: "metabase", config: { type: "http", url: "https://metabase.exemplo/mcp" }, note: "capim-backend" },
        { id: "n8n", config: { type: "stdio", command: "npx", args: ["-y", "n8n-mcp"], env: {} }, note: "" },
      ];
    // O hub de plugins, do mesmo jeito que o de MCP.
    case "plugin_hub":
      return pluginHub;
    case "plugin_save": {
      const plugin = args.plugin as Plugin;
      const at = pluginHub.findIndex((p) => p.id === plugin.id);
      if (at < 0) pluginHub.push(plugin);
      else pluginHub[at] = plugin;
      return pluginHub;
    }
    case "plugin_remove":
      pluginHub = pluginHub.filter((p) => p.id !== args.id);
      return pluginHub;
    // No navegador não há pasta para ler: o nome sai do fim do caminho, que é
    // o que o `plugin.json` costuma dizer mesmo.
    case "plugin_look": {
      const source = String(args.source ?? "").trim();
      const id = source.replace(/\/+$/, "").split("/").pop() ?? "";
      return { id: id.replace(/\.zip$/, ""), source, note: "" };
    }
    // Instalar, sem git nenhum: o repositório cujo nome termina em `-plugins`
    // faz as vezes de marketplace, que é o caso em que a folha pergunta qual;
    // qualquer outro é um plugin só, e entra direto.
    case "plugin_install": {
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
    }
    case "plugin_update":
      return pluginHub;
    case "plugin_scrap":
      return;

    // Criar um plugin, sem agente nenhum: os passos chegam de meio em meio
    // segundo, e no fim ele está no hub — que é o que a folha precisa mostrar.
    case "plugin_make": {
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
    }
    case "plugin_make_stop":
      return;
    case "set_workspace_plugins": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) {
        target.plugins = args.plugins as string[] | null;
        target.tabs.forEach((t) => (t.status = "desligada"));
      }
      writes++;
      // No app o quadro volta pela ponte, um tique depois — e é essa volta que
      // o seletor não pode ficar esperando para mover a marca.
      setTimeout(() => emit("board", board), 0);
      return;
    }
    // Trocar o modelo ou o esforço de uma conversa de pé: a escolha entra na
    // aba e o processo cai, como no Rust — e escolher de volta o do workspace
    // apaga a escolha, para a aba voltar a acompanhá-lo.
    case "set_tab_choice": {
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
    }
    case "set_workspace_mcp": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) {
        target.mcp = args.mcp as string[] | null;
        // Como no Rust: trocar de ferramenta derruba os processos das abas, e a
        // próxima fala as levanta com a lista nova.
        target.tabs.forEach((t) => (t.status = "desligada"));
      }
      writes++;
      setTimeout(() => emit("board", board), 0);
      return;
    }
    case "machine":
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
    case "list_branches":
      return {
        all: [
          "origin/main", "main", "entire/checkpoints/v1", "manual-sleep-button",
          "dashboard-app-preview", "export-project-zip", "fix/deploy-build-cache",
          "password-reset-crud", "project-renaming", "refactor/railsway-specs-and-lint",
          "origin/entire/checkpoints/v1", "origin/manual-sleep-button",
        ],
        default: "origin/main",
      };
    // O lançador inteiro funciona no navegador, e o workspace novo nasce sem
    // script nenhum — que é o estado em que a aba Setup tem algo a dizer.
    // Criar é otimista no back de verdade: o card volta na hora, sem aba, e o
    // worktree monta atrás. O mock imita isso — com um relógio no lugar do
    // `git worktree add` — porque é o único jeito de a tela de montagem existir
    // fora do Tauri, que é onde ela é desenhada.
    case "create_workspace": {
      const draft = args.draft;
      const id = `nova-${nextId++}`;
      const repo = String(draft.project).split("/").pop() ?? "repo";
      const fresh = ws(id, draft.project, repo, draft.title || draft.branch, draft.stage, []);
      fresh.branch = draft.branch || "main";
      // Mais de um repositório: a pasta que os reúne, com um worktree de cada.
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
      // O tempo de um `git worktree add` num repositório grande. Pedido com
      // "falha" escrito não monta: é como se olha a outra metade desta tela
      // sem precisar de um repositório em que o `git` realmente recuse.
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
        fresh.tabs = [{ id: `t-${id}`, title: "conversa", status: "pronta", note: null, tokens: null }];
        fresh.active = fresh.tabs[0].id;
        emit("board", board);
      }, 1400);
      return fresh;
    }
    case "workspace_scripts":
      return scripts[args.id] ?? noScripts;
    case "dock_state":
      return [...docks]
        .filter(([k]) => k.startsWith(`${args.id}:`))
        .map(([k, alive]) => ({ kind: k.split(":")[1], alive }));
    case "create_scripts_file":
      return ".prometeu/settings.toml";
    case "scripts_prompt":
      return "Descubra como preparar e como rodar este projeto, e escreva isso em `.prometeu/settings.toml`.";
    // No navegador puro, abrir de fora é abrir uma aba do próprio navegador.
    case "open_external":
      window.open(String(args.url), "_blank", "noreferrer");
      return null;
    case "open_run":
      console.log("abrir no navegador: http://localhost:" + ((scripts[args.id] ?? noScripts).port ?? 0));
      return null;
    // A webview nativa não existe fora do Tauri: a aba abre com o buraco vazio.
    case "browser_open":
      return (scripts[args.id] ?? noScripts).port ?? 3100;
    case "browser_url":
      return "http://localhost:" + ((scripts[args.id] ?? noScripts).port ?? 3100) + "/";
    case "browser_navigate":
      console.log("navegar para:", args.url);
      return null;
    case "browser_back":
    case "browser_forward":
    case "browser_bounds":
    case "browser_hide":
    case "browser_reload":
    case "browser_close":
      return null;
    // No navegador não há `gh`: o PR que o quadro já sabe é o que ele mostra,
    // e só o workspace em code review tem — é assim que se vê o botão
    // aparecendo num e não no outro.
    case "pr_open":
      return null;
    // No navegador não há `gh`: o que o quadro já sabe é o que ele continua
    // sabendo.
    case "refresh_prs":
      return null;
    case "finish_workspace": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) {
        target.stage = board.stages[board.stages.length - 1];
        target.archived = true;
        target.tabs.forEach((t) => (t.status = "desligada"));
      }
      emit("board", board);
      return;
    }
    case "cleanup_list":
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
          // Um bloqueado na lista é o que mostra a linha em vermelho com o
          // motivo — e ela dá para marcar assim mesmo.
          blocked: i === 1 ? 'i18n:{"args":{"n":"3"},"code":"err.cleanup.dirty"}' : null,
        }));
    case "cleanup_worktree": {
      const target = board.workspaces.find((x) => x.id === args.id);
      if (target) target.cleaned = true;
      emit("board", board);
      return;
    }
    case "open_pr":
      console.log("abrir o PR de " + args.id + " no navegador");
      return null;
    case "pr_prompt":
      return [
        "Quero abrir um PR deste worktree.",
        "",
        "Há 2 arquivos com mudanças fora de commit. A branch atual é `mock/ajuste`; o alvo é `origin/main`. Ainda não há branch upstream.",
      ].join("\n");
    case "open_dock": {
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
    }
    case "close_dock":
      docks.delete(`${args.id}:${args.kind}`);
      return;
    case "new_tab":
      return { id: "t1" };
    case "resume_tab":
      return true;
    // Pasta (projeto novo) não tem o que devolver no navegador. Arquivos, sim:
    // dois de mentira, para o clipe do lançador ter o que mostrar.
    case "plugin:dialog|open":
      return args.options?.multiple
        ? ["/Users/gustavo/dev/njord/docs/spec.md", "/Users/gustavo/Desktop/tela.png"]
        : null;
    // O Linear de mentira: conectar demora um pouco, como o navegador demora,
    // e avisa pelo mesmo evento que o back avisa.
    case "linear_status":
      return linear;
    // O back de verdade guarda o idioma para as poucas frases que escreve
    // inteiras; aqui não há nenhuma, mas o comando existe dos dois lados.
    case "set_lang":
      return undefined;

    case "linear_connect":
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
    case "linear_issues":
      if (!linear.connected) {
        return Promise.reject(`i18n:${JSON.stringify({ code: "err.linear.off" })}`);
      }
      return new Promise((done) => setTimeout(() => done({ issues: ISSUES, fetched_at: Date.now() / 1000 }), 600));
    case "linear_open":
      console.log("abrir no Linear:", args.url);
      return null;
    case "linear_disconnect":
      linear = { connected: false, who: null, busy: false };
      emit("linear", linear);
      return linear;
    // Fora do Tauri não existe bundle para perguntar a versão. Dizer isso na
    // tela é melhor que repetir aqui um número que envelhece sozinho. Como
    // versão ele vale 0.0.0, e é por isso que a folha de novidades não abre
    // sozinha aqui: nenhuma versão do changelog é menor ou igual a ela.
    case "plugin:app|version":
      return "0.0.0-mock";
    // O updater também não tem o que fazer aqui: `null` é "nada novo", que é
    // a resposta honesta para uma aba de navegador.
    case "plugin:updater|check":
      return null;
    default:
      return null;
  }
}

/* ---------- o relay de mentira ---------- */

/// Um time com dois colegas fixos, para mexer na tela sem relay: o `welcome`
/// chega meio segundo depois de conectar, `me` troca o nome, e o resto é
/// silêncio. `mock.presence(false)` derruba um colega para ver a lista mudar.
let marcusOnline = true;
const fakes: team.SocketLike[] = [];
const enc = new TextEncoder();
/// O workspace que o Marcus compartilhou: uma conversa rodando. É o que o
/// quadro mostra em "Do time".
const marcusShare = () => ({
  id: "ws-marcus",
  title: "Arquivar todos os concluídos",
  repo_name: "capim-backend",
  branch: "fix/archive-completed-todos",
  stage: "Fazendo",
  issue: { identifier: "CAP-218", title: "Digest semanal zera concluídos", url: "https://linear.app/x/issue/CAP-218" },
  active: "mt1",
  tabs: [
    { id: "mt1", title: "conversa 1", status: "rodando", note: "Edit src/todos/complete.ts", tokens: 41_200 },
    { id: "mt2", title: "testes", status: "pronta", note: null, tokens: 8_300 },
  ],
  sizes: { mt1: [100, 30], mt2: [100, 30] },
  audience: null,
  owner: "marcus",
  online: marcusOnline,
});
function fakeSocket(url: string): team.SocketLike {
  const u = new URL(url);
  const me = u.searchParams.get("m") ?? "eu";
  let name = u.searchParams.get("n") ?? "Você";
  let seq = 1;
  let ticking = 0;
  let attached: string | null = null;
  /// As notas de mentira, por workspace. Nascem com uma do Marcus no que ele
  /// compartilhou, para o painel ter o que mostrar de cara.
  const notes = new Map<string, unknown[]>([
    [
      "ws-marcus",
      [
        {
          id: "1-a",
          ws: "ws-marcus",
          author: "marcus",
          text: `Completar um todo agora carimba \`completed_at\` em vez de apagar a linha. @${name} a chamada que sobrou é sua: manter o histórico na tabela de todos, ou mover para uma tabela só delas?`,
          mentions: [me],
          quote: "edit migrations/0007_todo_completed_at.sql · +11",
          ts: Date.now() - 9 * 60_000,
        },
      ],
    ],
  ]);
  const members = () => [
    { id: me, name, online: true },
    { id: "marcus", name: "Marcus Hale", online: marcusOnline },
    { id: "john", name: "John Okafor", online: false },
  ];
  const text = (frame: unknown) => s.onmessage?.({ data: JSON.stringify(frame) });
  const bin = (bytes: Uint8Array) => s.onmessage?.({ data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) });
  const live = (o: unknown) => {
    if (!attached) return;
    bin(encodeLive(attached, [{ seq: ++seq, bytes: enc.encode(line(o) + "\n") }]));
  };
  const s: team.SocketLike & { presence: () => void } = {
    binaryType: "blob",
    onopen: null,
    onmessage: null,
    onclose: null,
    onerror: null,
    presence: () => {
      text({ t: "presence", members: members() });
      text({ t: "share", share: marcusShare() });
    },
    send(data) {
      if (typeof data !== "string" || data === "ping") return;
      const frame = JSON.parse(data);
      switch (frame.t) {
        case "me":
          name = frame.name;
          s.presence();
          break;
        // Abrir uma aba do Marcus: a conversa vem, e depois uma linha de vez
        // em quando — o suficiente para ver a tela andar sozinha.
        case "attach":
          attached = frame.tab;
          clearInterval(ticking);
          setTimeout(() => bin(encodeSnapshot(frame.tab, me, seq, enc.encode(SAMPLE))), 200);
          ticking = setInterval(
            () => live({ type: "assistant", message: { id: `mk${Date.now()}`, role: "assistant", content: [{ type: "text", text: `${new Date().toLocaleTimeString()} — ✓ 1 test passed` }] } }),
            2500,
          );
          break;
        case "detach":
          attached = null;
          clearInterval(ticking);
          break;
        // O que você escreve volta como o back dele ecoaria a fala.
        case "write":
          if (!String(frame.data).startsWith("{")) live({ type: "user", message: { role: "user", content: frame.data }, ts: Date.now() });
          break;
        // Quem compartilha o seu ganha o Marcus olhando, meio segundo depois.
        case "share":
          setTimeout(() => text({ t: "watch", ws: frame.share.id, tab: frame.share.active ?? frame.share.tabs[0]?.id, members: ["marcus"], added: ["marcus"] }), 500);
          break;
        case "unshare":
          break;
        case "notes":
          text({ t: "notes", ws: frame.ws, items: notes.get(frame.ws) ?? [] });
          break;
        // Nota nova: o relay dá o id e devolve a todos — inclusive a quem
        // escreveu, que é como ela ganha o id.
        case "note": {
          const note = {
            id: `${Date.now()}-m`,
            ws: frame.ws,
            author: me,
            text: frame.text,
            mentions: frame.mentions,
            quote: frame.quote,
            ts: Date.now(),
          };
          notes.set(frame.ws, [...(notes.get(frame.ws) ?? []), note]);
          text({ t: "note", note });
          // E o Marcus responde, se foi ele quem você marcou.
          if (frame.mentions.includes("marcus")) {
            setTimeout(() => {
              const reply = {
                id: `${Date.now()}-r`,
                ws: frame.ws,
                author: "marcus",
                text: "Vi. Coluna, então — uma migração contra um join em toda leitura não se paga.",
                mentions: [me],
                quote: null,
                ts: Date.now(),
              };
              notes.set(frame.ws, [...(notes.get(frame.ws) ?? []), reply]);
              text({ t: "note", note: reply });
              text({ t: "inbox", items: [{ id: reply.id, ws: frame.ws, author: "marcus", ts: reply.ts }] });
            }, 1200);
          }
          break;
        }
        case "inbox_read":
          text({ t: "inbox", items: [] });
          break;
      }
    },
    close() {
      fakes.splice(fakes.indexOf(s), 1);
      clearInterval(ticking);
      setTimeout(() => s.onclose?.());
    },
  };
  fakes.push(s);
  setTimeout(() => {
    s.onopen?.();
    text({
      t: "welcome",
      you: me,
      members: members(),
      shares: [marcusShare()],
      inbox: [{ id: "1-a", ws: "ws-marcus", author: "marcus", ts: Date.now() - 9 * 60_000 }],
      watching: {},
    });
  }, 500);
  return s;
}
// Com `VITE_RELAY` no ambiente o time é de verdade — o relay local do
// `wrangler dev` —, e só o back continua de mentira. É como dois navegadores
// testam o compartilhamento de ponta a ponta sem subir o Tauri.
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

// Atalho para testar o arrastar-e-soltar pelo console: `mock.drop([...])`.
w.mock = {
  /// Uma linha na conversa de mentira, como se o processo tivesse escrito.
  line: (tab: string, o: unknown) => pushLine(tab, o),
  /// Quantas gravações de MCP/plugin o back de mentira recebeu.
  writes: () => writes,
  /// O que o time diz agora — para dirigir a tela de fora e ver o que ela viu.
  team: () => ({ status: team.status(), remotes: team.remotes() }),
  presence: (online: boolean) => {
    marcusOnline = online;
    for (const s of fakes) (s as unknown as { presence: () => void }).presence();
  },
  /// Simula soltar arquivos num ponto da tela — o mesmo evento que o Tauri
  /// manda quando você arrasta de fora para dentro da janela.
  drop: (paths: string[], x = innerWidth / 2, y = innerHeight / 2, dropX = x, dropY = y) => {
    emit("tauri://drag-over", { position: { x: x * devicePixelRatio, y: y * devicePixelRatio } });
    emit("tauri://drag-drop", { paths, position: { x: dropX * devicePixelRatio, y: dropY * devicePixelRatio } });
  },
  over: (position: { x: number; y: number }) => emit("tauri://drag-over", { position }),
};
