---
type: Contract
title: "Personal catalog and local items"
description: "Plugins, MCP servers and skills in the SaaS, with private items and explicit sharing on the desktop."
status: stable
---

# Personal catalog and local items

The app obeys this contract.
[ADR 0020](../decisions/0020-personal-catalog-and-local-items.md) and
[ADR 0039](../decisions/0039-organization-catalog-on-desktop.md) record the
decisions.

## Authoring and availability

The browser gives `/catalog`. There, a person can make, edit and delete the MCP
servers, plugins, skills and Git projects of the authenticated account. Each
desktop merges those definitions with its own items. An item that a person makes
or installs on the desktop is local by default, also when there is an account.
A connection does not publish all of the hub.

`Share in the cloud` publishes a definition and makes a link. Subsequent edits
update the account. `Create local copy` makes a different definition without a
link, and keeps the shared definition. A plugin copy refers to the same
installed files. If a person removes one of those definitions, the app does not
delete the shared clone. To change the files independently, add a different
folder to the registry. Copied skills have their own package and content.

When the personal catalog has an equivalent plugin or skill at that time, the
action becomes `Link to personal catalog`. It links the private item to that
definition and does not publish a duplicate. The app refuses this action when a
different installed copy holds the link. MCP servers are not included, because
personal MCP servers always install their own copy.

The Resources list shows one row for each resource. Its source column shows
badges:

- `This Mac` (`This computer` on other systems than macOS) when there is a local
  item.
- `Personal`, with `⇄` when the item is linked.
- Each organization that gives the same definition or an equivalent definition.

A definition with the name of an installed item but with a different content
does not get its own row. The installed row shows `<source> ≠` and gives
`Install from <source>`. This action installs the definition under a different
local ID. The app puts the definitions that are not on this Mac into groups by
name, without a difference between uppercase and lowercase. Each group is one
row. The menu of that row gives `Install here`, or `Install from <source>` when
more than one catalog gives the definition.

Received MCP servers go into the hub. The app does not connect or enable them
automatically. Installed MCP rows give **Test connection**. For remote servers,
they also give **Authenticate** or **Sign out** on this Mac. These actions use
the current MCP IPC. They do not save the definition, make a copy or change the
catalog revision. The authentication opens the local browser. Credentials stay
in the private MCP store of the Mac. Login and logout examine the connection
again.

The row shows one of these states: not examined, authentication necessary,
connected or error. A stored token alone does not prove the connection. A stored
login gives the sign-out action. If a check explicitly tells that
authentication is necessary, the row gives the authentication action again. The
results are temporary. The app discards them when the configuration changes. But
operations that did not end keep the actions blocked until they end, also if a
catalog refresh replaces the definition.

The editor can also authenticate current definitions without a save. A person
must explicitly save changes to the connection first. Received plugins and skills
show as available, with `Install here`. An equivalent private item without a
link satisfies a personal definition. It counts as installed, and the app does
not make a link. Thus, its subsequent edits stay private. The selection for
each workspace and conversation continues to set what the providers receive.

The desktop looks for updates in these conditions:

- At login.
- When the window gets the focus again.
- At intervals of 60 seconds while a person can see the window.
- When a person clicks **Refresh account**.

There is no WebSocket for the catalog. After a change to the source of a plugin, the
`Use new source` action is necessary. A metadata synchronization does not automatically
replace a clone with new code. The desktop editor changes the description of
shared plugins. A person edits the source in the SaaS. Thus, the definition
stays different from the local installation. An update of a repository stays an
explicit action on the desktop.

A deletion through the browser removes the shared definition. Desktops keep the
installed records and files as local items. When a person deletes a shared MCP
server or plugin from the desktop, the person must give approval for the deletion
in the cloud. When a person removes a skill from the Mac, the app only uninstalls its
record. The skill goes back to the list of available items. A sign-out keeps the
hubs and forgets the links.

## Document and HTTP

An account has a maximum of one JSON document of 256 KB:

```ts
type Doc = {
  plugins: { id: string; source: string; note: string }[];
  mcp: { id: string; config: object; note: string }[];
  skills: { id: string; description: string; content: string }[];
  projects: { id: string; source: string; note: string }[];
  actions: Catalog | null;
};
```

