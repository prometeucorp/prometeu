---
type: Decision
title: "ADR 0060 — Isolated Desktop presentation"
description: "An isolated Desktop presentation with one view that production and the gallery share."
status: stable
---

# ADR 0060 — Isolated Desktop presentation

Date: 2026-09-25

## Context

The shared package supplied executable controls at that time. But the resource
library of the Desktop assembled DOM that the hubs owned. It also guessed
metadata from selectors. Legacy CSS was able to replace the styles of those
controls. For example, a narrow layout removed a grid column but kept its
icon on the screen. Agents were able to show primitives. But they were not able to
compose this application screen independently of its integrations.

## Decision

Settings / Resources is the first feature with an explicit presentation
boundary that receives data and callbacks. The hubs make typed resource items,
and they own the actions. `src/settings-resources.ts` puts these items into
`src/components/resource-view.ts`. The current desktop gallery imports the same
view with synthetic snapshots.

The view controls the rendering, the focus, the local search and the layout.
The current hubs control the authorization, the installation, the saved data,
the error processing and the operations that wait. A transitive import check
prevents the view from the use of those integrations.

The shared Desktop compositions in `src/components/compositions.ts` supply
headers, toolbars, rows, overflow menus and list states to Resources and to
Actions. Their CSS controls the shared geometry. The resource columns and the
action-card variants stay local. A different gallery composes these parts
without the adapter of a feature. The compositions have a stricter import
boundary than the Resource view. They can import only the UI and menu facades,
their own CSS and the shared package. The resource CSS controls its layout
without legacy row classes. Legacy button rules do not apply to shared buttons.
Consumers keep the shared classes.

Keep the current DOM and TypeScript implementation, the native platform font,
the warm palette and the shared tokens. A new framework, new dependencies, a
schema renderer and Cloud changes are not necessary. The Desktop components are
in `src/components/`, in groups of related families. Package primitives use
compatibility re-exports. They do not copy code. ChatView and Workspace Changes
use the same block, composer, request, file, group, commit and diff components
that the gallery shows. Controllers keep the transport, the business effects
and the saved data. Each reader instance owns its diff caches and observers.

A JSON catalog gives the source, the exports, the states and the production
consumers of each component. The current Vite gallery gives searchable navigation, a URL for
each story and standalone canvases for browser agents. Thus agents can examine
components without Storybook, a framework adapter or a second renderer. Catalog
tests make sure that production uses each component. The gallery shows that
each reader operates independently. Prometeu uses a component surface that agents
can find, one step at a time. ADR 0017 stays in effect for the shared component
package.

## Alternatives and consequences

If Prometeu kept the hub DOM behind an adapter, the implicit selector contracts
stay. Also, application setup is necessary before an agent can show the screen. A
general declarative screen engine or a framework migration adds a second
abstraction before this one feature shows that it is necessary. Typed data and
the current callbacks give the smallest boundary that can run independently.

This change edits many hub projection functions. Thus the current desktop
catalog and MCP journeys and an adapter test without DOM prove compatibility.
The view shows load and error snapshots, but it does not change the load policy
of the hubs. Actions uses the shared components, and it keeps its editor and
save operations. The other Settings pages keep their current integration.

No saved format or wire format changes. If Prometeu removes the presentation
and the adapters together, a data migration is not necessary. The shared
package adds only `iconNames`, a read-only catalog export. The current APIs
and the Cloud integration stay compatible.

## Evidence

- [Presentation contract](../contracts/desktop-presentation.md).
- [Composition recipe](../architecture/desktop-composition.md).
- [Component catalog](../../src/components/README.md).
- [View](../../src/components/resource-view.ts) and [gallery](../../src/resources/gallery.ts).
- [Adapter proof](../../src/resources/adapters.test.ts).
- [Settings and layout checks](../../e2e/settings.spec.ts).
- [Dependency checks](../../scripts/architecture-dependencies.test.mjs).
