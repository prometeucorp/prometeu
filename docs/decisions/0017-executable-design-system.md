---
type: Decision
title: "ADR 0017 — Executable Design System components"
description: "The Design System package supplies components that operate, not only styles."
status: stable
---

# ADR 0017 — Executable Design System components

Date: 2026-09-06

## Context

The shared CSS and SVG gave one appearance to the products. But each product
kept its own rendering and interaction. The Design System must supply complete
components. These components include state, events, focus, keyboard, validation
and lifecycle.

## Decision

`packages/design-system` has the package name `@prometeu/design-system`. It owns
these items:

- The tokens and the SVG brand.
- The DOM primitives, the menus and the generic icons.
- The forms and the styles that the interactions must have.

On the desktop, `src/ui.ts` and `src/menu.ts` stay as re-exports for
compatibility. No component of the package imports application modules or
provider rules.

The desktop imports the source directly. `src/ui-tokens.css` keeps only the
geometry of the desktop. Shared styles use opt-in classes and keep the token
names. Each consumer keeps its product layout, domain rules and translations.
Components receive translated text and callbacks. `ui-comfortable` gives 44px
controls for touch flows.

Dialogs use `dialog.showModal()` with an explicit Tab cycle. They put the focus
back when they close, and they put menus in the same layer. New controls use
these primitives again. Current screens use them when a person changes the
screen. The standalone gallery and the browser tests show the focus, keyboard,
validation and failure states.

The package supplies ESM modules, TypeScript types and a browser bundle. The
TypeScript and Vite in the project make the artifact. A framework and runtime
JavaScript dependencies are not necessary for consumers.

For Rails, the same package supplies a FormBuilder and helpers. They render
complete fields, buttons and containers. Views use these APIs and do not write
the markup of the controls again. The shared runtime connects menus, passwords,
optional confirmations, validation and protection against a second send of the
same form. Forms keep the usual Rails form behavior, the values of the button
that sends the form, and CSRF. Without JavaScript, the necessary controls stay
native. Authorization does not use the runtime at any time.

The Cloud keeps a copy of the assets from the build and the Ruby adapter, with
hashes and a version. Only `vendor/design-system/assets` goes into the public
pipeline. Its CSP lets local scripts and a nonce operate. It does not let free
inline scripts, `eval` or external sources operate. Ruby code, the database,
tokens and credentials do not go into the bundle.

## Alternatives and consequences

With React, the team must replace the presentation of the desktop and of Rails.
With Web Components, the team must write the current primitives again and adapt
fields to the native send operation of the form. The selected option moves the
current DOM code into the package and adds a Rails adapter. Thus the two
products stay standard and keep the tested behaviors.

The package has two renderers, DOM and Action View. Each renderer agrees with
its environment. Classes and accessibility have a shared contract and tests in
Chromium and WebKit. Browser interactions have one JavaScript implementation.
Product layout, business rules and translations are not in the package.

The team must build the package to supply new versions. Node is not necessary
for the Cloud to build or operate the service. A rollback changes the runtime,
styles, adapter and manifest back together. The API, the IPC, the stored data
and the content that goes to the service do not change.

## Evidence

- [API and integration](../../packages/design-system/README.md).
- [Executable components](../../packages/design-system/src/index.ts).
- [Rails adapter](../../packages/design-system/rails/prometeu_design_system.rb).
- [Standalone gallery](../../packages/design-system/index.html).
- [Behavior tests](../../e2e/design-system.spec.ts).
- [Screen compatibility](../../e2e/actions.spec.ts).
