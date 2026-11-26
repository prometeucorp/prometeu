---
type: Decision
title: "ADR 0020 — Catalog in the SaaS and explicit sharing on the desktop"
description: "The catalog is in the SaaS, and the desktop shares an item only when a person tells it to."
status: stable
---

# ADR 0020 — Catalog in the SaaS and explicit sharing on the desktop

Date: 2026-09-07

## Context

The SaaS stored the previous catalog, but a person was able to write items only
on the desktop. When a person connected an account, the desktop published the
local hubs. A person was not able to keep a portable MCP or plugin private.

A person must be able to add MCPs, plugins and skills in the browser and
receive them on the desktops. The desktop must also use that set together with
optional local definitions. Organization catalogs use the same document model
under [ADR 0021](0021-cloud-organizations.md).

## Options considered

1. Add only forms to the previous document. This option gives no local privacy.
2. Make organizations, teams, permissions and offline synchronization with merge
   immediately.
3. Keep the account as the owner of the personal catalog. Let persons write
   items in the browser, and add explicit links between remote definitions and
   local records.

## Decision

Use option 3. An item that a person makes on the desktop is private by
default. To share an item is an explicit action. A change to a linked definition
publishes it to the account. A local copy gets a different ID and does not
change the shared item.

An MCP connection check and OAuth are local operations on an installed
definition. They do not use catalog changes or revision checks. They do not
publish an item or make a local copy at any time. The desktop shows these
actions directly in the hub and keeps the credentials on each Mac. After a
connection change in the editor, the person must save it before authentication.

The SaaS uses standard Rails, Design System forms, cookies and CSRF. The desktop
keeps the Bearer. The current document with revisions continues to be the
storage unit. A web operation changes only the selected item and keeps the other
collections. When two changes do not agree, the SaaS shows this and keeps the
drafts. The SaaS does not automatically send a change from a previous revision
again on top of a new revision.

Skills are text definitions. The desktop writes them as packages so that the
current plugin adapters and selectors can use them. Plugin code continues to
come from a remote source through an explicit installation. There is no folder
upload.

A private item without a link can be equivalent to a personal definition. Then
it satisfies that definition without a link, and its changes stay private. When
a person shares such a plugin or skill, the desktop links it to the current
definition. It does not publish a duplicate. The desktop shows each resource one
time, with each catalog that supplies it as a source. A definition with the same
name and different content gets a flag on the installed row. The desktop does
not show it again.

When the desktop receives definitions, it does not enable tools in
conversations. A remote delete operation does not delete files or credentials
from the Macs. A link map keeps the private names. The map connects the ID of
the account to the ID in the local hub.

## Consequences

- Management in the browser and local selection operate without a new service,
  dependency or database migration.
- The full JSON and its limit of 256 KB stay. Concurrent changes on different
  items can also make an explicit retry necessary.
- Previous clients keep new skills because they do not write the collection.
  Their previous behavior of automatic publication changes only when the person
  updates the desktop.
- A plugin copy shares the files of the clone, but not its online definition.
  Each copied skill has its own written content.
- Legacy Actions continue to go to all desktops as before. They do not get a web
  editor at this stage.
- Personal catalogs and organization catalogs have different owners and
  revisions. Installation directly from an organization follows
  [ADR 0039](0039-organization-catalog-on-desktop.md). It makes a local record.
  Changes that the organization makes after that do not go to that record.
- A rollback puts back the previous code and keeps the JSON and the local files.
  A previous desktop version ignores skills. Do not use a rollback to remove new
  data.

## Evidence

The [catalog contract](../contracts/cloud-catalog.md) gives the formats, the
compatibility, the behavior that a person can see, and the tests in the two
repositories.
