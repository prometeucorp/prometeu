import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const checker = fileURLToPath(new URL("./check-docs.mjs", import.meta.url));
const page = "docs/contracts/page.md";

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "prometeu-docs-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (name, text) => {
    const path = join(root, name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, text);
  };
  const edit = (name, before, after) => write(name, readFileSync(join(root, name), "utf8").replace(before, after));
  write("AGENTS.md", "# Standing instructions\n");
  write("CLAUDE.md", "@AGENTS.md\n");
  write("README.md", "# Product\n[Documentation](docs/README.md)\n");
  write("src/components/README.md", "# Component guide without metadata\n");
  const areas = { architecture: "Architecture", contracts: "Contract", decisions: "Decision", operations: "Operations", quality: "Quality" };
  write("docs/README.md", Object.keys(areas).map((area) => `- [${area}](${area}/README.md)`).join("\n"));
  for (const [area, type] of Object.entries(areas)) {
    write(`docs/${area}/README.md`, "# Area\n\n- [Sample](page.md) — Example concept.\n");
    write(`docs/${area}/page.md`, `---\ntype: ${type}\ntitle: "Sample"\ndescription: "Example concept."\nstatus: stable\n---\n\n# Sample\n`);
  }
  return { root, write, edit };
}

test("documentation bundle validation", async (t) => {
  const cases = [
    ["accepts area indexes without a flat root catalog", () => {}, null],
    ["accepts plain and single-quoted YAML strings", ({ edit }) => {
      edit(page, 'title: "Sample"', "title: Workspace Git");
      edit(page, 'description: "Example concept."', "description: 'The owner''s Git operations.'");
    }, null],
    ["accepts quoted punctuation and escaped quotes", ({ edit }) => {
      edit(page, 'title: "Sample"', `title: ${JSON.stringify('Git: the "owner" #1')}`);
    }, null],
    ["accepts draft and deprecated lifecycle values", ({ edit }) => {
      edit("docs/decisions/page.md", "status: stable", "status: draft");
      edit(page, "status: stable", "status: deprecated");
    }, null],
    ["resolves indexed links with fragments and encoded paths", ({ write, edit, root }) => {
      write("docs/contracts/workspace git.md", readFileSync(join(root, page), "utf8"));
      rmSync(join(root, page));
      edit("docs/contracts/README.md", "(page.md)", "(./workspace%20git.md#sample)");
    }, null],
    ["rejects a concept linked only from the root", ({ write }) => {
      write("docs/contracts/README.md", "# Contracts\n");
      write("docs/README.md", "[Contracts](contracts/README.md)\n[Sample](contracts/page.md)\n");
    }, /docs\/contracts\/README.md: does not index docs\/contracts\/page.md/],
    ["rejects a disconnected area index", ({ edit }) => {
      edit("docs/README.md", "- [contracts](contracts/README.md)", "");
    }, /docs\/README.md: does not index docs\/contracts\/README.md/],
    ["rejects a missing folder index", ({ root }) => {
      rmSync(join(root, "docs/contracts/README.md"));
    }, /target does not exist: contracts\/README.md/],
    ["does not count image links as index entries", ({ edit }) => {
      edit("docs/contracts/README.md", "[Sample]", "![Sample]");
    }, /does not index docs\/contracts\/page.md/],
    ["rejects missing frontmatter", ({ write }) => write(page, "# Sample\n"), /missing YAML frontmatter/],
    ["rejects a missing field", ({ edit }) => {
      edit(page, 'description: "Example concept."\n', "");
    }, /missing metadata field: description/],
    ["rejects an empty title", ({ edit }) => {
      edit(page, 'title: "Sample"', 'title: " "');
    }, /title must be a nonempty single-line YAML string/],
    ["rejects a YAML null value", ({ edit }) => {
      edit(page, 'description: "Example concept."', "description: null");
    }, /description must be a nonempty single-line YAML string/],
    ["rejects malformed quoted YAML", ({ edit }) => {
      edit(page, 'title: "Sample"', 'title: "Sample');
    }, /title must be a nonempty single-line YAML string/],
    ["rejects unquoted YAML punctuation", ({ edit }) => {
      edit(page, 'title: "Sample"', "title: Git: operations");
    }, /title must be a nonempty single-line YAML string/],
    ["rejects duplicate fields", ({ edit }) => {
      edit(page, "status: stable", "status: stable\nstatus: draft");
    }, /duplicate metadata field/],
    ["rejects extra metadata fields", ({ edit }) => {
      edit(page, "status: stable", "status: stable\ngenerated: today");
    }, /unknown, malformed or duplicate metadata field/],
    ["rejects a type from the wrong area", ({ edit }) => {
      edit(page, "type: Contract", "type: Quality");
    }, /type must match its bundle area/],
    ["rejects old lifecycle names", ({ edit }) => {
      edit(page, "status: stable", "status: Accepted");
    }, /status must be draft, stable or deprecated/],
    ["rejects a competing body status", ({ edit }) => {
      edit(page, "# Sample", "# Sample\n\nStatus: Accepted");
    }, /use only YAML status/],
    ["keeps broken local link checks", ({ write }) => {
      write("README.md", "[Missing](missing.md)\n");
    }, /target does not exist: missing.md/],
    ["keeps invalid link escape checks", ({ write }) => {
      write("README.md", "[Invalid](%ZZ.md)\n");
    }, /invalid link escape/],
    ["keeps the standing-instructions line limit", ({ write }) => {
      write("AGENTS.md", "Instruction\n".repeat(201));
    }, /short-map limit is 200/],
    ["keeps the Claude import check", ({ write }) => {
      write("CLAUDE.md", "# Instructions\n");
    }, /does not import @AGENTS.md/],
  ];
  for (const [name, change, expected] of cases) {
    await t.test(name, (t) => {
      const files = fixture(t);
      change(files);
      const result = spawnSync(process.execPath, [checker], { cwd: files.root, encoding: "utf8" });
      const output = (result.stdout + result.stderr).replaceAll("\\", "/");
      assert.equal(result.status, expected ? 1 : 0, output);
      if (expected) assert.match(output, expected);
    });
  }
});
