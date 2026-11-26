---
type: Architecture
title: "Prometeu Design System"
description: "The tokens, the components, the gallery and the adoption of the design system in steps."
status: stable
---

# Prometeu Design System

The design system is a set of shared components that you can run.
[ADR 0017](../decisions/0017-executable-design-system.md) records the decision.
The package contains the rendering, the behavior, the styles and the Rails
adapter.

## Source of truth

- [`packages/design-system`](../../packages/design-system/README.md): the
  `@prometeu/design-system` package. It is the canonical source of colors,
  fonts, radii, spacing, DOM controls, menus, dialogs, icons, styles and the
  Rails adapter. The token names and values do not change. The package gives
  the APIs, the imports, the lifecycle and the accessibility contracts.
- [`src/ui-tokens.css`](../../src/ui-tokens.css): imports the tokens. It keeps
  only the geometry and the states that only the desktop uses.
- [`src/ui.css`](../../src/ui.css): imports the shared components and keeps the
  legacy control rules. The legacy button selectors do not include
  `.ui-button`. Thus, the shared buttons keep the package contract. `style.css`
  imports that base. It continues to contain some styles for screen composition,
  together with the feature stylesheets.
- [`src/ui.ts`](../../src/ui.ts): exports the components of the package again.
  It has no local implementation of `button`, `input`, `field`, `checkbox`,
  `select`, `dropdown`, `disclosure` or `formDialog`.
- [`src/menu.ts`](../../src/menu.ts) and [`src/icons.ts`](../../src/icons.ts):
  export the generic menus and icons of the package again. The icons for files
  and providers stay in the presentation adapter of the desktop.

The components get text that the caller translated before, and callbacks. They
do not import IPC, stored state, the agent catalog or business rules. A screen
composes these primitives and keeps its own domain validations.

## Usage and accessibility

`field(label, control, help)` connects the name and the description to the
control. `input` keeps the native `required`, `pattern`, `min` and `max`. When
the domain validation finds an error, use `aria-invalid="true"` and a related
error text. `checkbox` keeps the native input in the label. Its dimension does
not change with the text fields. A long label goes to a new line adjacent to
the control.

`toggle` uses the native checkbox again with switch semantics. `radio` keeps the
native keyboard behavior of a radio group. The two controls are in the
standalone galleries and in the notification preferences.
`e2e/notifications.spec.ts` examines the keyboard use.

`select` returns `control`, `value`, `onchange` and `setOptions`. The selector
uses the same `dropdown` as the launcher and the shared menu. The model
selection updates its options without a new component. If the consumer uses
the native form submission, a necessary selection uses `root` and the
`name`/`required` attributes. The `native` field is part of `FormData` and of
the HTML validation. Legacy screens that use only `control` and send their own
commands continue to validate the domain rules before they save.

When a dropdown opens, the focus goes to its menu options. The arrow keys move
the focus, not only the visual selection. When the dropdown closes, the focus
goes back to the trigger. A submenu opens adjacent to its row. If there is no
space at the right edge of the window, the submenu opens on the left. Thus, a
submenu does not cover the menu that opened it at any time.

`formDialog` uses `dialog.showModal()`. These conditions apply:

- The content behind the dialog becomes inert.
- When the dialog closes, the focus goes back to the opener.
- Tab and Shift+Tab move the focus only through the form.
- The body scrolls and the footer stays in view.
- The footer buttons have an explicit tab stop. Thus, WebKit does not skip them
  during keyboard navigation.

When the person saves, the dialog obeys the HTML validation and prevents a
second submission. It sets `aria-busy` and shows failures in `role="alert"`. The
typed text stays in the fields. A menu that opens in the dialog goes into the
same layer. The first Escape closes the menu, and the second Escape closes the
dialog. Global shortcuts do not change the workspace while the dialog is open.
While a submission is pending, Escape and cancellation do not close the dialog.
Thus, the progress and the failures stay in view.

The shared dialogs use the same spacing for the header and the body. Their
buttons have the dimension of their content, and the footer actions are in a
group. A form for text entry puts the focus on its first field. A dialog that
starts with actions or a selection puts the focus on the title. Thus, no button
shows as selected when the dialog opens. Tab continues to go to the controls
with a focus indicator that you can see. Shift+Tab from the title goes to the
last enabled control.

The confirmation in the standalone gallery shows this initial state.
`e2e/design-system.spec.ts` examines it, and `e2e/projects.spec.ts` examines
the Projects flow. A background refresh can replace the opener of a dialog. In
that condition, the opener keeps the focus restoration through its stable `id`
or `data-focus` key.

## Interaction metadata

Desktop consumers give stable control identities through `data-track` or
through the optional `track` arguments and options of the current controls.
`identifyControl` and `identifyArea` only set metadata. Menus, dialogs and
pickers with a search field keep the explicit area of the opener. This also
applies when they render in a portal. Each action in a menu has its own `track`.
The text that a person sees does not set the identity at any time.

