# Personal catalog and local items

Status: implemented; decisions in
[ADR 0020](../decisions/0020-personal-catalog-and-local-items.md) and
[ADR 0039](../decisions/0039-organization-catalog-on-desktop.md).

## Authoring and availability

The browser offers `/catalog`, with creation, editing and deletion of the
authenticated account's MCPs, plugins, skills and Git projects. Each desktop merges those
definitions with its own items. Creating or installing an item on the desktop is
local by default, even when there is an account. Connecting never publishes the
whole hub.

`Share in the cloud` publishes a definition and establishes a link: later edits
update the account. `Create local copy` creates another definition, without a
link, and keeps the shared one. A plugin copy references the same installed
files; removing either of those definitions does not delete the shared clone. To
modify the files independently, register another directory. Copied skills have
their own package and content.

When the personal catalog already has an equivalent plugin or skill, the action
becomes `Link to personal catalog`: it links the private item to that
definition instead of publishing a duplicate. It is refused when another
installed copy still holds the link. MCPs are excluded because personal MCPs
always install their own copy.

The Resources list shows one row per resource. Its source column lists badges:
`This Mac` when a local item exists, `Personal` (with `⇄` when linked) and each
organization that offers the same or an equivalent definition. A definition with
the name of an installed item but a different definition does not get its own
row; the installed row shows `<source> ≠` and offers `Install from <source>`,
which installs it under a distinct local ID. Definitions missing from this Mac
are grouped by name, case-insensitively, into one row whose menu offers
`Install here`, or `Install from <source>` when several catalogs offer it.

Received MCPs enter the hub, without automatic connection or activation.
Installed MCP rows offer **Test connection** and, for remote servers,
**Authenticate** or **Sign out** on this Mac. These actions use the existing
MCP IPC without saving the definition, creating a copy or changing the catalog
revision. Authentication opens the local browser; credentials remain in the
Mac's private MCP store. Login and logout recheck the connection. The row shows
an unchecked, authentication-required, connected or error state; a stored token
alone does not prove connectivity. A stored login offers sign-out; authentication
is offered again if a check explicitly requires it. Results are ephemeral and
discarded when the configuration changes, but pending operations keep actions
blocked until they settle even if a catalog refresh replaces the definition.
Existing definitions also authenticate from the
editor without saving; connection edits must be explicitly saved first.
Received plugins and skills appear as available, with `Install here`. An
unlinked equivalent private item satisfies a personal definition: it counts as
installed without creating a link, so its later edits stay private. The
per-workspace/per-conversation selection still determines what the providers
receive. The desktop checks for updates on login, when the window regains focus,
every 60 seconds while visible and on **Refresh account**. There is no catalog
WebSocket. Changes to a plugin's source require `Use new source`; syncing
metadata does not automatically replace a clone with new code. The desktop
editor changes the description of shared plugins; the source is edited in the
SaaS, preserving the difference between definition and local installation.
Updating a repository is still an explicit action on the desktop.

Deleting through the browser removes the shared definition. Desktops keep the
records and files already installed as local ones. Deleting a shared MCP or
plugin from the desktop requires confirming the deletion in the cloud. Removing
a skill from the Mac only uninstalls its record: it returns to the list of
available items. Signing out keeps the hubs and forgets the links.

## Document and HTTP

An account has at most one 256 KB JSON document:

```ts
type Doc = {
  plugins: { id: string; source: string; note: string }[];
  mcp: { id: string; config: object; note: string }[];
  skills: { id: string; description: string; content: string }[];
  projects: { id: string; source: string; note: string }[];
  actions: Catalog | null;
};
```

`skills` and `projects` are additive: old documents omit these collections. The API
preserves each existing collection when an old desktop sends a PUT without its
key. Sending `skills: []` or `projects: []` removes that collection's definitions. Previous Actions keep the existing
contract; they still have no editor in the SaaS and no individual sharing
control.

| Route | Authentication | Body and result |
| --- | --- | --- |
| `GET /api/catalog` | desktop Bearer | `{ catalog: Doc or null, revision: number or null }` |
| `PUT /api/catalog` | desktop Bearer | `{ catalog: Doc, revision }`; returns the document and revision |
| `/catalog` and `/catalog/:kind` | browser cookie and CSRF | item CRUD in `plugins`, `mcp`, `skills` or `projects` |

`revision: null` means the first write. A different revision returns 409,
without writing. The browser keeps the draft for review; the desktop updates its
cache and reports the conflict, without silently resending. Desktop editors
carry the revision from when they were opened; a background refresh does not
authorize overwriting another edit. A network failure blocks only shared
editing. Private edits stay available offline.

The server validates types, unique names, remote sources and limits before
persisting. Skill IDs follow `[a-z0-9][a-z0-9-]{0,55}`; the description allows
up to 2000 characters and the content up to 65536 bytes. The content is a
Markdown body, without frontmatter: the desktop generates the name and
description with escaped YAML strings.

