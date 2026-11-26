---
type: Architecture
title: "Compose a Desktop screen"
description: "The procedure for agents to make isolated screens, with examples that you can run."
status: stable
---

# Compose a Desktop screen

Start at [`src/components/README.md`](../../src/components/README.md) and
[`catalog.json`](../../src/components/catalog.json). This directory contains the
APIs, the styles and the stories of the Desktop components. You can run the
stories. The application uses the same exports. The catalog records the
production consumers.

Run `npm run dev` and open `/design-system.html`. Find a component with its name or
with its source path. Each component has a URL with a `component` id and a
`state`. Add `embed=1` to get a standalone canvas. A browser agent can examine
the rendered component. At the same time, a coding agent can follow the
manifest to the implementation and to the callers.

At this time, Chat, Git, icons, Markdown, Resources and Actions use these
components. The business effects stay in their controllers. The catalog is a
layer for navigation and examples. It is not a schema that renders the product.

## Reusable Desktop components

Import the components from `src/components/compositions.ts`. These components
use only the shared controls, the DOM helpers, the icons and their stylesheet.
They do not import i18n, resource models, Actions, IPC or provider catalogs. All
labels come from the caller. The API has types. The layout slots accept usual
elements that you make from the current primitives.

| Component | Input and responsibility | Production use |
| --- | --- | --- |
| `sectionHeader` | Optional title, count, description, related controls and actions. It goes to a new line when the width is not sufficient. | Resources introduction. Actions command and profile sections. Tasks introduction. |
| `toolbar` | Arrays of controls for the start and the end. It puts the search field, the filters and the actions in position. It does not own their rules. | The search and filter bars of Resources and Tasks. Section headers. |
| `itemRow` | Title, description, icon with a type, optional status or busy state, metadata and action slots. It returns named DOM parts for local additions. | Resource rows. Command and profile cards. |
| `overflowAction` | Accessible label, stable focus key, menu items and busy flag. It uses the shared keyboard and menu lifecycle again. It disables the actions that are not available. | Resource menus. Card and template menus of Actions. |
| `listState` | A ready, empty, loading or error state. An optional retry callback with a label. | Resource feedback. Empty command and profile lists. |

Open `/design-system.html#components-preview` for an isolated composition of
these parts. It does not import Resources or Actions. Search, pending states,
error recovery and local callbacks operate without application services. The
same parts also operate in two Desktop pages of the product, not only in a
gallery.

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

The caller supplies `labels`, `item`, `onAdd`, `onEdit` and `host`. The
component does not load or save data when it makes its DOM. The row metadata
stays local to the screen. Do not add flags for a domain to the shared
component. Each call makes a new DOM. These components have no subscriptions and
no update loop. The screen owns the replacement and the focus restoration.
Close the open menu of the screen before you remove its controls. The resource
view does this lifecycle at this time.

## Start from the executable example

1. Run `npm run dev`.
2. Open `/design-system.html#resources-preview`.

   The preview imports the production presentation.
3. Select one of these states: populated, empty, loading, error, pending,
   unavailable or long text.
4. Type a name that is not in the list to see the empty result.
5. Edit an example item to update its snapshot. The focus stays on the item.

All example actions stay local.

Read these files. Do not copy markup from a hub.

- `src/resources/gallery.ts`: a composition that you can run, with sample data.
- `src/components/resource-view.ts`: the presentation inputs and the lifecycle.
- `src/resources/model.ts`: the items with types and the search logic.
- `src/resources/labels.ts`: the desktop i18n adapter.
- `src/components/resource-view.css`: the local layout with the shared tokens.
- `packages/design-system/README.md`: the current control APIs and the
  accessibility.

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

The import paths above are for a caller in `src/`. Example resource data can be
literal text. Interface labels must use i18n. Supply the callbacks of the
product in the desktop adapter, not in the presentation. Use `update` for new
snapshots. Use stable keys to keep the focus.

## Rules for new work

1. Use `src/ui.ts`, `src/menu.ts`, the icons with types and the tokens again.
2. Keep `.ui-button` when you add local classes. Use an available variant. Do
   not replace the class.
3. Keep data loading, business decisions and persistence in the current owner.
4. Give the data, the translated labels and the callbacks to the view.
5. Keep each component on one topic. Parts for one domain can stay in `chat/`
   or `git/`.
6. When two or more consumers use the same composition, move a generic version
   into the shared components.
7. Do not build a generic screen renderer, service or event bus.
8. Show the same implementation in the gallery with deterministic data.
9. Do not make a second version of the screen only for the mock.
10. Use unit tests for the domain mapping.
11. Use browser checks only for focus, CSS layout or engine risks. Obey the E2E
    scope policy.
12. Examine the import limit, the typecheck and the nearest related tests.
13. Run `npm run check` for changes that go across many modules.

For changes to the resource view, the architecture checker lets the view import
only these items:

- Its model.
- The Settings search logic.
- The Desktop compositions.
- The current UI and DOM facades.
- The dependencies of the shared package.

The compositions have a stricter limit. They can import only the UI and menu
facades, their stylesheet and the shared package. Change that limit only on
purpose, and write the change in the documentation.

The [presentation contract](../contracts/desktop-presentation.md) specifies the
lifetime, the owner of each input and the compatibility. The
[Design System](design-system.md) gives the current component catalog.
