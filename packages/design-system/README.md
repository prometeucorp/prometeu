# Prometeu Design System

`@prometeu/design-system` 0.5.0 supplies executable components. They include
rendering, state, events, keyboard operation, focus, validation, CSS and the
brand. The implementation is in this package. The desktop imports the
TypeScript components. The Cloud uses the Rails adapter of the package and the
same JavaScript runtime in the browser.

The package does not use React, Vue, Web Components or JavaScript dependencies
at runtime. The Ruby adapter uses the native Action View and FormBuilder. The
business rules, the translations and the transport stay in the products.

## Installation

At the root of the Prometeu repository, run these commands:

```sh
npm run design-system:build
npm pack ./packages/design-system
```

The `prometeu-design-system-0.5.0.tgz` artifact includes ESM modules,
TypeScript declarations, a browser bundle, the Ruby adapter, CSS, SVG and the
gallery. `prepack` builds the JavaScript again. Thus, the package cannot contain
JavaScript that is older than the source. Nobody published the package to a
registry at this time.

In a different project, run this command:

```sh
npm install /path/to/prometeu-design-system-0.5.0.tgz
```

```ts
import { button, field, input, formDialog } from "@prometeu/design-system";
import "@prometeu/design-system/components.css";

const name = input("Team");
name.required = true;

const edit = button("Edit profile", () => {
  const dialog = formDialog({
    title: "Edit profile", save: "Save", cancel: "Cancel",
    submit: async () => { await saveProfile({ name: name.value }); },
    error: () => "Could not save. Try again.",
  });
  dialog.body.append(field("Name", name, "How you appear in the product."));
  dialog.open();
});
document.querySelector("main")!.append(edit);
```

`saveProfile` belongs to the consumer. The component does these tasks:

- It makes the dialog.
- It connects the labels to the controls.
- It validates the form.
- It prevents a second submit of the same data.
- It controls the state of the submit.
- It shows failures and keeps the data.
- It moves the focus back when the dialog closes.

The component does not send requests.

## Catalog of components

| API DOM/TypeScript | Responsibility |
| --- | --- |
| `button(label, callback, variant)` | a native button with the `outline`, `pri`, `ghost` and `danger` variants, focus and an event |
| `input(value, multiline)` | a native input or textarea. The HTML attributes stay available |
| `field(label, control, hint)` | the structure of a field, the label, the ID and an accessible description |
| `checkbox(label, checked)` | a native input and its label. It returns `control` and `label` |
| `password(value, {show, hide})` | a field and a button that shows or hides the value. The button does not replace the value |
| `select(value, options, attributes)` | a selection with a menu, keyboard operation, updates of the options and a native field for the submit |
| `dropdown(button, groups, get, set)` | the selection behavior on a current button |
| `menuButton(label, items)` | a menu of actions with focus, Escape, Home/End and submenus that the keyboard can open |
| `menu.openAt`, `menu.selectAt`, `menu.close`, `menu.onClose` | the context menu and its lifecycle. `openAt` returns the open panel. `selectAt` moves the keyboard selection to an item again. `onClose` returns a function that cancels the callback |
| `formDialog(options)` | an asynchronous modal form with validation, focus, the state of the submit and the recovery after an error |
| `confirmDialog(options)` | an accessible confirmation that resolves `Promise<boolean>` and accepts `AbortSignal` |
| `disclosure(title, ...content)` | a native disclosure without an external control of its state |
| `card`, `badge`, `notice` | the rendering of containers, indicators and feedback with the applicable roles |
| `avatar(image, kind, size)` | a photo or a logo. Without an image, it shows a decorative glyph of a person or an organization |
| `iconNames` | the typed names of the icons for catalogs and for discovery |
| `icon(name, size)` | a typed catalog of shared vector icons |
| `enhance(root)` | connects the HTML of the Rails adapter to the runtime. It returns a cleanup function |

`select` returns `control`, `root`, `native`, `value`, `onchange` and
`setOptions`. To make the selector a part of a native form, use `root` with
`{ name, required, disabled }`. The selector keeps the validation and the data
of `FormData`. `control` stays compatible with the screens from before that read `value`
and send their own commands.

`formDialog` returns `root` (the native dialog), `body`, `save` (the submit
button), `cancel`, `open`, and `close`. Consumers can use these elements to
compose content and to change labels or availability. The shared form owns the
validation, the focus, the busy state and the cancellation.

