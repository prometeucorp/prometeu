---
type: Contract
title: "Desktop presentation"
description: "Typed snapshots, callbacks, focus and the life of the resource view."
status: stable
---

# Desktop presentation

The Desktop catalog, the isolated Resources view, the shared compositions and
the chat and Git components are complete. Decision:
[ADR 0060](../decisions/0060-isolated-desktop-presentation.md).

## Boundary

`src/components/resource-view.ts` controls the DOM, the filter, the focus and
the layout of the resource library. `src/settings-resources.ts` connects that
presentation to the desktop hubs. The same view operates in
`/design-system.html#resources-preview` with synthetic items and simulated
callbacks. When the view starts, it does not start IPC, Tauri, the browser mock,
the provider catalogs, the storage or a backend.

`ResourceItem` and `ResourceSnapshot` are in `src/resources/model.ts`. They are
presentation types in the process. They are not stored formats or network
formats:

- `key` is different for each item across all types and grouped catalog
  entries. It stays the same after an update, and the view uses it to put the
  focus on the same item again. The hubs put definitions that are not installed
  into groups by name. When the hubs compare names, they ignore the
  difference between capital letters and small letters. Installed items keep their local identity.
- `kind` selects a filter and a translated type label.
- `description`, `origins[].label`, the optional origin hints and `status` are
  plain text. The view renders the source badges, which include the local
  `here` marker. The callers translate the interface text. Resource names and
  descriptions stay in their source language.
- `glyph` is a typed icon name. The view renders the icon. The callers do not
  supply HTML.
- `actions` are shared menu items with callbacks. The hubs control the
  authorization, the confirmation, the errors, the installation and the
  storage. The construction of a snapshot must not start an action or make
  hidden controls.
- `busy` disables the action menu of an item. An operation can continue after
  a new render. The hub that owns the operation monitors it. A temporary DOM
  node does not monitor it.
- `scope` is optional. It records the identity of the organization for
  inspection.

`ResourceSnapshot` supplies the items and an optional loading or error state.
At this time, the desktop gives hub snapshots and the operation states of the
items. The view can show a loading state and an error with a retry for
the full list. The isolated gallery shows these inputs. This extraction does not change
the current initial load and error policy of the hubs. A snapshot with a failure
can keep the current items.

The view accepts these inputs:

- Translated labels.
- An initial `{ filter, query }`.
- A `changed` callback.
- Default navigation.
- The construction of the Add resource menu.
- An optional retry callback.

The Add callback receives its anchor for the current import menus. The
callbacks start only when a person interacts with the view. The owner of each
callback must process its failures.

## Lifetime

`resourceView(options)` returns `root`, `update(snapshot)` and `destroy()`.
An update closes the open menu of this view before it replaces the rows. The
update keeps the search text and the filter. It puts the focus on the same
action again through its stable key. If the update removes or disables the
action, the focus moves to the search field. `destroy()` closes the menus that the view owns and removes the
root. When the Settings coordinator of the desktop makes a page again, it closes
the snapshot menus. It also sets the focus, the disclosures and the scroll
position of the page to their previous values.

The view accepts usual data and callbacks. It does not accept DOM from a hub.
It does not examine the `.txt` or `.act` nodes of a different feature, and it
does not make a general screen schema.

`src/components/compositions.ts` contains section headers, toolbars,
item rows, overflow controls and list feedback that more than one screen can
use. Resources and Actions use these compositions. The compositions accept translated strings, typed icons,
presentation slots and callbacks. They do not have feature imports,
subscriptions or application effects. The construction returns new DOM. The
hosts control the replacement, the menu cleanup and the continuity of the
focus. `overflowAction` uses stable keys and disables menus that are busy or
fully unavailable. `listState` uses a discriminated union, thus only errors
have retries. Actions keeps its current editor and storage integration. The
isolated component gallery does not start that integration.

## Chat and Git components

`src/components/chat/` contains these items:

- Conversation blocks and work summaries.
- Request cards and held messages from teammates.
- The composer structure and attachments.
- Content helpers and Markdown.

The canonical `Block` and `Ask` data are presentation inputs. The components do
not have ChatView, Tauri or catalog subscriptions. Domain components use the
current i18n adapter, which reads the language preference. Shared layout
components receive labels.

`requestCard` reports a typed `RequestResponse`. `allowAlways` is a different
callback. ChatView continues to send the change of permission mode. It waits
for a correct result before it gives approval to the request. Local answers to
questions and the feedback focus stay in the card. The host keeps the identity
of an open feedback request.

`pendingCard` shows the held message of a teammate as a card that shows that the
person must do an action. It reports Send, Edit and Discard through callbacks. The text renders
as typed text. It does not render as markup. The collaboration core keeps, sends and
discards the message. Before ChatView moves an edited message into the shared
draft, it stops the dictation
([ADR 0090](../decisions/0090-approved-remote-input.md)).

`composer` returns its root, its textarea and named controls. ChatView supplies
the click callbacks. ChatView also keeps these functions:

- Completion and keyboard input.
- Paste and voice.
- Transport and capability checks.
- Draft storage.

 When
the model, tool or busy indicators change, the textarea stays the same. The desk
and the workspace keep their current behavior for the shared draft.
`attachmentChip` reports a removal and does not access files.

The entry to a workspace can wait for a conversation snapshot. During that
wait, the person can select a supporting shell. If the person does this, that
shell keeps the center area and the keyboard focus when the attachment is
complete.
The load of a transcript cannot replace a tab selection that the person made
after it. This prevents an accidental submission of a composer draft that the
view keeps.