`skills` and `projects` add to the format. Previous documents do not include these
collections. When a previous desktop sends a PUT without the key of a collection,
the API keeps the current collection. When a client sends `skills: []` or
`projects: []`, the API removes the definitions of that collection. Previous
Actions keep the current contract. They continue to have no editor in the SaaS
and no control to share each item.

| Route | Authentication | Body and result |
| --- | --- | --- |
| `GET /api/catalog` | desktop Bearer | `{ catalog: Doc or null, revision: number or null }` |
| `PUT /api/catalog` | desktop Bearer | `{ catalog: Doc, revision }`. Returns the document and the revision |
| `/catalog` and `/catalog/:kind` | browser cookie and CSRF | item CRUD in `plugins`, `mcp`, `skills` or `projects` |

`revision: null` tells that this is the first write. A different revision
returns 409 and does not write. The browser keeps the draft for a review. The
desktop updates its cache and reports the conflict. It does not send the change
again silently. Desktop editors keep the revision from the time when they
opened. A background refresh does not let an editor replace a different
edit. A network failure stops only shared edits. Private edits stay available
offline.

The server validates the types, the unique names, the remote sources and the
limits before it stores the document. Skill IDs follow
`[a-z0-9][a-z0-9-]{0,55}`. The description can have a maximum of 2000
characters, and the content a maximum of 65536 bytes. The content is a Markdown
body without front matter. The desktop makes the name and the description with
YAML strings that encode special characters.

Plugins store a Git address or a `.zip` URL. They do not store an upload of a
local folder. MCP servers store the portable command or URL and the arguments.
The `env` and `headers` values stay empty in the cloud. The server refuses
values that are not empty. A person enters the credentials on each Mac, and the
app keeps them during updates. The person publishes the arguments and the free
text as content.

> **CAUTION:** Do not put secrets in the arguments or in the free text. Other
> persons can read this content.

The service does not run commands, does not install plugins and does not open
the registered sources.

## Identity and local persistence

`<root>/catalog.json` stores `{ revision, doc, links }`. `links` maps
`<type>:<id in the account>` to a local ID. If a private item has the name, the
remote definition gets a different ID (`cloud-<name>-<n>`), and the private item
stays. A previous cache without `links` migrates the historical links by name. A
change of the account or the origin forgets the previous cache and keeps the
local items.

`<root>/skills.json` stores the installed definitions. The app writes each skill
in `<root>/skills-packages/<id>/`, with Claude and Codex manifests and
`skills/<id>/SKILL.md`. The plugin hub contains `skill-<id>` and uses the
current selection and adapters. These packages show on the Skills page and in
the selectors. The app does not show the registration again on the Plugins
page.

Each catalog is the catalog of the person (`user_id`) or of the organization
(`organization_id`). Only one of these owners is in the database. The desktop
updates its copy of the personal catalog. It also shows the plugins, MCP servers and
skills of each organization where the person has an accepted membership. Each
item of an organization shows the name of the organization and
`Install here`. A copy into the personal account is not necessary. The
installation makes a local record that is not connected to the catalog. The app does not enable or
publish it automatically. An equivalent local definition counts as installed.
Thus, the desktop does not show a duplicate installation.

The app uses these rules to find equivalent items:

- Plugins agree when the ID and the source agree, without a difference between
  uppercase and lowercase. GitHub owners and repositories also agree without a
  difference between uppercase and lowercase, and the app ignores a `.git`
  suffix.
- MCP servers agree when the ID and the portable configuration agree, after the
  app removes the credentials.
- Skills agree when the full definition agrees.

The personal catalog uses the same rules. An installation from an expired state
links that local record and does not make a new record. In the Cloud, the owner
and the administrators do the CRUD operations. A copy of definitions between
catalogs stays optional. See [organizations](cloud-organizations.md) and
[ADR 0039](../decisions/0039-organization-catalog-on-desktop.md).

`GET /api/organizations/:id/catalog` accepts Bearer. It returns the same
`{ catalog, revision }` envelope as the personal catalog. The current
membership authorizes the request. The desktop reads the list from
`GET /api/organizations` and fetches each document independently. Thus, the
limit for each response stays. A 404 makes that catalog not available, and the
app stays compatible with a previous Cloud. After network failures, the app keeps
the cache.