While a submit is not complete, Escape and cancel do not operate. If a submit
gives an error, the dialog stays open and shows the error. The first text field gets the focus when
the dialog opens. If a dialog starts with actions or a selection, the title gets
the focus. Thus, no button looks like a preselected button. The keyboard focus indicators
and the keyboard navigation continue to operate. The opener can change while
the dialog shows. Then, its stable `id` or its `data-focus` key lets the dialog
move the focus to the new opener.

The primitives accept text, not HTML templates. Labels, help and errors use
`textContent`. The `glyph` slot of the menus accepts only SVG that the code of
the product makes. Do not give HTML from users to it. Components return DOM
elements, and you can compose them in all hosts. Close dialogs and menus when
you remove the screen. When you remove a dynamic host, call the function that
`enhance` returns.

## Authored interaction metadata

Consumers can give a static `track` identifier to these items:

- controls (the last optional argument)
- `select` attributes
- menu items
- dialogs
- searchable pickers

`identifyControl(element, target, area?)` and `identifyArea(element, area)` also
add metadata to current controls and surfaces. Menus copy their explicit `area`,
or the area of their anchor, into their portal. Dialogs accept `area`, or they
get it from the opener. Give `anchor` or `area` to `menu.openAt`.

The `track` prefix of a dialog gives the names `.save` and `.cancel`. The prefix
of a picker gives the names `.search`, `.select`, `.secondary`, `.refresh` and
`.filter`. The metadata of an item, a secondary action, a refresh or a filter
can replace those names. The values must be product identifiers that an author
wrote. They must not be item IDs, translated text, field values or user
content.

The package contains only DOM metadata. The product that uses the package owns
the collection, the delivery and the coverage checks. The desktop coverage
includes hidden controls that are enabled. Native disabled controls and
`aria-disabled="true"` controls are not in the coverage. When these controls
become enabled, the same checks apply to them. The desktop check is
`node scripts/check-analytics.mjs`.

## Rails

The package includes `rails/prometeu_design_system.rb`. The Cloud imports an
explicit version, with the hashes of all the files:

```sh
bin/design-system ../prometeu/packages/design-system
bin/design-system --check ../prometeu/packages/design-system
```

`vendor/design-system/assets` contains CSS, SVG and `design-system.js` (a single
bundle without remote imports). `vendor/design-system/rails` contains the Ruby
adapter. It is not in the public path of the assets. The server and the build of
the Cloud do not use Node or the desktop checkout. To connect a different Rails
application, use this code:

```ruby
require_relative "../../vendor/design-system/rails/prometeu_design_system"

module ApplicationHelper
  include Prometeu::DesignSystem::Helpers
end
```

```erb
<%= ds_form_with model: @user, url: account_path,
      password_labels: { show: "Mostrar senha", hide: "Ocultar senha" } do |form| %>
  <%= form.field :name, label: "Nome", required: true, autocomplete: "name" %>
  <%= form.field :password, label: "Senha", type: :password, required: true %>
  <%= form.button "Salvar", variant: :pri %>
<% end %>
```

The adapter renders full fields, labels, messages and buttons. Do not copy its
markup into the views. `form.field` accepts `text`, `email`, `password`,
`textarea`, `number` and `date`. `form.checkbox` keeps the native Rails value for
a checkbox that is not checked.

The helpers `ds_button_to`, `ds_link`, `ds_card`, `ds_badge`, `ds_notice`,
`ds_disclosure`, `ds_menu`, `ds_tabs` and `ds_avatar` compose the other server
primitives. `ds_menu` accepts these items:

- `:sep` between groups
- items with `method:`, which it renders as a native form with CSRF
- `danger: true` for destructive actions
- `avatar:` on the button that opens the menu

`ds_avatar(image:, kind:, size:)` shows a photo or a logo. Without an image, it
shows the glyph of a person or an organization. `ds_tabs` renders tabs with a
line below them (`ui-tabs`). `ui-nav-link` is for sidebars.

Include `components.css` and the `design-system.js` bundle through the asset
pipeline. The bundle runs `enhance()` one time. The Cloud lets local scripts operate.
It uses a nonce on the script, and it does not let `unsafe-inline` or `eval`
operate.
The components do not read cookies, tokens or credentials, and they do not send
requests.

With JavaScript, the runtime adds these functions:

- menus
- a button that shows the password
- accessible validation
- optional confirmations (`confirm: {title:, message:, accept:, cancel:}`)
- a lock that prevents a second submit of the same data
- the recovery of the form after the person goes back in the browser

