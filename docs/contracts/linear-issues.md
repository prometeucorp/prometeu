# Linear issue discovery and assignment

Status: current contract.

The Issues screen contains Linear and GitHub provider tabs. The Linear pane has
**Mine** and **Available in my teams** scopes. Mine retains
the existing assigned-issue list and workspace launch action. Available shows
open, unassigned issues only from teams of which the connected Linear user is a
member. Both tabs share search, team filtering, state grouping and priority
ordering. When Mine is empty and available issues exist, the screen shows up to
three suggestions ordered by priority and a link to the complete available tab.
The sidebar combines known assigned-issue counts from both providers; the
launcher issue picker still contains assigned Linear issues only. See
[GitHub discovery](github-issues.md). Claiming an issue does
not create a workspace; the newly assigned issue becomes eligible for the
existing workspace action.

`linear_issues({ force })` returns `{ issues, available, fetched_at, available_error? }`. The Rust
adapter fetches both lists through paginated GraphQL queries, with at most ten
pages of 50 per list. The available query filters by missing assignee, active
state and team membership in Linear. The two-minute cache persists privately in
`linear-issues.json`. Older cache files without `available` trigger an immediate
refresh; older binaries ignore the added field. Network fetches run outside the
cache lock. Fresh cached reads bypass the separate refresh lock. Network
refreshes serialize so an older fetch cannot publish after a newer one; claims
remain independent. Snapshots fetched before
a claim or account change are discarded.
If the available query fails after Mine succeeds, the response keeps Mine,
retains previously fetched available issues when present, and sets
`available_error`. The Available tab shows the error and offers a retry when
there are no retained issues; the Mine count remains available. A failed Mine
query still fails the whole request. Refresh errors preserve the last successful
screen snapshot. Disconnect and
successful reauthorization clear the cache.

`linear_claim({ id })` requires a credential granted the `write` OAuth scope.
It checks the current issue before mutation: unassigned, active and in a team
joined by the user. It then calls `issueUpdate` with the authenticated user's
ID and returns the updated `Issue`. The UI disables further claim actions while
the request runs, updates the list after success, and refreshes from Linear.
Failure leaves the issue visible with an error; no local assignment is invented.
Linear does not expose a conditional assignment mutation, so another assignee
could change the issue between the check and update. The direct claim action
accepts this residual risk. The mutation response must confirm success. The
backend invalidates its cache after a successful claim.

New OAuth authorization requests `read,write` with PKCE. Existing credentials
without recorded scopes remain connected for reading and must be reauthorized
before claiming. Reauthorization retains the old credential if authorization
fails. A failed reauthorization keeps the visible issue snapshot until a later
refresh succeeds. `linear_status` and the `linear` event expose `can_assign`; they never
expose tokens. Older `linear.json` files without `scopes` or `who.id` deserialize
with safe defaults; older binaries ignore the new fields. A refresh response
that omits scopes preserves the recorded
grant. Disconnect deletes both the credential and the issue cache.

The browser mock implements the same commands with fictional issues and an
in-memory claim. The feature is independent of Claude, Codex and Antigravity.

Evidence: `src-tauri/src/linear.rs` tests cover old formats, scope decoding,
partial availability failure and
claim eligibility; `src/backend-contract.test.ts` checks IPC registration;
`e2e/issues.spec.ts` checks pending actions, count updates, keyboard focus,
partial availability failure and read-only permission navigation through the
browser mock. Live OAuth consent
and mutation require a connected Linear account for manual verification.