`catalog.json` adds `organizations: [{ id, name, revision, doc, links }]`. Previous
caches do not have this field. Those `links` identify local installations by
type and ID. They are not part of personal PUTs. Collisions use
different local IDs, also between organizations and personal items that are not
installed. A refresh updates the available definitions. Installations and
credentials do not change. After a sign-out or the end of a membership, the
installed records and files stay.

## IPC

| Command | Arguments | Return |
| --- | --- | --- |
| `catalog_state` | none | connected, revision, plugins, mcp, skills and shared |
| `catalog_install_project` | organization (nullable), id, revision, directory, existing | registered Project. Before the clone or the link, it examines the current membership, revision and source again |
| `catalog_share` | kind, local id | empty. Publishes and links, or links to an equivalent personal plugin or skill |
| `catalog_copy` | kind, local id, newId | empty. Makes a private definition |
| `catalog_install_plugin` | account id | empty. Installs the selected source |
| `catalog_install_skill` | account id | empty. Writes the skill package |
| `catalog_install_organization_item` | organization, kind, id, revision | empty. Examines the membership and the revision that the app showed, and installs the item locally |
| `skill_hub` | none | installed Skill[] |
| `skill_save` | skill, revision | Skill[]. Publishes only if the skill is linked |
| `skill_remove` | local id | Skill[]. Removes the skill only from this Mac |

The catalog refresh operates through `cloud_status` with `refresh: true`. This
includes **Refresh account** and the current refresh at focus and at intervals.
[ADR 0043](../decisions/0043-retire-unused-ipc.md) retired the
`catalog_refresh` IPC, which had no caller.

`plugin_save` and `mcp_save` also receive `revision` when a person edits a
shared item. A revision is not necessary for new private items. The `catalog` event
updates the hubs and the markers of the interface. `plugins` and `skills` in the
state include `local_id` and `installed`. Plugins also include
`source_changed`. `local_id` identifies the linked installation. When there is
no linked installation, it identifies the equivalent private item that
satisfies the definition. `shared` maps `<type>:<local id>` to the ID in the
account.

## Git projects

A person writes projects in the personal browser catalog or in the browser
catalog of an organization. Members can read the projects of an organization.
Owners and administrators can edit them. The Projects settings page of the
desktop and the Add project action in the sidebar open a selector. The selector
shows the personal definitions and the definitions of the organizations. Select
more than one project and one parent folder, one time. The app adds each
correct clone to the registry immediately. Projects with a failure stay selected, each with its
error.

Each row with a failure shows the cause and the Git diagnostic (a maximum of
4096 characters). For an authentication failure, the row tells the person how to
examine the Git account, the SSH key or the token of the Mac. A missing
repository can mean an incorrect address or no access. The UI keeps that
difference. For other failures, the row shows the diagnostic and does not tell
that the cause is an access problem. The app removes URL credentials, query
strings and authorization headers from the data. Diagnostics stay local, and the
app does not send them to the Cloud. A new try does not do the correct rows
again.