Submit buttons keep `name` and `value` in the POST. Without JavaScript, forms,
checkboxes, links and disclosures stay native. The server continues to validate
the authorization, CSRF and the mandatory confirmations. The optional dialogs do
not change this validation.

## Identity, gallery and maintenance

The source of the tokens is `tokens.css`. The controls do not use the
`style.css` of the application. The product sets the composition and the base
text. The default density is compact, with controls of 32px. The desktop and the
application screens of the Cloud use this density. `ui-comfortable` gives
controls of 44px for touch flows, for example the login and the authorization of
the Mac. `--fg-3` is for states that are not active and for decoration, not for important
text. The brand is `prometeu.svg`.

At the root, `npm run dev` opens Vite. Go to
`/packages/design-system/index.html`. The gallery uses the API of the package to
make all the components. These include menus, submenus, the selection, the
password and a dialog with a simulated failure. The gallery also operates with a
static server after the build of the package. `/design-system.html` examines
how the desktop screens use the package.

A change starts in the package, in the gallery and in the tests. Before you publish a new version, update the version number with these rules:

- A compatible repair increases the patch number.
- A new function increases the minor number.
- A removal or a contract that is not compatible increases the major number. During
  0.x, it increases the minor number.

Import the package into the Cloud again, and commit the manifest in the same
change. A rollback moves the package, the adapter and the assets back together.
No product downloads new versions from the network at runtime.

Checks: `e2e/design-system.spec.ts` and `e2e/actions.spec.ts` in the desktop
repository. `test/helpers/design_system_components_test.rb`,
`test/design_system_test.rb` and `test/browser/accounts.spec.js` in the Cloud.

## Feedback widget

`feedbackWidget({ labels, submit, capture?, error, publicIssue? })` builds a
portable composition with these parts:

- an optional link to a public issue
- categories
- a description
- an optional upload
- a thumbnail
- an asynchronous submit

`submit` gets `{ kind, description, image?: File }`. `capture` returns a `File`,
or `undefined` if the person cancels. `publicIssue` opens the URL and does not
copy data from the form. The host gives the texts after translation. The host controls the
transport and the capture. `trigger` gives access to the button, so that the host
can put it in its own navigation. `destroy()` removes the listeners, the popover
and the temporary URLs.

The widget uses the top layer and follows the active modal, so that it stays
interactive. It does not clear the form after an error. PNG, JPEG and WebP have
a limit of 5 MiB. The gallery shows a local submit. `e2e/feedback.spec.ts` tests
the integration with the desktop.

## Searchable collections

`searchablePicker(anchor, options)` opens a small panel in the shared menu
lifetime (`menu.isOpen`, `menu.close`, `menu.onClose`). If the anchor is in an
open dialog, the picker goes into that dialog. `options` supplies these values:

- the translated `label`, `searchPlaceholder` and `empty`
- `items` and `select(key)`
- an optional `status`
- `refresh: { label, run }`
- `additional: { label, checked, change }`
- `closed` and `search(query)`

With `search`, the caller owns the order of the results. The picker reports each query and
shows the items that the caller gives to `update`. It does not filter them
again. Thus, the picker does not compare the terms locally and remove fuzzy results
from the backend. Between a keystroke and the next `update`, the previous results stay on the screen but disabled, and the list is `aria-busy`. Thus, Enter or a click cannot
select an answer to an older query.

Items contain a stable `key`, a `label` and these optional fields: `detail`,
`searchText`, `group`, `checked`, `disabled` and
`secondary: { label, pressed, run }`. The secondary button does not change the
selection, and it shows `aria-pressed`. The caller owns its updated state and
its accessible action label.

The `update(items, status?)` function that the picker returns keeps the search
query, and the focused item or action, by key. If that item is not there, the
focus goes to the search field. The search ignores uppercase, lowercase and accents. Each term between spaces
must agree with the label, the detail or the searchText. The terms can be in a
different sequence. The search input gets the first focus.

These keys operate in the picker:

- The arrows move between the items.
- Enter selects an item.
- Home and End move between the items while an item has the focus.
- Tab goes to the secondary controls and the footer controls.
- Escape closes the picker before its parent dialog.

An interaction out of the picker, a blur or `close()` releases the shared menu
lifetime. Call `close()` before you remove the host. This component does not
load data or store preferences.

The standalone gallery shows 100 items, disabled items and selected items. It
also shows secondary actions, refresh, the additional option, an asynchronous
remote search and the picker in a dialog. `e2e/search-picker.spec.ts` tests the search with the
keyboard, accents, the selection with the pointer, the focus sequence and a
narrow dialog in Chromium and WebKit.
