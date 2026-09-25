# ADR 0039 — Organization catalogs available on the desktop

Date: 2026-09-11
Status: Accepted

Defines desktop discovery and installation for the organization catalogs in
[ADR 0021](0021-cloud-organizations.md).

## Context

The organization's catalog already contains portable definitions, but requiring
a copy in the browser prevents its discovery in the place where the person
installs tools.

## Decision

The desktop lists plugins, MCPs and skills of every organization with an
accepted membership, next to the existing hubs, with the organization's name and
`Install here`. The organization active in the relay does not filter those
definitions. Installation is explicit and creates an independent local record,
without writing to the personal or institutional catalog and without enabling
tools in conversations.

Each catalog is read with a Bearer on its own route, respecting the 256 KB limit
per document and authorization through the current membership. The existing
refresh updates availability. Before installing, the desktop checks the access
and the displayed definition again; changes require reviewing the updated list.

The cache keeps the installed local IDs per organization, type and item. Taken
names get another ID, using the existing collision rule. Organization updates do
not replace installed code, configuration or credentials. Revocation removes the
availability on the next refresh, preserving installations. An equivalent local
plugin, MCP or skill satisfies availability; the desktop neither suggests nor
creates a duplicate installation. Comparison excludes local MCP credential
values but preserves executable configuration. Plugin sources on GitHub compare
owner and repository case-insensitively, as GitHub resolves them; other Git
hosts compare exactly.

## Alternatives and consequences

Copying through the browser is still available, but is no longer a requirement.
Merging organizations into the personal document would confuse authorization,
revisions and ownership. Synchronizing installations continuously would require
resolving local edits and changed sources; this change is limited to discovery
and direct installation.

There is no server migration. Publish the Cloud first and then the desktop. An
old Cloud answers 404 and keeps the personal catalog working. Old clients ignore
the cache's additive field; installed files stay local.

## Evidence

- `src-tauri/src/catalog.rs`: collisions, credentials, local installation and an
  old cache.
- The optional discovery and installation form has no dedicated browser
  scenario; native installation rules remain covered in `catalog.rs`.
- Cloud, `test/integration/organizations_test.rb`: Bearer, membership,
  revocation and isolation.