Plugins store a Git address or a `.zip` URL, never an upload of a local folder.
MCPs store the portable command or URL and arguments. `env` and `headers` values
stay empty in the cloud; the server refuses filled-in values. Credentials are
filled in on each Mac and preserved during updates. Arguments and free text are
content published by the person: do not put secrets in them. The service does
not run commands, install plugins or access the registered sources.

## Identity and local persistence

`<root>/catalog.json` stores `{ revision, doc, links }`. `links` maps
`<type>:<id in the account>` to a local ID. Taken private names get a distinct ID
for the remote definition (`cloud-<name>-<n>`), preserving the private item. An
old cache without `links` migrates the historical links by name. Switching
account/origin forgets the previous cache, keeping local items.

`<root>/skills.json` stores installed definitions. Each skill is materialized in
`<root>/skills-packages/<id>/`, with Claude/Codex manifests and
`skills/<id>/SKILL.md`. The plugin hub contains `skill-<id>` and reuses the
existing selection and adapters. These packages appear on the Skills page and in
the selectors, without duplicating the registration on the Plugins page.

Each catalog belongs to the person (`user_id`) or to the organization
(`organization_id`), exclusively in the database. The desktop synchronizes the
personal catalog and lists plugins, MCPs and skills of every organization with
an accepted membership. Each institutional item shows the organization's name
and `Install here`, without requiring a copy into the personal account. The
installation creates an independent local record, without automatic activation
or publication. An equivalent local definition counts as installed, so the
desktop does not offer a duplicate installation. Plugins match by
case-insensitive ID and source; GitHub owners and repositories also compare
case-insensitively and ignore a `.git` suffix. The personal catalog uses the
same rules,
MCPs by ID and portable configuration after credentials are blanked, and skills
by their full definition. Installing from stale state links that local record
instead of creating another one. In the Cloud, the owner and administrators do CRUD; copying
definitions between catalogs is still optional. See
[organizations](cloud-organizations.md) and
[ADR 0039](../decisions/0039-organization-catalog-on-desktop.md).

`GET /api/organizations/:id/catalog` accepts Bearer and returns the same
`{ catalog, revision }` envelope as the personal catalog, authorized by the
current membership. The desktop enumerates `GET /api/organizations` and fetches
each document separately, preserving the per-response limit. A 404 removes that
catalog's availability and allows compatibility with an old Cloud. Network
failures preserve the cache.

`catalog.json` adds `organizations: [{ id, name, revision, doc, links }]`, absent
from old caches. Those `links` identify local installations by type and ID; they
never take part in personal PUTs. Collisions use distinct local IDs, including
between organizations and personal items not yet installed. A refresh updates
the available definitions; installations and credentials stay independent.
Signing out or losing membership keeps the installed records and files.

## IPC

| Command | Arguments | Return |
| --- | --- | --- |
| `catalog_state` | none | connected, revision, plugins, mcp, skills and shared |
| `catalog_install_project` | organization (nullable), id, revision, directory, existing | registered Project; rechecks current membership, revision and source before clone or link |
| `catalog_share` | kind, local id | empty; publishes and links, or links to an equivalent personal plugin or skill |
| `catalog_copy` | kind, local id, newId | empty; creates a private definition |
| `catalog_install_plugin` | account id | empty; installs the selected source |
| `catalog_install_skill` | account id | empty; materializes the skill |
| `catalog_install_organization_item` | organization, kind, id, revision | empty; checks the membership and displayed revision and installs locally |
| `skill_hub` | none | installed Skill[] |
| `skill_save` | skill, revision | Skill[]; publishes only if linked |
| `skill_remove` | local id | Skill[]; removes only from this Mac |

Catalog refresh runs through `cloud_status` with `refresh: true`, including
**Refresh account** and the existing focus/periodic refresh. The unused
`catalog_refresh` IPC was retired in [ADR 0043](../decisions/0043-retire-unused-ipc.md).

`plugin_save` and `mcp_save` also receive `revision` when editing a shared item.
New private items do not need a revision. The `catalog` event updates the
interface's hubs and markers. `plugins` and `skills` in the state include
`local_id` and `installed`; plugins also include `source_changed`. `local_id`
names the linked installation or, when it is missing, the equivalent private
item that satisfies the definition. `shared` maps
`<type>:<local id>` to the ID in the account.

## Git projects

Projects are authored in the personal or organization browser catalog. Members
can read organization projects; owners and administrators can edit them. The
desktop's Projects settings page and sidebar Add project action open a selector
with personal and organization definitions. Choose several projects and a parent
directory once. Each successful clone is registered immediately; failures remain
selected with their individual errors. Each failed row highlights the reason
and shows the Git diagnostic (up to 4096 characters). Authentication failures
explain how to check the Mac’s Git account, SSH key or token. A missing repository
can mean a wrong address or missing access; the UI preserves that distinction.
Other failures retain their diagnostic without claiming an access problem. URL
credentials, query strings and authorization headers are removed from details;
diagnostics remain local and are never sent to Cloud. Retrying does not repeat successful rows.
Local folder registration remains available without an account.

