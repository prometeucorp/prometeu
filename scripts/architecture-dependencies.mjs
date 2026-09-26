import path from "node:path/posix";
import ts from "typescript";

const componentRoot = "packages/design-system/src/";
const desktopComponentRoot = "src/components/";
const viewDependencies = new Set([
  "src/resources/model.ts", "src/settings-navigation.ts", "src/ui.ts", "src/menu.ts", "src/icons.ts", "src/util.ts",
  "src/feedback-i18n.ts", "src/i18n.ts", "src/i18n.en.ts", "src/i18n.pt.ts", "src/platform.ts", "src/markdown.ts",
  "src/highlight.ts", "src/timeline.ts", "src/conversation.ts", "src/conversation-legacy.ts", "src/context.ts",
  "src/browser-context.ts", "src/browser-types.ts", "src/mentions.ts", "src/types.ts",
]);
const isolatedViews = new Set(["src/components/resource-view.ts", "src/components/compositions.ts"]);
const compositionDependencies = new Set(["src/components/compositions.css", "src/components/primitives.ts", "src/components/menu.ts"]);
const resourceDependencies = new Set([
  "src/resources/model.ts", "src/settings-navigation.ts", "src/util.ts", "src/ui.ts", "src/menu.ts",
  "src/components/resource-view.css", "src/components/compositions.ts", "src/components/compositions.css",
  "src/components/primitives.ts", "src/components/menu.ts",
]);
const componentEffects = new Set(["fetch", "XMLHttpRequest", "WebSocket", "localStorage", "sessionStorage", "indexedDB"]);
const pureRoots = new Set(["src/timeline.ts", "relay/src/logic.ts", "relay/src/protocol.ts"]);
const desktop = /^src\/(?:ipc|mock|team)\.ts$/;
const mobileDesktop = /^src\/(?:ipc|mock|team|chat|session|main)\.ts$/;
const ambientEffects = new Set([
  "document", "window", "navigator", "globalThis", "self", "localStorage", "sessionStorage",
  "indexedDB", "caches", "fetch", "XMLHttpRequest", "WebSocket", "WebSocketPair", "EventSource",
  "Worker", "DurableObject", "DurableObjectState", "ExecutionContext", "process", "console",
  "setTimeout", "setInterval", "requestAnimationFrame",
]);

function imports(source) {
  const dependencies = [];
  const effects = [];
  const add = (node, literal, typeOnly = false) => dependencies.push({
    specifier: literal && ts.isStringLiteralLike(literal) ? literal.text : null,
    typeOnly,
    line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
  });
  const visit = (node) => {
    if (ts.isImportDeclaration(node)) {
      const clause = node.importClause;
      const bindings = clause?.namedBindings;
      const onlyTypes = clause?.isTypeOnly || (!clause?.name && bindings && ts.isNamedImports(bindings)
        && bindings.elements.length > 0 && bindings.elements.every((item) => item.isTypeOnly));
      add(node, node.moduleSpecifier, Boolean(onlyTypes));
    } else if (ts.isExportDeclaration(node) && node.moduleSpecifier) {
      const onlyTypes = node.isTypeOnly || (node.exportClause && ts.isNamedExports(node.exportClause)
        && node.exportClause.elements.length > 0 && node.exportClause.elements.every((item) => item.isTypeOnly));
      add(node, node.moduleSpecifier, Boolean(onlyTypes));
    } else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) {
      add(node, node.argument.literal, true);
    } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) {
      add(node, node.moduleReference.expression, node.isTypeOnly);
    } else if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword
      || (ts.isIdentifier(node.expression) && node.expression.text === "require"))) {
      add(node, node.arguments[0]);
    }
    if (ts.isIdentifier(node) && ambientEffects.has(node.text)) {
      // Property names are not ambient references; globalThis itself remains forbidden.
      const parent = node.parent;
      const propertyName = (ts.isPropertyAccessExpression(parent) || ts.isPropertyAssignment(parent)
        || ts.isPropertySignature(parent) || ts.isMethodDeclaration(parent) || ts.isMethodSignature(parent))
        && parent.name === node;
      if (!propertyName) effects.push(`${node.text} at line ${source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1}`);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return { dependencies, effects };
}