`contextGauge` updates its open panel from the latest snapshot of the host. The
snapshot includes the availability of the actions and the callbacks. The gauge
keeps the scroll position and the focus on an enabled action. If that action is
disabled or removed, the focus goes back to the panel. Thus Tab and Escape
continue to operate. The gauge does not control the conversation status or
commands.

Git file rows and groups receive display data, the selected and closed
state, and callbacks. `commitForm` only processes the message input and the
validation of an empty message. The host supplies the availability and
validates the operation that the backend does. These functions stay in `workspace-changes.ts`
and the backend:

- The staging of files.
- Refusals and checks for a repository state that is not current.
- The processing of merge conflicts.
- The storage of commits.

Each `diffView({ isSeen, setSeen })` owns its cache, its closed state and its
viewport observer. `render(host, snapshot)` keeps the focus and the scroll
position. `invalidate`, `fold` and `foldAll` change only that instance.
`destroy` disconnects the observer and releases the cached elements. The host
owns the DOM container and removes it when it stops the reader. The current
`src/diff.ts` adapter supplies the storage of the review. It keeps the same
storage keys and patch signatures. Each gallery reader uses its own set in memory. The parse, the line limits and the lazy render do not change.

## Discovery contract

`src/components/catalog.json` lists the ids, sources, exports, states and
production consumers. The primitives from a package also identify the source
of their implementation. Each entry has an executable factory in `stories.ts`.
The samples import the production components and supply local callbacks. The gallery
uses the `component` and `state` query parameters, and `embed=1` for a
standalone canvas. An unknown component shows an error. An unknown state
selects the first state that the component can show. The gallery does not make
a dynamic module path from the URL input. The build links the JSON manifest as
a Vite build asset.

The compatibility facades (`ui.ts`, `menu.ts`, `icons.ts`, `chat-presentation.ts`
and `markdown.ts`) keep the current imports and do not copy the implementation.
The component folder is the discovery surface. This contract does not add a
screen schema that a tool makes, or a different rendering runtime.

## Visual ownership

`src/components/compositions.css` controls the shared layout and the text
hierarchy. `src/components/resource-view.css` controls the resource columns and
the container breakpoints. It uses the shared tokens and its own named container
queries. It does not use the previous row selectors of Settings. The narrow
layout removes the icon and its grid column. The library keeps the current warm
surfaces, small radii and compact controls.

The previous button rules in `src/ui.css` do not apply to `.ui-button`. Shared
buttons keep the distances, the colors and the focus behavior of the package. A
screen can continue to have its own composition styles. When Actions adds its
local classes, it keeps the shared class. The shared package adds the read-only
`iconNames` catalog export. No current package API or Cloud consumer changes.

## Compatibility and proof

No IPC, stored identifier, installation rule or network format changes. The
previous destinations in Settings keep their navigation map. The owners of these
flows do not change:

- The current MCP connection.
- The OAuth login that is not complete.
- The catalog confirmation, the revision and the local copy.

The item sequence stays plugins, MCP,
skills. The built-in MCP stays read-only. Definitions with the same name share a
row with all source badges and with installation callbacks for each source.
This keeps the group rules and the link rules of the catalog.

- `src/components/catalog.test.ts` examines the coverage of the manifest by
  the stories and the import reachability from production. It does not use the
  imports to find if the visual result is correct.
- The current Git, conversation, Markdown and file-drop journeys prevent
  regressions in the production integration, the streaming, the drafts, the
  safety limits and the copy behavior.
- `src/resources/adapters.test.ts` makes production hub snapshots without DOM.
  It examines the grouped origins and the actions for each source. It also
  examines the MCP connections that are not complete and the installation locks
  across refreshes.
- `src/settings-navigation.test.ts` prevents regressions in the previous page
  destinations and in the rules that find them.
- `e2e/settings.spec.ts` prevents regressions in these items in Chromium and
  WebKit:
  - The keyboard focus of shared rows and menus, and the narrow geometry, in
    an isolated composition and in the Actions consumer.
  - The focus through snapshots.
  - The interaction with the isolated gallery and the narrow layout.
- `e2e/mcp.spec.ts` and `e2e/cloud.spec.ts` keep the coverage of the desktop
  integration.
- `e2e/design-system.spec.ts` compares the compact button styles between hosts.
- `scripts/architecture-dependencies.test.mjs` prevents regressions in the
  transitive import boundary. It is an import check. It does not prove that the code does not use
  an ambient browser API.

A person can look at the isolated chat and Git stories. These stories do not
have a dedicated E2E scenario. The current product journeys examine their
integration.

See the [agent composition recipe](../architecture/desktop-composition.md).

The `allowAlways` callback of the request card is optional. A host that cannot
change the permission policy does not supply it. Then the card does not show
the permanent approval and the plan-bypass actions. The card continues to show
the approval of one request, the denial and the answers to questions. The
current Desktop consumers supply the callback and keep their behavior.

## Project management

Settings > Projects makes the shared toolbar, the item rows and the empty state
from the board projects that the person added. When a person selects a computer, the page
filters by `Project.host`. It does not merge origin aliases. The saved
`projects` destination opens this page. The machine preparation and the Claude
login stay in Work and team > Remote machines.

The registration of a remote folder and the clone of a repository use shared
form dialogs from `src/projects.ts`. The launcher also opens these dialogs, and
it does not discard its task draft. If the registration adds only some folders, it
keeps the folders that it added. After a failure, the clone keeps the URL
and the destination. While the write continues, the dialog cannot close. No
stored format or IPC format changes.
