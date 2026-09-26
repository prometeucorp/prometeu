# Desktop components

This directory is the entry point for agents and people composing Desktop UI.
The application and gallery use these implementations. Stories contain sample
data and callbacks, never a parallel implementation of a component.

## Find and inspect a component

1. List this directory or read [`catalog.json`](catalog.json). Every entry names
   its source, exports, production consumers and available states.
2. Run `npm run dev`. Open `/design-system.html` to search the catalog.
3. Open `/design-system.html?component=chat-block&state=error` for a component
   and state. Add `&embed=1` for a canvas without navigation, useful for browser
   agents, screenshots and visual comparisons.
4. Follow the manifest's `source` path to the typed API. `stories.ts` shows
   runnable inputs; `consumers` identifies real integration code.

The gallery also links to the JSON manifest as a build asset. URLs work in
Vite dev and the production preview. No Storybook runtime or backend is needed.
The same state identifiers are stable navigation keys and machine-readable data.

## Ownership

| Family | Source | Production adoption |
| --- | --- | --- |
| Buttons, fields, selection, dialogs, notices | `primitives.ts`, `menu.ts` | Existing Desktop imports route through `src/ui.ts` and `src/menu.ts` |
| Icons, file icons, stages, provider marks, avatars | `icons.ts`, `icon-button.ts` | Existing `src/icons.ts` facade, chat composer, Git toolbar |
| Headers, toolbars, rows, list states | `compositions.ts` | Resources and Actions |
| Resource library | `resource-view.ts` | Settings Resources |
| Tool, reasoning, text, work and error blocks | `chat/blocks.ts` | `ChatView` in workspace and desk |
| Questions, plans, permissions | `chat/requests.ts` | `ChatView`; callbacks send canonical responses in the host |
| Composer and attachments | `chat/composer.ts` | `ChatView`; host owns drafts, completion, voice and transport |
| User message, browser context, tool input, context report | `chat/content.ts` | Desktop and existing mobile consumers via compatibility facade |
| Markdown and fenced code | `chat/markdown.ts` | Existing consumers via `src/markdown.ts`; preserves escaping and copy behavior |
| Git file rows, groups, commit form | `git/file-row.ts`, `git/group.ts`, `git/commit-form.ts` | Workspace Changes |
| Diff reader | `git/diff-view.ts` | `src/diff.ts` adapter; review persistence remains there |

`primitives.ts` and `menu.ts` re-export the existing design-system package; there
is one implementation, not a Desktop fork. The manifest records that package
as `implementation`. Desktop-specific components remain here. Pure diff parsing
and signatures are in `git/patch.ts` and retain their existing unit tests.

`styles.css` provides the Desktop component baseline. The app's `style.css`
imports it and adds shell layout. Chat and Git styles sit beside their component
families; the gallery loads the app stylesheet to expose the real CSS cascade.

## Compose and integrate

```ts
import { button } from "./components/primitives";
import { itemRow, sectionHeader } from "./components/compositions";

host.append(sectionHeader({ title: labels.title, actions: [
  button(labels.add, onAdd),
] }));
host.append(itemRow({
  title: item.name, description: item.description, glyph: "terminal",
  actions: [button(labels.edit, () => onEdit(item.id), "ghost")],
}).root);
```

The example assumes a caller in `src/`. The host supplies translated `labels`,
presentation data, callbacks and the target DOM node. Domain-specific components
may use the Desktop i18n adapter; this reads the existing language preference,
not application data. Shared compositions accept already-translated labels.

Components may render and manage transient interaction state. They must not
invoke IPC, load provider catalogs, write business storage or make network
requests. Hosts decide authorization and availability. A disabled control is
presentation, not authorization. The host still validates actions at execution.

Fresh-node components need no subscription cleanup. Close an owned menu before
removing its trigger. `resourceView` exposes `update` and `destroy`. Each
`diffView` instance owns its observer, cache and collapsed state; call `destroy`
when retiring it. Review state is supplied through `isSeen` and `setSeen`, so
multiple readers and gallery examples cannot modify each other's storage.
Composer returns named controls for host updates without replacing typed input.

## Add or extract a component

- Keep a cohesive responsibility in the appropriate family. A product-specific
  component does not need an artificial second screen to justify isolation.
- Change the real consumer to call it and remove the previous markup there.
- Add its manifest entry and stories using the production export. Show relevant
  states and meaningful callbacks, with deterministic sample data.
- Preserve existing layout, keyboard behavior, selection, drafts and contracts.
- Run the closest tests, `npm run architecture:check`, then `npm run check` for
  changes spanning layers. The catalog test verifies production reachability.

The [presentation contract](../../docs/contracts/desktop-presentation.md) defines
compatibility. The [composition recipe](../../docs/architecture/desktop-composition.md)
explains the integration boundary. Remaining app-specific layouts do not become
catalog components merely by being moved to a new filename.