function resolve(file, specifier, sources) {
  if (!specifier.startsWith(".")) return { target: specifier, local: false, runtime: false };
  const [name] = specifier.split(/[?#]/);
  const target = path.normalize(path.join(path.dirname(file), name));
  // Vite raw/url imports are assets, not execution of the referenced source.
  const query = new URLSearchParams(specifier.match(/^[^?#]*\?([^#]*)/)?.[1]);
  if (query.has("raw") || query.has("url")) return { target, local: true, runtime: false, asset: true };
  const extension = path.extname(target);
  const replacements = { ".js": [".ts", ".tsx", ".js"], ".jsx": [".tsx", ".jsx"],
    ".mjs": [".mts", ".mjs"], ".cjs": [".cts", ".cjs"] };
  const extensions = [".ts", ".tsx", ".js", ".jsx"];
  const candidates = replacements[extension]?.map((ext) => target.slice(0, -extension.length) + ext)
    ?? (extension && !sources.has(target + ".ts") ? [target] : [...extensions.map((ext) => target + ext),
      ...extensions.map((ext) => `${target}/index${ext}`)]);
  const found = candidates.find((candidate) => sources.has(candidate));
  return { target: found ?? target, local: true, runtime: Boolean(found) };
}

/** Check production source imports. Type-only edges never create runtime cycles or transitive effects. */
export function checkDependencies(sources) {
  const failures = new Set();
  const modules = new Map();
  for (const [file, text] of sources) {
    const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
    const parsed = imports(source);
    for (const dependency of parsed.dependencies) {
      if (dependency.specifier === null) {
        failures.add(`${file}:${dependency.line}: use a literal import so dependency boundaries can be checked`);
        continue;
      }
      Object.assign(dependency, resolve(file, dependency.specifier, sources));
      if (dependency.local && !dependency.runtime && !dependency.asset
        && (!path.extname(dependency.target) || /\.[cm]?[jt]sx?$/.test(dependency.target))) {
        failures.add(`${file}:${dependency.line}: unresolved local source: ${dependency.specifier}`);
      }
    }
    modules.set(file, parsed);
  }

  const visited = new Set();
  const active = [];
  const visit = (file) => {
    const cycle = active.indexOf(file);
    if (cycle !== -1) {
      failures.add(`runtime import cycle: ${[...active.slice(cycle), file].join(" -> ")}`);
      return;
    }
    if (visited.has(file)) return;
    visited.add(file);
    active.push(file);
    for (const dependency of modules.get(file).dependencies) {
      if (dependency.runtime && !dependency.typeOnly) visit(dependency.target);
    }
    active.pop();
  };
  for (const file of modules.keys()) visit(file);

  for (const root of modules.keys()) {
    const components = root.startsWith(componentRoot);
    const core = /^src\/team-[^/]+\.ts$/.test(root);
    const mobile = root.startsWith("src/mobile/");
    const pure = pureRoots.has(root);
    const desktopComponent = root.startsWith(desktopComponentRoot) && !/\/(?:stories|gallery)\.ts$/.test(root);
    const isolatedView = isolatedViews.has(root);
    const allowedViewDependencies = root === "src/components/compositions.ts" ? compositionDependencies : resourceDependencies;
    if (!components && !core && !mobile && !pure && !isolatedView && !desktopComponent) continue;
    const seen = new Set();
    const walk = (file, chain) => {
      if (seen.has(file)) return;
      seen.add(file);
      const module = modules.get(file);
      if (desktopComponent && file.startsWith(desktopComponentRoot)) {
        for (const effect of module.effects) {
          if (componentEffects.has(effect.split(" ")[0])) failures.add(`${chain.join(" -> ")}: component effect ${effect}`);
        }
      }
      if (pure) {
        for (const effect of module.effects) failures.add(`${chain.join(" -> ")}: ambient effect ${effect}`);
      }
      for (const dependency of module.dependencies) {
        if (dependency.specifier === null) continue;
        const { target, local, runtime, typeOnly, line } = dependency;
        const tauri = target.startsWith("@tauri-apps/");
        const permittedComponent = target.startsWith(desktopComponentRoot) && !/\/(?:stories|gallery)\.ts$/.test(target);
        const forbidden = (isolatedView && (!local || (!allowedViewDependencies.has(target) && !target.startsWith(componentRoot))))
          || (desktopComponent && !(local && (permittedComponent || target.startsWith(componentRoot) || viewDependencies.has(target))) && target !== "marked")
          || (components && (!local || !target.startsWith(componentRoot)))
          || (core && (tauri || desktop.test(target)))
          || (mobile && (tauri || mobileDesktop.test(target)))
          || (pure && !typeOnly && !runtime);
        if (forbidden) {
          failures.add(`${chain.join(" -> ")}:${line}: forbidden dependency ${dependency.specifier}`);
        } else if (runtime && !typeOnly) {
          walk(target, [...chain, target]);
        }
      }
    };
    walk(root, [root]);
  }
  return [...failures];
}
