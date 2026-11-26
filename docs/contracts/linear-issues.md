---
type: Contract
title: "Linear issue discovery and assignment"
description: "The available issues of a team, the OAuth write scope, the claim flow and the compatibility of the cache."
status: stable
---

# Linear issue discovery and assignment

This is the current contract.

When a provider is not connected, it shows a small connection card. The scope
tabs, the filters and the column headings are not shown until there is a
connection. A column heading shows only when there are tasks that agree with it.
Keyboard navigation does not go to controls that are not shown.

The Tasks screen has a Linear tab and a GitHub tab for the providers. The Linear
pane has two scopes: **Assigned to me** and **Unassigned**.

- **Assigned to me** keeps the current list of assigned issues and the action
  that starts a workspace.
- **Unassigned** shows open issues without an assignee. It shows issues only
  from the teams that have the connected Linear user as a member.

The two tabs use the same search, the same team and status filters, the same
groups by state and the same sort by priority. The filters operate together with
the search, and the search includes label names. A person can remove all the
filters at one time. The options come from the full scope, thus the search does
not remove options. The team selection keeps its current local preference. The
status selection is temporary.

Each row shows the identifier, the title, the team or project, the labels, the
status and the time of the last update. **Start** opens the launcher. If a
workspace for the issue is there, the row shows **Open workspace**. When the
assigned list is empty and there are unassigned issues, the screen shows a
maximum of three suggestions. It sorts them by priority and shows a link to the
full Unassigned tab.

The sidebar adds the known counts of assigned issues from the two providers. The
issue picker of the launcher continues to show only assigned Linear issues. See
[GitHub discovery](github-issues.md). A claim does not make a workspace. After
the claim, the issue is assigned and the current workspace action can use it.

`linear_issues({ force })` returns
`{ issues, available, fetched_at, available_error? }`. The Rust adapter reads
the two lists through GraphQL queries with pages. Each list has a maximum of ten
pages of 50 issues. The query for available issues filters in Linear by three
conditions:

- The issue has no assignee.
- The issue has an active state.
- The team of the issue has the user as a member.

The cache is private and it keeps the data for two minutes in
`linear-issues.json`. These rules apply to the cache:

- If an older cache file has no `available` field, the adapter reads the issues
  from Linear again immediately. Older binaries ignore the added field.
- The adapter does network reads when it does not hold the cache lock.
- A read of a cache that is not expired does not use the different lock for
  network reads.
- Network reads occur one after the other. Thus an older read cannot publish
  its result after a newer read. Claims do not use this sequence.
- If a claim or an account change occurs after the adapter starts a read, the
  adapter discards the result of that read.

Sometimes the query for assigned issues is correct, but then the query for
unassigned issues has a failure. In this condition, the response keeps the
assigned issues. It also keeps the
available issues from a previous read, if there are issues, and sets
`available_error`. If there are no kept issues, the Unassigned tab shows the
error and an option to try again. The count of assigned issues stays available.
If the query for assigned issues has a failure, the full request has a failure.
If a new read has a failure, the screen keeps the data from the last correct
read. A disconnect or a correct new authorization removes the data from the
cache.

`linear_claim({ id })` is possible only with a credential that has the `write`
OAuth scope. Before the mutation, the command reads the issue again and makes
sure of these three conditions:

- The issue has no assignee.
- The issue is active.
- The team of the issue has the user as a member.

Then it calls `issueUpdate` with the ID of
the authenticated user and returns the updated `Issue`.

While the request operates, the interface disables the other claim actions.
After a correct claim, the interface updates the list and reads the issues
from Linear again. If the claim has a failure, the interface continues to show
the issue with an error.
The interface does not show a local assignment that Linear did not make.

Linear does not have a conditional mutation for an assignment. Thus a different
person can change the issue between the check and the update. The claim action
accepts this small risk. The response of the mutation must show that the
mutation is correct. After a correct claim, the backend discards its cache.

A new OAuth authorization requests the `read,write` scopes with PKCE. A current
credential without recorded scopes stays connected and can read issues. Before a
claim, the person must authorize Prometeu again. If the new authorization has a
failure, the backend keeps the previous credential. After that failure, the
screen continues to show the same issues until a subsequent read is correct.

`linear_status` and the `linear` event give `can_assign`. They do not give
tokens at any time. When an older `linear.json` file has no `scopes` or
`who.id`, the backend reads it with safe default values. Older binaries ignore
the new fields. If a response to a token refresh does not include scopes, the
backend keeps the recorded scopes. A disconnect deletes the credential and the
issue cache.

The browser mock has the same commands with fictional issues and a claim that
it keeps only in memory. The feature does not change between Claude, Codex and
Antigravity.

The tests in `src-tauri/src/linear.rs` examine these items:

- The older formats.
- How the backend decodes the scopes.
- A failure of only the available issues.
- The conditions for a claim.

`e2e/issues.spec.ts` uses the browser mock. It examines these items:

- The actions that wait for a response.
- The count updates and the keyboard focus.
- A failure of only the available issues.
- The navigation with read-only permission.

`e2e/issues.spec.ts` also examines the focus of the shared filters, the
visibility of the actions at different widths and the claim and workspace
interactions. `src/backend-contract.test.ts` examines the IPC registration.
`src/issues-filter.test.ts` examines the task filters. The live OAuth consent
and the live mutation are possible only with a connected Linear account. A
person must examine them manually.

This feature does not change an IPC or a stored format.