The registration of a local folder and **Create new project** stay available
without an account. **Create new project** initializes a new empty Git
repository and uses the current registration path. It does not publish a
catalog definition. See the
[`create_project` contract](ipc.md#executable-serialization-examples).

Project IDs agree with `[A-Za-z0-9][A-Za-z0-9_.-]{0,127}` and become the names
of clone folders. Sources accept these formats:

- HTTPS without user information.
- SSH with the `git` user, and `git@host:path`.
- GitHub `owner/repo` or `github.com/owner/repo`.

Sources reject credentials, query strings, fragments, local paths, traversal,
percent-encoded text and other Git transports. `note` can have 2000 bytes, and the
source can have 4096 bytes. The shared [fixtures](../../fixtures/cloud-api.json)
contain accepted and rejected sources and previous documents. Rails and the
production Rust parser use them.

`catalog_state.projects` contains each portable definition, and also the
nullable fields `organization`, `organization_name`, `revision` and
`local_path`. The app shows the path while its folder and its board registration
are there. It also shows the path when a registered project has the same
normalized Git origin. The installation selector does not show the projects that
are on the Mac. Older versions of the desktop do not send this field, because it
adds to the format.

`catalog_install_project` fetches the current document of the account or the
organization. It rejects a changed revision or definition before the clone or
the link. A conflict refreshes the cache. To examine the current definitions,
open the selector again. With `existing: false`, `directory` is the selected
parent folder. With `existing: true`, it is the root of the current repository.
The app can add a root and origin that agree to the registry again. It does
not discard changes and does not make a new clone. To compare origins, the app
normalizes the
GitHub short form, the scp syntax and an optional `.git` suffix. To change
between HTTPS and SSH, the catalog source must agree with the local origin. If a
registered project has that origin, the installation records its path and
returns it. It does not clone and does not show the folder selector again.

Origin queries use a snapshot of the registered projects. They do not hold the
board mutex or the catalog mutex. Before the installation links a project, it
examines again that the project continues to be registered. Catalog writes stay
in sequence. When a person adds or removes a local project, the app uses the same catalog
guard.
They get the guard before the board lock. Thus, they cannot change the
registration set during the Cloud request or the checkout. These commands
operate on worker threads while they wait. Board reads and workspace edits that
are not related stay available. IPC payloads do not change.

The app rejects a folder that is not related, a subfolder of a repository or a
different origin. Git clones into a temporary folder adjacent to the destination.
Then it moves the full clone into the reserved destination. After a failure, the
app removes only that temporary clone and the empty reservation. The current
files stay. A clone that is not completed does not become a registered project
after a new try.

The registration uses the current Project format of the board. Personal caches
and organization caches store `projects:<catalog id>` with the local path in
`links`. These installation links do not publish the state of the file system
and do not change catalog revisions. After the removal of a definition, after a
sign-out and after the revocation of a membership, the local files and the
registered projects stay. Updates do not pull or replace an installed clone.

The clone uses local Git credentials. It disables the password prompts of the
terminal and the Git hooks. It accepts only the HTTPS and SSH transports. It
does not run the setup of the project, install dependencies or configure skills,
MCP servers or plugins. Native credential helpers and SSH authentication operate only on a
configured Mac. Browser tests simulate the clone. Rust tests run Git
against a local upload-pack fixture.

The `organization_items` field adds to the format. It contains `{ organization,
organization_name, revision, kind, id, description, installed, local_id }`.
`local_id` is the installed local item that satisfies the definition, or null.
The frontend accepts a state without this field. The installation queries the
catalog of the organization again. If the document changed, it updates the cache
and returns a conflict before the installation. `installed` also includes
equivalent local definitions, also before there is an installation link for the
organization. MCP servers of an organization go into the hub only after
`Install here`, with empty credentials.

## Evidence

- [`fixtures/cloud-api.json`](../../fixtures/cloud-api.json): empty, current and
  earlier documents. The production Rust parser and the Rails endpoints use
  them. Paired tests examine revision conflicts, rejected credentials and the
  skills that stay when older clients do not send them. See the
  [shared fixture workflow](../operations/development.md#shared-cloud-api-fixtures).
- `src-tauri/src/catalog.rs`: link migration, name collisions, private items
  and credentials that stay, and rejected incorrect documents. It also
  examines personal equivalence, how the app compares GitHub sources, and the link
  at a share.
- `src/catalog.test.ts`: the sources in the resource list, definitions with the
  same name and a different content, and the groups of missing definitions.
- `src-tauri/crates/tools/src/skills.rs`: shared validation, isolated folders,
  front matter, the manifests of the two providers and content updates.
- `e2e/cloud.spec.ts`: explicit sharing, a private copy, offline edits and
  revision conflicts through the mock. `catalog.rs` examines the rules for the
  installation from an organization. The optional installation form has no
  E2E test of its own.
- `e2e/mcp.spec.ts`: local connection checks, a new login try and logout for
  shared MCP servers while the Cloud is not available. It also examines the
  guards for actions that did not end, and that catalog revisions do not change the
  authentication. Browser tests simulate OAuth. For the consent
  with an actual server, a manual validation continues to be necessary. This feature does not
  change stored formats or IPC formats.
- `prometeu-cloud/test/integration/catalog_test.rb` and
  `catalog_browser_test.rb`: authentication, isolated data for each account,
  browser and API CRUD, conflicts and compatibility.
- `prometeu-cloud/test/browser/catalog.spec.js`: actual Rails forms, CSRF, the
  use of the desktop API and the layout in Chromium and WebKit.
