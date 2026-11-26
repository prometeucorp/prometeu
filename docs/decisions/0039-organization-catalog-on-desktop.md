---
type: Decision
title: "ADR 0039 — Organization catalogs available on the desktop"
description: "The desktop shows the catalogs of the organizations of a person and installs an item only when the person selects it."
status: stable
---

# ADR 0039 — Organization catalogs available on the desktop

Date: 2026-09-11

This ADR specifies how the desktop finds and installs items from the
organization catalogs of [ADR 0021](0021-cloud-organizations.md).

## Context

The catalog of the organization contains portable definitions at this time. But
the person had to copy them in the browser first. Thus, the person did not
find them in the area where the person installs tools.

## Decision

The desktop shows the plugins, MCPs and skills of each organization with an
accepted membership. They are adjacent to the current hubs, with the name of the
organization and `Install here`. The organization that is active in the relay
does not filter those definitions. The person must select the installation. The
installation makes a local record that does not change with the catalog. It does not write to the
personal catalog or to the catalog of the organization, and it does not enable
tools in conversations.

The desktop reads each catalog with a Bearer on the path of that catalog. It
obeys the limit of 256 KB for each document and the authorization through the
current membership. The current refresh updates the availability. Before the
installation, the desktop examines the access and the shown definition again. If
they changed, the person must review the updated list.

The cache keeps the installed local IDs for each organization, type and item:

- If a different item has the name, the item gets a different ID through the current
  collision rule.
- Updates from the organization do not replace installed code, configuration or
  credentials.
- A revocation removes the availability at the next refresh. The installations
  stay.
- An equivalent local plugin, MCP or skill makes the item available. The desktop
  does not suggest or make a second installation.
- When the desktop compares items, it ignores the values of local MCP credentials, but it compares
  the executable configuration.
- For plugin sources on GitHub, the desktop compares the owner and the
  repository. As GitHub does, it ignores the difference between uppercase and
  lowercase letters. For other Git hosts, the desktop compares them accurately.

## Alternatives and consequences

A person can continue to copy items through the browser, but it is not
necessary. If we put the organizations into the personal document, the
authorization, the revisions and the ownership are not clear. A continuous
synchronization of installations must resolve local edits and changed
sources. This change is only for the discovery and the installation from the desktop.

There is no server migration. Publish the Cloud first, and then the desktop. A
Cloud from a previous version answers 404, and the personal catalog continues to operate. Clients from a
previous version ignore the added field of the cache. Installed files stay local.

## Evidence

- `src-tauri/src/catalog.rs`: collisions, credentials, local installation and a
  cache from a previous version.
- The optional form for discovery and installation has no browser scenario. The
  tests in `catalog.rs` continue to cover the native installation rules.
- Cloud, `test/integration/organizations_test.rb`: Bearer, membership,
  revocation and isolated organizations.
