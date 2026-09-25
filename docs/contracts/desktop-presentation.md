# Desktop presentation

Status: Desktop catalog, isolated Resources view, shared compositions, chat and Git components implemented. Decision:
[ADR 0060](../decisions/0060-isolated-desktop-presentation.md).

## Boundary

`src/components/resource-view.ts` owns the resource library's DOM, filtering, focus and
layout. `src/settings-resources.ts` binds that presentation to the desktop hubs.
The same view runs in `/design-system.html#resources-preview` with synthetic
items and simulated callbacks. Mounting the view does not initialize IPC, Tauri,
the browser mock, provider catalogs, storage or a backend.

`ResourceItem` and `ResourceSnapshot` live in `src/resources/model.ts`. These are
in-process presentation types, not persisted or network formats:

- `key` is unique across kinds and grouped catalog entries, stable across updates,
  and restores focus. Pending definitions group by case-insensitive name;
  installed items retain their local identity.
- `kind` selects a filter and a translated type label.
- `description`, `origins[].label`, optional origin hints and `status` are plain
  text. The view renders source badges, including the local `here` marker. Callers
  translate interface copy; resource names and descriptions retain their source language.
- `glyph` is a typed icon name. The view renders the icon; callers do not supply HTML.
- `actions` are shared menu items with callbacks. Hubs own authorization,
  confirmation, errors, installation and persistence. Constructing a snapshot
  must not execute an action or construct hidden controls.
- `busy` disables an item's action menu. Pending operations that outlive a
  redraw remain tracked by the owning hub, not by a temporary DOM node.
- `scope` optionally records organization identity for inspection.

`ResourceSnapshot` supplies items and optional loading/error state. The desktop
currently passes hub snapshots and item operation states. Whole-list loading
and recoverable errors are supported presentation inputs demonstrated by the
isolated gallery; this extraction does not change the hubs' existing initial
load/error policy. A failed snapshot may retain existing items.

The view accepts translated labels, an initial `{ filter, query }`, a `changed`
callback, defaults navigation, Add resource menu construction, and an optional
retry callback. The Add callback receives its anchor for existing import menus.
Callbacks execute only on interaction. They must handle failures in their owner.

## Lifetime

`resourceView(options)` returns `root`, `update(snapshot)` and `destroy()`.
An update closes this view's open menu before replacing rows, retains search
and filter, and restores a focused action by its stable key. If the action is
removed or disabled, focus moves to search. `destroy()` closes owned menus and
removes the root. The desktop's Settings coordinator already closes snapshot
menus and restores page focus, disclosures and scroll when rebuilding a page.

The view accepts ordinary data and callbacks. It does not accept hub-produced
DOM, inspect another feature's `.txt`/`.act` nodes, or create a general screen
schema. `src/components/compositions.ts` holds reusable section headers, toolbars, item rows,
overflow controls and list feedback, consumed by both Resources and Actions.
It accepts translated strings, typed icons, presentation slots and callbacks.
It has no feature imports, subscriptions or application effects. Construction
returns fresh DOM; hosts own replacement, menu cleanup and focus continuity.
`overflowAction` uses stable keys and disables busy or wholly unavailable menus.
`listState` uses a discriminated union so retries belong only to errors.
Actions retains its existing editor and persistence integration. The isolated
component gallery does not initialize that integration.

## Chat and Git components

`src/components/chat/` owns conversation blocks, work summaries, request cards,
composer structure, attachments, content helpers and Markdown. The canonical
`Block` and `Ask` data are presentation inputs. Components have no ChatView,
Tauri or catalog subscriptions. Domain components use the existing i18n adapter,
which reads the language preference. Shared layout components receive labels.

`requestCard` reports a typed `RequestResponse`; `allowAlways` is a distinct
callback. ChatView still sends the permission mode change and waits for success
before allowing the request. Local question answers and feedback focus remain
in the card. The host keeps the identity of an open feedback request.

`composer` returns its root, textarea and named controls. ChatView supplies click
callbacks and retains completion, keyboard dispatch, paste, voice, transport,
capability checks and draft persistence. Updating model, tool or busy indicators
does not replace the textarea. Desk and workspace keep their existing shared
draft behavior. `attachmentChip` reports removal without accessing files.

