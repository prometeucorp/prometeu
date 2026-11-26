# Desktop components

This folder is the entry point for agents and persons that compose Desktop UI.
The application and the gallery use these implementations. Stories contain
sample data and callbacks. They do not contain a parallel implementation of a
component.

## Find and inspect a component

1. List this folder or read [`catalog.json`](catalog.json). Each entry gives its
   source, exports, production consumers and available states.
2. Run `npm run dev`. To find a component in the catalog, open `/design-system.html`.
3. For one component and one state, open
   `/design-system.html?component=chat-block&state=error`.
4. For a canvas without navigation, add `&embed=1`. This canvas is good for
   browser agents, screenshots and visual comparisons.
5. Follow the `source` path of the manifest to the typed API.
6. Read `stories.ts` for inputs that you can run. `consumers` gives the integration code in production.

The gallery also has a link to the JSON manifest as a build asset. The URLs
operate in the Vite development server and in the production preview. A
Storybook runtime or a backend is not necessary. The same state identifiers are
stable navigation keys and data that a machine can read.

## Ownership

| Family | Source | Production adoption |
| --- | --- | --- |
| Buttons, fields, selection, dialogs, notices | `primitives.ts`, `menu.ts` | Current Desktop imports go through `src/ui.ts` and `src/menu.ts` |
| Icons, file icons, provider marks, avatars | `icons.ts`, `icon-button.ts` | The current `src/icons.ts` facade, the chat composer, the Git toolbar |
| Headers, toolbars, rows, list states | `compositions.ts` | Resources and Actions |
| Resource library | `resource-view.ts` | Settings Resources |
| Local usage and the data of each session | `usage-history.ts` | Settings Analytics |
| Tool, reasoning, text, work and error blocks | `chat/blocks.ts` | `ChatView` in workspace and desk |
| Questions, plans, permissions | `chat/requests.ts` | `ChatView`. Callbacks send canonical responses in the host |
| Teammate message that waits for the owner | `chat/pending.ts` | `ChatView`. The collaboration core keeps, sends and discards the message |
| Composer and attachments | `chat/composer.ts` | `ChatView`. The host owns drafts, completion, voice and transport |
| User message, browser context, tool input, context report | `chat/content.ts` | Desktop and current mobile consumers through a compatibility facade |
| Markdown and fenced code | `chat/markdown.ts` | Current consumers through `src/markdown.ts`. It keeps the escape behavior and the copy behavior |
| Git file rows, groups, commit form | `git/file-row.ts`, `git/group.ts`, `git/commit-form.ts` | Workspace Changes |
| Review notes, editor and summary | `git/review-note.ts`, `git/review-summary.ts` | The diff reader and the Workspace review controller |
| Review message tag | `chat/review-context.ts` | Chat drafts and portable history |
| Diff reader | `git/diff-view.ts` | `src/diff.ts` adapter. The storage of reviews stays there |

`primitives.ts` and `menu.ts` export the current design-system package again.
There is one implementation, not a Desktop fork. The manifest records that
package as `implementation`. The components only for the Desktop stay in this
folder. The pure parse of diffs and the signatures are in `git/patch.ts`, and
they keep their current unit tests.

`styles.css` gives the base styles of the Desktop components. The `style.css`
of the app imports it and adds the layout of the shell. The Chat styles and the
Git styles are adjacent to their families of components. The gallery loads the
stylesheet of the app to show the CSS cascade of the app.

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

The example uses a caller in `src/`. The host supplies the translated `labels`,
the presentation data, the callbacks and the target DOM node. Components for a
specified domain can use the Desktop i18n adapter. This adapter reads the
current language preference, not application data. Shared compositions accept labels that the caller translated.

Components can render and manage temporary interaction state. They must not
call IPC, load provider catalogs, write business storage or send network
requests. Hosts make the decisions about authorization and availability. A
disabled control is presentation, not authorization. The host continues to
validate actions at execution.

Components that make new nodes do not have subscriptions to clean. Close an
owned menu before you remove the button that opens it. `resourceView` gives
`update` and `destroy`. Each `diffView` instance owns its observer, its cache
and its collapsed state. Call `destroy` when you stop the use of the instance.
The host supplies the review state through `isSeen` and `setSeen`. Thus, many
readers and gallery examples cannot change the storage of each other. The
composer returns named controls for host updates, and it does not replace typed
input.

## Add or extract a component

- Put a component with one clear responsibility in the applicable family. For a component only for one product, an artificial second screen is not
  necessary to isolate it.
- Change the production consumer so that it calls the component. Remove the previous
  markup from the consumer.
- Add the manifest entry and the stories of the component with the production
  export. Show the applicable states and useful callbacks, with sample data that
  does not change.
- Keep the current layout, keyboard behavior, selection, drafts and contracts.
- Run the nearest tests and `npm run architecture:check`.
- For changes across layers, then run `npm run check`. The catalog test makes
  sure that production code uses each component.

The [presentation contract](../../docs/contracts/desktop-presentation.md) gives
the compatibility rules. The [composition recipe](../../docs/architecture/desktop-composition.md)
gives the integration boundary. If you move a layout only for the app to a new
file name, it does not become a catalog component.