The shared package has no analytics transport and no dependency on the app.
The desktop capture and the coverage checks are out of the package. Refer to the
[product analytics contract](../contracts/product-analytics.md). Current callers
without tracking options keep their behavior.

## Adoption

The command and agent editors in Actions use the shared fields, selectors,
checkboxes, disclosures and dialog. The launcher composes shared fields,
buttons, text input and checkboxes into two columns. The first column is for
the task and the repository. The second column is for the agent and the tools.
In a narrow window, the columns are one above the other in a body that scrolls.
The branch summary and the actions that make a workspace stay in a different
footer that you can always see.

The model search, the favorites and the native effort selection use the current
pickers. The optional skill for the first prompt is a ghost button in the attachment row of
the prompt. It opens the picker of the launcher, which has a search field.

Account identities go to a new line. They are not cut. The account popovers
use one row to add and manage accounts. The account dialog keeps its footer in
view while its cards scroll.
[`e2e/launcher-layout.spec.ts`](../../e2e/launcher-layout.spec.ts) examines the
launcher layout and the focus in the account dialog. It uses the English
interface at representative viewport dimensions, with selected WebKit checks.

The text fields in the MCP and plugin hubs use `input` and `field`. This also
applies to the request that makes a plugin. The worktree cleanup also uses
`formDialog`, with its protection against cancellation when busy and with shared
checkboxes. Their application callbacks own the progress labels and the
operation results. The legacy modals in the hubs keep the previous lifecycle.

Screenshots of the browser mock: [workspace](../images/ui/workspace.png),
[narrow workspace](../images/ui/workspace-narrow.png),
[account popover](../images/ui/accounts.png),
[account selector](../images/ui/account-dialog.png),
[model picker](../images/ui/model-picker.png), and
[Projects dialog](../images/ui/projects-dialog.png).

New controls and changes to current controls must use this base. If a behavior
is missing, add it to the related primitive and show the state in the gallery.
Do not copy CSS from one screen to a different screen. Do not add a generic
abstraction if two or more consumers do not use it. Chat or Git components that
have one topic can stay in their domain. They must also give isolated examples
and APIs with types.

## Desktop catalog and production adoption

[`src/components/`](../../src/components/README.md) is the entry point where you
can find the Desktop components. Its `catalog.json` connects component ids to
source files, exports, states and the consumers in the product. The gallery has
a search, URLs for each state (`/design-system.html?component=diff&state=split`) and
isolated canvases (`&embed=1`). Its JSON link stays available in the production
build.

At this time, ChatView calls the block, work, request, composer and attachment
components of the catalog. ChatView keeps the transport, the timeline
coordination, the drafts and the changes to the permission mode. Workspace
Changes calls the Git file rows, the groups and the commit form. The operations
and the protection against state that is not current stay in the adapter. The diff reader owns its
observer and caches for each instance. The current Desktop adapter supplies the
stored review state. Icons and Markdown also have their canonical Desktop source
in `components/`. They have compatibility exports for the current consumers.

The stories import those production implementations with deterministic sample
data. They do not start the app, start the mock or start real operations. The
catalog test makes sure of these conditions:

- The source files are there.
- Each component has stories.
- Each declared production consumer gets to its component through imports.

The boundary checks reject integration imports and network or storage effects
in component code. The gallery gives two diff readers that operate
independently, for visual inspection.

## Reusable Desktop compositions

`src/components/compositions.ts` gives section headers, toolbars, item rows,
overflow actions and list states with types. These compositions are above the
package primitives. Resources, Actions and the Tasks inbox use them.
`src/components/compositions.css` owns the shared layout. The screen styles keep
only their variants. These Desktop components have no feature imports or
application effects. They do not change the Cloud API that the package gives.

The [composition recipe](desktop-composition.md) gives each API and its use in
the product. `/design-system.html#components-preview` shows an isolated
screen that these parts make, with local callbacks and states that you can
select.

## Settings composition

Analytics composes `components/usage-history.ts` from shared headers, buttons
and the read-only pattern of the form dialog. Its gallery entry is
`/design-system.html?component=usage-history&state=ready&embed=1`, with the
ready, empty and partial states. `telemetry-settings.ts` keeps the filters and
the view when Settings renders again. The query invalidation and the erasure
stay in that host. The component uses formatted snapshots without IPC or
provider imports.

`src/settings.ts` puts the preferences into these groups: General, Agents,
Resources, Actions, Projects, Work and team, and Analytics. The navigation keeps
the small, neutral sidebar. The settings search is in the content header.
`src/settings.css` specifies the composition and replaces some control
dimensions. The base Settings styles also stay in `src/style.css`. Shared
primitives and legacy controls are in use at the same time. Actions keeps the shared button
classes when it adds local styles.

The resource library uses `src/components/resource-view.ts` and
`resource-view.css`. These files do not use the legacy row selectors. Hubs give
data with types and callbacks, and keep the validation and the busy state. The
item operations are in an overflow menu. The Add resource menu keeps all the
current paths to make and import resources. When a person adds a resource,
Prometeu does not automatically enable it.

