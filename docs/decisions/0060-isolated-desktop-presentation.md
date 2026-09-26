# ADR 0060 — Isolated Desktop presentation

Date: 2026-09-25
Status: Accepted

## Context

The shared package already supplies executable controls, but the Desktop's
resource library assembled hub-owned DOM and inferred metadata from selectors.
Legacy CSS could override those controls, including a narrow layout that hid
a grid track without hiding its icon. Agents could demonstrate primitives but
could not compose this application screen independently of its integrations.

## Decision

Settings / Resources is the first feature with an explicit data-and-callback
presentation boundary. Hubs project typed resource items and own actions;
`src/settings-resources.ts` composes them into `src/components/resource-view.ts`. The
existing desktop gallery imports the same view with synthetic snapshots.

Rendering, focus, local search and layout belong to the view. Authorization,
installation, persistence, error handling and pending operations belong to the
existing hubs. A transitive import check prevents the view from reaching those
integrations. Shared Desktop compositions in `src/components/compositions.ts` supply headers, toolbars,
rows, overflow menus and list states to both Resources and Actions. Their CSS
owns common geometry; resource columns and action-card variants remain local.
An independent gallery composes these parts without either feature adapter.
The compositions have a stricter import boundary than the Resource view: only
UI/menu facades, their own CSS and the shared package.
The resource CSS owns its layout without legacy row classes.
Legacy button rules exclude shared buttons; consumers retain shared classes.

Keep the current DOM/TypeScript implementation, native platform font, warm
palette and shared tokens. No new framework, dependencies, schema renderer or
Cloud changes are required. The Desktop component surface lives in `src/components/`, grouped by cohesive
families. Package primitives use compatibility re-exports, not copied code.
ChatView and Workspace Changes consume the same block, composer, request, file,
group, commit and diff components shown in the gallery. Controllers retain
transport, business effects and persistence. Diff caches and observers belong
to individual reader instances.

A JSON catalog names source, exports, states and real consumers. The existing
Vite gallery supplies searchable navigation, direct story URLs and standalone
canvases for browser agents. This provides component exploration without adding
Storybook, a framework adapter or a second renderer. Catalog tests verify
production reachability; the gallery demonstrates independent readers.
This is incremental adoption of a discoverable component surface. ADR 0017 remains in force for the
shared component package.

## Alternatives and consequences

Keeping hub DOM behind an adapter would preserve implicit selector contracts
and require application setup to demonstrate the screen. A general declarative
screen engine or framework migration would add a second abstraction before this
single feature established a need. Typed data and existing callbacks provide
the smallest independently runnable boundary.

The extraction changes multiple hub projection functions, so compatibility is
proved through existing desktop catalog/MCP journeys and a DOM-free adapter
test. The view supports loading and error snapshots without redefining hub
loading policy. Actions adopts the common components while retaining its editor and save
operations. Remaining Settings pages retain their current integration.

No persisted or wire format changes. Reverting the presentation and adapters
together needs no data migration. The shared package adds only `iconNames`, a read-only catalog export; existing
APIs and the Cloud integration remain compatible.

## Evidence

- [Presentation contract](../contracts/desktop-presentation.md).
- [Composition recipe](../architecture/desktop-composition.md).
- [Component catalog](../../src/components/README.md).
- [View](../../src/components/resource-view.ts) and [gallery](../../src/resources/gallery.ts).
- [Adapter proof](../../src/resources/adapters.test.ts).
- [Settings and layout checks](../../e2e/settings.spec.ts).
- [Dependency checks](../../scripts/architecture-dependencies.test.mjs).
