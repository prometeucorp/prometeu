# Compose a Desktop screen

Start at [`src/components/README.md`](../../src/components/README.md) and
[`catalog.json`](../../src/components/catalog.json). This directory contains the
Desktop component APIs, styles and executable stories. The application uses
the same exports; the catalog records the production consumers.

Run `npm run dev` and open `/design-system.html`. Search by name or source path.
Each component has a URL with a `component` id and `state`; add `embed=1` for a
standalone canvas. A browser agent can inspect the real rendering while a coding
agent follows the manifest to its implementation and callers.

Chat, Git, icons, Markdown, Resources and Actions now consume these components.
Business effects remain in their controllers. The catalog is a navigation and
example layer, not a schema that renders the product.

## Reusable Desktop components

Import from `src/components/compositions.ts`. These components depend only on shared
controls, DOM helpers, icons and their stylesheet. They do not import i18n,
resource models, Actions, IPC or provider catalogs. Every label comes from the
caller. The API is typed; layout slots accept ordinary elements built from the
existing primitives.

| Component | Input and responsibility | Production use |
| --- | --- | --- |
| `sectionHeader` | Optional title, count, description, supporting controls and actions. Wraps at available width. | Resources introduction; Actions command and profile sections |
| `toolbar` | Leading and trailing control arrays. Arranges search, filters and actions without owning their rules. | Resources search/filter bar; both screens through `sectionHeader` |
| `itemRow` | Title, description, typed icon, optional status/busy state, metadata and action slots. Returns named DOM parts for local additions. | Resource rows; command and profile cards |
| `overflowAction` | Accessible label, stable focus key, menu items and busy flag. Reuses shared keyboard/menu lifecycle; disables unavailable actions. | Resource menus; Actions card and template menus |
| `listState` | Discriminated ready, empty, loading or error state; optional labeled retry callback. | Resource feedback; empty command/profile lists |

Open `/design-system.html#components-preview` for an independent composition of
these parts. It imports neither Resources nor Actions. Search, pending states,
error recovery and local callbacks run without application services. The same
parts run in two real Desktop pages, not only in a gallery.

```ts
import { button } from "./ui";
import { sectionHeader, itemRow } from "./components/compositions";

const header = sectionHeader({ title: labels.title, actions: [
  button(labels.add, onAdd),
] });
const row = itemRow({
  title: item.name, description: item.description, glyph: "terminal",
  actions: [button(labels.edit, () => onEdit(item.id), "ghost")],
});
host.append(header, row.root);
```

The caller supplies `labels`, `item`, `onAdd`, `onEdit` and `host`. No loading or
saving happens during component construction. Row metadata remains local to the
screen; avoid adding domain-specific flags to the shared component. Each call
creates fresh DOM. These building blocks have no subscriptions or update loop.
The screen owns replacement and focus restoration; close its open menu before
removing its controls. Resource view already handles that lifecycle.

## Start from the executable example

Run `npm run dev` and open `/design-system.html#resources-preview`. The preview
imports the production presentation. Select populated, empty, loading, error,
pending, unavailable or long-text states. Search for an absent name to exercise
no results. Edit an example item to update its snapshot while preserving focus.
All example actions stay local.

Read these files instead of copying markup from a hub:

- `src/resources/gallery.ts`: runnable composition with sample data.
- `src/components/resource-view.ts`: presentation inputs and lifecycle.
- `src/resources/model.ts`: typed items and search matching.
- `src/resources/labels.ts`: the desktop i18n adapter.
- `src/components/resource-view.css`: local layout using the shared tokens.
- `packages/design-system/README.md`: existing control APIs and accessibility.

## Minimal composition

```ts
import { resourceView } from "./components/resource-view";
import { resourceLabels } from "./resources/labels";

const screen = resourceView({
  labels: resourceLabels(),
  snapshot: {
    items: [{
      key: "example-review", id: "review", kind: "skills",
      description: "Review the current changes", origins: [{ label: "Example" }],
      glyph: "sparkles", actions: [],
    }],
  },
  defaults: () => {},
  add: () => [],
});
host.append(screen.root);
// When the host removes the screen:
// screen.destroy();
```

The import paths above assume a caller in `src/`. Example resource data may be
literal; interface labels must use i18n. Supply real callbacks in the desktop
adapter, never inside the presentation. Use `update` for new snapshots and
stable keys for focus continuity.

## Rules for new work

1. Reuse `src/ui.ts`, `src/menu.ts`, typed icons and tokens. Keep `.ui-button`
   when adding local classes; use a supported variant instead of replacing it.
2. Keep data loading, business decisions and persistence in the existing owner.
   Pass data, translated labels and callbacks to the view.
3. Keep components cohesive. Domain-specific pieces may stay under `chat/` or
   `git/`; promote a generic composition when real consumers share it. Do not
   build a generic screen renderer, service or event bus.
4. Demonstrate the same implementation in the gallery with deterministic data.
   Never create a second mock-only version of the screen.
5. Cover domain mapping with unit tests. Use browser checks for actual focus,
   CSS layout or engine risks under the E2E scope policy.
6. Verify the import boundary, typecheck and closest affected tests. Run
   `npm run check` for changes that cross several modules.

For resource-view changes, the architecture checker permits only its model,
Settings matching, Desktop compositions, existing UI/DOM facades and shared package dependencies.
The compositions themselves have a stricter boundary: only UI/menu facades,
their stylesheet and the shared package are allowed.
Changes to that boundary must be deliberate and documented.

The [presentation contract](../contracts/desktop-presentation.md) defines
lifetime, input ownership and compatibility. The
[Design System](design-system.md) describes the existing component catalog.