Projects uses the shared toolbar, item rows and empty state, with a filter for
each computer. The procedure to add remote folders and the clone operation use
form dialogs that are only in Projects. The launcher selects from the
projects that are in this list.

General keeps the primary notification control and the native permission
warning in view while its other settings open in position. Agents shows the defaults,
then small account cards. The quota disclosures do not change the current
presentation of the account picker. The account notices, the login progress
and the selection stay in view. The search and the resource filters do not keep
business state.

Background updates keep the search value, the focus, the open disclosures and
the scroll position. The menus for a resource snapshot that is not current close before
the replacement. `src/settings-navigation.test.ts` examines the search logic and the
legacy page destinations. `e2e/settings.spec.ts` examines the focus restoration
and the disclosure geometry in Chromium and WebKit. When the browser replaces
DOM, it can remove the keyboard focus or limit the scroll before a disclosure
opens again. Pure search tests cannot show this behavior. The current scenarios
for accounts, the Cloud catalog, MCP, notifications, projects and organizations
continue to examine their production paths through the grouped interface.

The [composition recipe](desktop-composition.md) shows how agents can use the
same presentation with synthetic data. The
[presentation contract](../contracts/desktop-presentation.md) specifies its
inputs and lifecycle. Other Settings pages keep their current integration.

## Gallery and verification

`/packages/design-system/index.html` is the standalone gallery of the company.
It renders components through the built API of the package, without the
styles or the JavaScript of the app. It shows a menu, a keyboard submenu, a
password, a selection and an asynchronous dialog with a simulated failure.

The Cloud uses the runtime, the styles and the Ruby adapter in
`vendor/design-system`. `bin/design-system` imports these files, and CI validates
them with SHA-256. The Cloud views use `ds_form_with`, `form.field`,
`form.button` and the helpers of the package. These helpers keep the markup out
of the screens.

The desktop and the application screens of the Cloud use the default compact
density. `ui-comfortable` (44px) is for the touch flows of the Cloud, for
example the login and the Mac authorization. The standalone gallery also uses
it. The desktop gallery uses the compact density with the CSS of the app. A
change starts in the package and goes to the Cloud through a new import. Do not
edit the copied files in the Cloud at any time.

Run `npm run dev` and open `/design-system.html` on the same port. The gallery is
also an entry point of the web build. It does not start the backend or agents.
It shows tokens, buttons, fields, selection, error, disabled, checkbox, a menu
with a tag, a disclosure and a form with a simulated success or failure.
`#resources-preview` renders the production Resources view with these examples:
populated, empty, loading, recoverable error, pending, unavailable and long
text. The search and the filter operate locally. The example callbacks do not
call the hubs at any time.

[`e2e/design-system.spec.ts`](../../e2e/design-system.spec.ts) examines the
keyboard, the focus, the validation, a recoverable failure and a narrow viewport
in the standalone consumer.
[`e2e/actions.spec.ts`](../../e2e/actions.spec.ts) examines the primitives in
the flows of the feature. The architecture check prevents a dependency from
`ui.ts` on domain modules. It also prevents a new version of the native
selectors in Actions. The current web and E2E tests keep the launcher, the
menus and the hubs safe during the adoption.

## Searchable selection

`searchablePicker` composes through the lifetime of the shared menu. It can open
in a current modal. It accepts these inputs:

- Translated labels.
- Stable item keys.
- Groups and descriptions.
- Optional secondary actions.
- Status text.
- A refresh.
- A filter checkbox.

It does not own domain catalogs or persistence. Its update method keeps the
query, the scroll and the focused action. The model picker subscribes to catalog
changes and supplies the updated entries. With a `search` callback, the caller
puts the results of each query in order. The Command-P file search sends the
fuzzy `find_paths` results of the backend without changes. Command-F in the
project view and its search button open the same file search. When the view
changes, the search closes and ignores the answers to its previous queries.
A focused text field, the file viewer and a terminal keep Command-F.

The model favorites are secondary buttons. They are not nested interactive
content in a button that you can select. The rows become taller when long model
names and descriptions go to a new line. The square favorite buttons keep their
focus indicator in the scroll area. `e2e/model-picker.spec.ts` examines these
limits in the desktop composition, also in narrow windows.

When the picker opens, the focus goes to the search input. The arrow keys move
between the enabled items. Tab goes to the current item, its secondary
action and the footer controls. It is not necessary to go through the
full catalog. Escape closes the picker before the parent dialog, and the focus
goes back to the trigger.

The standalone gallery and `e2e/search-picker.spec.ts` examine these items in
Chromium and WebKit:

- A list of 100 entries.
- A filter that ignores accents.
- Keyboard and pointer selection.
- The focus order.
- Narrow dialogs.

The gallery also shows star actions that operate independently, and dynamic
updates.