Git file rows and groups receive display data, selected/collapsed state and
callbacks. `commitForm` only handles message input and empty-message validation;
the host supplies availability and validates the actual operation. Staging,
refusals, stale-repository checks, conflict handling and commit persistence stay
in `workspace-changes.ts` and the backend.

Each `diffView({ isSeen, setSeen })` owns its cache, collapsed state and viewport
observer. `render(host, snapshot)` preserves focus and scroll; `invalidate` and
`foldAll` affect that instance. `destroy` disconnects the observer and releases
cached elements. The host owns the DOM container and removes it when retiring
the reader. The existing `src/diff.ts` adapter supplies review persistence with
unchanged storage keys and patch signatures. Gallery readers use independent
in-memory sets. Parsing, line limits and lazy rendering remain unchanged.

## Discovery contract

`src/components/catalog.json` lists ids, sources, exports, states and production
consumers. Package-backed primitives also identify their implementation source.
Every entry has an executable factory in `stories.ts`; samples import the real
components and provide local callbacks. The gallery uses `component` and `state`
query parameters, plus `embed=1` for a standalone canvas. Unknown components show
an error; unknown states select the first supported state. No dynamic module path
is derived from URL input. The JSON manifest is linked as a Vite build asset.

Compatibility facades (`ui.ts`, `menu.ts`, `icons.ts`, `chat-presentation.ts` and
`markdown.ts`) preserve existing imports without copying implementation. The
component directory is the discovery surface; no generated screen schema or
alternate rendering runtime is introduced.

## Visual ownership

`src/components/compositions.css` owns reusable layout and text hierarchy.
`src/components/resource-view.css` owns resource columns and container breakpoints, uses shared tokens and its own
named container queries. It does not depend on legacy Settings row selectors.
The narrow layout removes both the icon and its grid track. The library keeps
existing warm surfaces, small radii and compact controls.

Legacy button rules in `src/ui.css` exclude `.ui-button`; shared buttons retain
the package's spacing, colors and focus behavior. Screen-specific composition
styles remain possible. Actions preserves the shared class when adding its
local classes. The shared package adds the read-only `iconNames` catalog export. No existing
package API or Cloud consumer changes.

## Compatibility and proof

No IPC, persisted identifier, installation rule or wire format changes. Former
Settings destinations retain their navigation mapping. Existing MCP connection,
pending OAuth, catalog confirmation, revision and local-copy flows remain in
their owners. Item order remains plugins, MCP, skills; built-in MCP stays
read-only. Same-name definitions share a row with all source badges and source-specific
installation callbacks, preserving the catalog grouping and linking rules.

- `src/components/catalog.test.ts` checks manifest/story coverage and production
  import reachability. It does not infer visual correctness from imports.
- `e2e/components.spec.ts` exercises independent diff observers, review state,
  request focus and composer callbacks without app bootstrap, in Chromium and
  WebKit. Parser tests cannot prove simultaneous DOM/observer behavior.
- Existing Git, conversation, Markdown and file-drop journeys protect production
  integration, streaming, drafts, safety guards and copy behavior.
- `src/resources/adapters.test.ts` constructs real hub snapshots without DOM,
  checks grouped origins and source-specific actions, and exercises pending
  MCP and installation locks across refreshes.
- `src/settings-navigation.test.ts` protects former page destinations and matching.
- `e2e/settings.spec.ts` protects shared row/menu keyboard focus and narrow
  geometry in an independent composition and the Actions consumer, plus
  focus through snapshots, isolated gallery
  interaction and narrow layout in Chromium and WebKit.
- `e2e/mcp.spec.ts` and `e2e/cloud.spec.ts` retain desktop integration coverage.
- `e2e/design-system.spec.ts` compares compact button styles between hosts.
- `scripts/architecture-dependencies.test.mjs` protects the transitive import
  boundary. It is an import check, not proof against every ambient browser API.

See the [agent composition recipe](../architecture/desktop-composition.md).