Project IDs match `[A-Za-z0-9][A-Za-z0-9_.-]{0,127}` and become clone directory
names. Sources accept HTTPS without user information, SSH with the `git` user,
`git@host:path`, and GitHub `owner/repo` or `github.com/owner/repo`. They reject
credentials, query strings, fragments, local paths, traversal, percent encoding
and other Git transports. `note` allows 2000 bytes and source allows 4096 bytes.
The shared [fixtures](../../fixtures/cloud-api.json) cover accepted and rejected
sources and old documents. Rails and the production Rust parser consume them.

`catalog_state.projects` contains each portable definition plus nullable
`organization`, `organization_name`, `revision` and `local_path`. The path is
shown while its directory and board registration exist, or when a registered
project has the same normalized Git origin. Projects already on the Mac are
omitted from the installation selector. The desktop may omit this additive state
field on older versions.

`catalog_install_project` fetches the current account or organization document
and rejects a changed revision or definition before cloning or linking. A conflict
refreshes the cache; reopen the selector to review the current definitions. With
`existing: false`, `directory` is the chosen parent. With `existing: true`, it
is the existing repository root. A matching root and origin can be registered
again without resetting changes or making another clone. Origin comparison
normalizes GitHub shorthand, scp syntax and an optional `.git` suffix; changing
between HTTPS and SSH requires the catalog source to match the local origin.
If a registered project already has that origin, installation records its path
and returns it without cloning or asking for the folder again.
Origin probes use a snapshot of registered projects without holding the board or
catalog mutex. Installation rechecks that a match is still registered before
linking it; catalog writes remain serialized.
Local project additions and removals use the same catalog guard, acquired before
the board lock. They cannot change the checked registration set during the Cloud
request or checkout. These commands run on worker threads while waiting; board
reads and unrelated workspace edits remain available. IPC payloads are unchanged.
An unrelated directory, nested repository subdirectory or different origin is
rejected. Git clones into a temporary sibling directory and moves the completed
clone into the reserved destination. Failures remove only that temporary clone
and empty reservation; existing files are preserved. Incomplete clones never
become registered projects on a retry.

Registration uses the existing board Project format. Personal and organization
cache `links` store `projects:<catalog id>` to the local path. These installation
links do not publish filesystem state or change catalog revisions. Definition
removal, sign-out and membership revocation preserve local files and registered
projects. Updates do not pull or replace an installed clone.

Cloning uses local Git credentials, disables terminal password prompts and Git
hooks, and allows only HTTPS and SSH transports. It does not run project setup,
install dependencies or configure skills, MCPs or plugins. Native credential
helpers and SSH authentication require a configured Mac; browser tests simulate
cloning, while Rust tests exercise real Git against a local upload-pack fixture.
The additive `organization_items` field contains `{ organization,
organization_name, revision, kind, id, description, installed, local_id }`.
`local_id` is the installed local item that satisfies the definition, or null. The frontend
tolerates its absence. The installation queries the organization's catalog
again; if the document changed, it updates the cache and returns a conflict
before installing. `installed` also covers equivalent local definitions, even
before an organization installation link exists. Institutional MCPs enter the
hub only after `Install here`,
with empty credentials.

## Evidence

- [`fixtures/cloud-api.json`](../../fixtures/cloud-api.json): empty, current and
  legacy documents exercised by the production Rust parser and actual Rails
  endpoints. Paired tests cover revision conflicts, credential rejection and
  preservation of skills omitted by older clients. See the
  [shared fixture workflow](../operations/development.md#shared-cloud-api-fixtures).
- `src-tauri/src/catalog.rs`: link migration, name collisions, preservation of
  private items and credentials, rejection of malformed documents, personal
  equivalence, GitHub source comparison and linking on share.
- `src/catalog.test.ts`: resource-list sources, differing same-name definitions
  and grouping of missing definitions.
- `src-tauri/src/skills.rs`: validation, directory isolation, frontmatter, both
  providers' manifests and content updates.
- `e2e/cloud.spec.ts`: explicit sharing, private copy, offline editing and
  revision-conflict flows over the mock. Organization installation rules are
  covered by `catalog.rs`; the optional installation form has no dedicated E2E.
- `e2e/mcp.spec.ts`: local connection checks, login retry and logout for shared
  MCPs while Cloud is unavailable, pending-action guards and authentication
  independent of catalog revisions. Browser tests simulate OAuth; consent with
  a real server still requires manual validation. No persisted or IPC format
  changes are introduced.
- `prometeu-cloud/test/integration/catalog_test.rb` and
  `catalog_browser_test.rb`: authentication, per-account isolation, browser/API
  CRUD, conflict and compatibility.
- `prometeu-cloud/test/browser/catalog.spec.js`: real Rails forms, CSRF,
  consumption of the desktop API and layout in Chromium/WebKit.
