import assert from "node:assert/strict";
import test from "node:test";
import { checkDependencies } from "./architecture-dependencies.mjs";

const check = (sources) => checkDependencies(new Map(Object.entries(sources)));

test("detects runtime cycles through re-exports and literal dynamic imports", () => {
  const failures = check({
    "src/start.ts": 'export * from "./nested/barrel.js";',
    "src/nested/barrel.ts": 'export const load = () => import("../start");',
  });
  assert.match(failures.join("\n"), /runtime import cycle: src\/start.ts -> src\/nested\/barrel.ts -> src\/start.ts/);
});

test("ignores type-only cycles, comments and strings, but keeps mixed imports", () => {
  assert.deepEqual(check({
    "src/a.ts": 'import type { B } from "./b"; export type { B } from "./b";',
    "src/b.ts": 'import { type A } from "./a"; export { type A } from "./a"; type C = import("./a").A;',
    "src/c.ts": '// import "./c";\nconst text = \'import("./c")\';',
  }), []);
  assert.match(check({
    "src/a.ts": 'import { type B, value } from "./b";',
    "src/b.ts": 'import "./a";',
  }).join("\n"), /runtime import cycle/);
});

test("rejects indirect desktop imports from core and nested mobile sources", () => {
  const failures = check({
    "src/team-member.ts": 'import { run } from "./shared";',
    "src/mobile/nested/view.ts": 'export * from "../../shared";',
    "src/shared.ts": 'export * from "./ipc";',
    "src/ipc.ts": 'import { invoke } from "@tauri-apps/api/core";',
  });
  assert.match(failures.join("\n"), /src\/team-member.ts -> src\/shared.ts:1: forbidden dependency .\/ipc/);
  assert.match(failures.join("\n"), /src\/mobile\/nested\/view.ts -> src\/shared.ts:1: forbidden dependency .\/ipc/);
});

test("portable type imports do not execute dependencies, but shell types remain forbidden", () => {
  assert.deepEqual(check({
    "src/team-member.ts": 'import type { Board } from "./types";',
    "src/types.ts": 'import "./ipc";',
    "src/ipc.ts": 'import "@tauri-apps/api/core";',
  }), []);
  assert.match(check({
    "src/team-member.ts": 'import type { Shell } from "./team";',
    "src/team.ts": '',
  }).join("\n"), /forbidden dependency .\/team/);
});

test("design system allows internal .js paths and rejects external package dependencies", () => {
  const sources = {
    "packages/design-system/src/index.ts": 'export * from "./nested/button.js";',
    "packages/design-system/src/nested/button.ts": 'import "../dom.js";',
    "packages/design-system/src/dom.ts": 'export const h = () => document.createElement("div");',
  };
  assert.deepEqual(check(sources), []);
  sources["packages/design-system/src/nested/button.ts"] = 'export * from "../../../../src/ipc";';
  sources["src/ipc.ts"] = '';
  assert.match(check(sources).join("\n"), /forbidden dependency .*src\/ipc/);
});

test("pure roots reject ambient effects and external imports through helpers", () => {
  assert.deepEqual(check({
    "src/timeline.ts": 'export * from "./helper";',
    "src/helper.ts": 'const label = "fetch"; export const item = { window: "label" }; // document\n',
  }), []);
  for (const effect of ['fetch("https://example.test")', 'globalThis["fetch"]("/")', 'document.createElement("p")']) {
    assert.match(check({
      "src/timeline.ts": 'export * from "./helper";',
      "src/helper.ts": effect,
    }).join("\n"), /src\/timeline.ts -> src\/helper.ts: ambient effect/);
  }
  assert.match(check({
    "relay/src/logic.ts": 'import { readFile } from "node:fs";',
  }).join("\n"), /forbidden dependency node:fs/);
});

test("reports imports that cannot be checked instead of silently dropping them", () => {
  assert.match(check({ "src/start.ts": 'import("./" + name);' }).join("\n"), /use a literal import/);
  assert.match(check({ "src/start.ts": 'import "./missing";' }).join("\n"), /unresolved local source/);
  assert.deepEqual(check({ "src/start.ts": 'import "./style.css"; import text from "./script.js?raw";' }), []);
});

test("custom queries and hashes preserve runtime boundaries; raw and url imports are assets", () => {
  for (const suffix of ["?custom", "#custom"]) {
    assert.match(check({
      "src/team-member.ts": `import "./helper${suffix}";`,
      "src/helper.ts": 'export * from "./ipc";',
      "src/ipc.ts": '',
    }).join("\n"), /src\/team-member.ts -> src\/helper.ts:1: forbidden dependency .\/ipc/);
  }
  for (const suffix of ["?raw", "?url"]) {
    assert.deepEqual(check({
      "src/team-member.ts": `import text from "./helper.ts${suffix}";`,
      "src/helper.ts": 'import "./ipc";',
      "src/ipc.ts": '',
    }), []);
  }
});


test("resource presentation allows primitives and rejects indirect integration imports", () => {
  const sources = {
    "src/components/resource-view.ts": 'import "../ui"; import "../resources/model"; import "./resource-view.css";',
    "src/resources/model.ts": 'import type { Item } from "../menu";',
    "src/ui.ts": 'export * from "../packages/design-system/src/ui";',
    "src/menu.ts": 'export * from "../packages/design-system/src/menu";',
    "packages/design-system/src/ui.ts": 'export const create = () => document.createElement("button");',
    "packages/design-system/src/menu.ts": '',
  };
  assert.deepEqual(check(sources), []);
  sources["src/ui.ts"] = 'export * from "./mcp";';
  sources["src/mcp.ts"] = 'import "./ipc";';
  sources["src/ipc.ts"] = '';
  assert.match(check(sources).join("\n"), /src\/components\/resource-view.ts -> src\/ui.ts:1: forbidden dependency .\/mcp/);
});


test("desktop compositions reject integration dependencies even without a screen consumer", () => {
  assert.deepEqual(check({
    "src/components/compositions.ts": 'import "./primitives"; import "./compositions.css";',
    "src/components/primitives.ts": 'export * from "../../packages/design-system/src/ui";',
    "packages/design-system/src/ui.ts": 'export const create = () => document.createElement("button");',
  }), []);
  assert.match(check({
    "src/components/compositions.ts": 'import "../helper";',
    "src/helper.ts": 'import "./ipc";',
    "src/ipc.ts": '',
  }).join("\n"), /src\/components\/compositions.ts:1: forbidden dependency ..\/helper/);
});


test("desktop compositions cannot couple reusable parts to resource models", () => {
  assert.match(check({
    "src/components/compositions.ts": 'import type { ResourceItem } from "../resources/model";',
    "src/resources/model.ts": '',
  }).join("\n"), /forbidden dependency ..\/resources\/model/);
});

test("Desktop components reject transport and direct storage effects", () => {
  for (const source of ['import "../../ipc";', 'localStorage.setItem("x", "y");', 'fetch("/data");']) {
    assert.notDeepEqual(check({ "src/components/chat/composer.ts": source, "src/ipc.ts": '' }), []);
  }
});

test("dotted source basenames remain part of transitive dependency checks", () => {
  assert.match(check({
    "src/team-member.ts": 'import "./labels.en";',
    "src/labels.en.ts": 'import "./ipc";',
    "src/ipc.ts": '',
  }).join("\n"), /src\/team-member.ts -> src\/labels.en.ts:1: forbidden dependency .\/ipc/);
});
