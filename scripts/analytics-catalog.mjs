import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import ts from "typescript";

const output = "docs/contracts/product-analytics-catalog.json";
const controls = new Map();
const derived = [];
const primitiveArgs = { identifyControl: 1, button: 3, input: 2, checkbox: 2, toggle: 2, radio: 4, field: 3, menuButton: 2, password: 2, passwordToggle: 2 };
const files = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
  const path = `${directory}/${entry.name}`;
  return entry.isDirectory() ? files(path) : [path];
});
const paths = ["index.html", ...files("src"), ...files("packages/design-system/src")].filter(path =>
  /\.(?:ts|js|html)$/.test(path) && !/\.test\.|\.d\.ts$|\/stories\.|\/gallery\.|-gallery\.|\/mock|\/mobile\//.test(path));
function record(target, path) {
  if (!/^[A-Za-z0-9._-]{1,80}$/.test(target)) return;
  if (!controls.has(target)) controls.set(target, new Set());
  controls.get(target).add(path);
}
function unwrapped(node) {
  while (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isSatisfiesExpression(node)) node = node.expression;
  return node;
}
function literals(node) {
  node = unwrapped(node);
  if (ts.isStringLiteralLike(node)) return { values: [node.text], complete: true };
  if (ts.isConditionalExpression(node)) {
    const yes = literals(node.whenTrue), no = literals(node.whenFalse);
    return { values: [...yes.values, ...no.values], complete: yes.complete && no.complete };
  }
  return { values: [], complete: false };
}
for (const path of paths) {
  const text = readFileSync(path, "utf8");
  // Only an explicit tracking attribute, or an explicit i18n key on an actionable element.
  for (const tag of text.matchAll(/<[a-z][^<>]*>/gi)) {
    const authored = tag[0].match(/\bdata-track(?:-double-click|-context-menu)?=["']([A-Za-z0-9._-]+)["']/g) ?? [];
    for (const attribute of authored) record(attribute.match(/=["']([^"']+)/)[1], path);
    if (/^<(?:button|a|input|select|textarea|summary)\b|\brole=["'](?:button|tab|option|menuitem|checkbox|radio|switch)["']/.test(tag[0])) {
      const key = tag[0].match(/\bdata-t(?:-title)?=["']([A-Za-z0-9._-]+)["']/);
      if (key) record(key[1], path);
    }
  }
  if (path.endsWith(".html")) continue;
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true);
  const aliases = new Map();
  for (const statement of source.statements) {
    if (!ts.isImportDeclaration(statement) || !/(?:ui|primitives)$/.test(statement.moduleSpecifier.text)) continue;
    const bindings = statement.importClause?.namedBindings;
    if (bindings && ts.isNamedImports(bindings)) for (const imported of bindings.elements) {
      if (imported.propertyName) aliases.set(imported.name.text, imported.propertyName.text);
    }
  }
  // Local helpers can shadow a primitive with a different argument order, or be unrelated to UI.
  const localArgs = new Map();
  function declarations(node) {
    const fn = ts.isFunctionDeclaration(node) ? node
      : ts.isVariableDeclaration(node) && node.initializer && (ts.isArrowFunction(node.initializer) || ts.isFunctionExpression(node.initializer)) ? node.initializer : undefined;
    const name = fn && node.name && ts.isIdentifier(node.name) ? node.name.text : undefined;
    if (name && (Object.hasOwn(primitiveArgs, name) || name === "iconButton")) {
      const index = fn.parameters.findIndex(parameter => ts.isIdentifier(parameter.name) && parameter.name.text === "track");
      localArgs.set(name, index < 0 ? undefined : index);
    }
    ts.forEachChild(node, declarations);
  }
  declarations(source);
  const note = node => {
    const { values, complete } = literals(node);
    values.forEach(value => record(value, path));
    if (!complete) derived.push({ source: path, expression: node.getText(source) });
  };
  const noteEmptyAction = node => {
    node = unwrapped(node);
    if (ts.isArrayLiteralExpression(node)) {
      if (node.elements[2]) note(node.elements[2]);
    } else if (ts.isConditionalExpression(node)) {
      noteEmptyAction(node.whenTrue); noteEmptyAction(node.whenFalse);
    } else if (node.kind !== ts.SyntaxKind.NullKeyword && node.getText(source) !== "undefined") {
      derived.push({ source: path, expression: `(${node.getText(source)})[2]` });
    }
  };
  // These specific helpers explicitly forward their authored Key to UI tracking; visible labels
  // in other objects are never identities. Keep unresolved forwarded expressions in the inventory.
  const noteFieldOptions = node => {
    node = unwrapped(node);
    if (ts.isObjectLiteralExpression(node)) {
      const label = node.properties.find(property => ts.isPropertyAssignment(property) && property.name.getText(source) === "label");
      if (label) note(label.initializer);
    } else if (ts.isConditionalExpression(node)) {
      noteFieldOptions(node.whenTrue); noteFieldOptions(node.whenFalse);
    } else derived.push({ source: path, expression: `(${node.getText(source)}).label` });
  };
  function visit(node) {
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && /\.dataset\.(?:track|trackDoubleClick|trackContextMenu)$/.test(node.left.getText(source))) note(node.right);
    if (ts.isPropertyAssignment(node) && node.name.getText(source) === "track") note(node.initializer);
    if (ts.isCallExpression(node)) {
      const bare = ts.isIdentifier(node.expression);
      const localName = node.expression.getText(source).split(".").at(-1);
      const name = bare ? aliases.get(localName) ?? localName : localName;
      let index = bare && localArgs.has(localName) ? localArgs.get(localName) : primitiveArgs[name];
      if (bare && path === "src/action-settings.ts" && ["button", "field", "checkbox"].includes(name)) index = 0;
      if (bare && node.arguments[0] && ((path === "src/mcp.ts" && name === "field") || (path === "src/plugins.ts" && ["field", "area"].includes(name)))) noteFieldOptions(node.arguments[0]);
      if (index !== undefined && node.arguments[index] && node.arguments[index].kind !== ts.SyntaxKind.UndefinedKeyword && node.arguments[index].getText(source) !== "undefined") note(node.arguments[index]);
      if (name === "empty" && node.arguments[2]) noteEmptyAction(node.arguments[2]);
      if (name === "setAttribute" && node.arguments[0] && ts.isStringLiteralLike(node.arguments[0]) && /^data-track(?:-double-click|-context-menu)?$/.test(node.arguments[0].text)) note(node.arguments[1]);
      if (path === "src/appmenu.ts" && name === "our" && node.arguments[1]) note(node.arguments[1]);
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
}
const featureSource = ts.createSourceFile("analytics-events.ts", readFileSync("src/analytics-events.ts", "utf8"), ts.ScriptTarget.Latest, true);
const featureType = featureSource.statements.find(node => ts.isTypeAliasDeclaration(node) && node.name.text === "FeatureEvent");
const events = featureType.type.types.map(node => Object.fromEntries(node.members.map(member => [member.name.getText(featureSource), member.type.getText(featureSource)]))).map(({ name, ...properties }) => ({ name: JSON.parse(name), properties })).sort((a, b) => a.name.localeCompare(b.name));
const catalog = {
  description: "Source-derived inventory of explicit control identities and feature event schemas. Composed identities require runtime coverage; this is not a count of rendered controls. Regenerate with node scripts/analytics-catalog.mjs.",
  controls: [...controls].sort(([a], [b]) => a.localeCompare(b)).map(([target, sources]) => ({ target, sources: [...sources].sort() })),
  composed: [...new Map(derived.map(item => [JSON.stringify(item), item])).values()].sort((a, b) => a.source.localeCompare(b.source) || a.expression.localeCompare(b.expression)),
  feature_events: events,
};
const generated = `${JSON.stringify(catalog, null, 2)}\n`;
if (process.argv.includes("--check")) {
  if (readFileSync(output, "utf8") !== generated) throw new Error("Analytics catalog is stale. Run node scripts/analytics-catalog.mjs.");
} else writeFileSync(output, generated);
console.log(`${catalog.controls.length} authored control identities; ${events.length} feature event schemas; ${catalog.composed.length} composed metadata expressions.`);
