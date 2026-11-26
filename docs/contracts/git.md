---
type: Contract
title: "Workspace Git"
description: "The index, the worktree, the review and the Git operations on the machine of the owner."
status: stable
---

# Workspace Git

The implementation is complete. Desktop Git operates on the machine that owns
the workspace.

## Responsibilities

`src/workspace-changes.ts` controls the selection, the drafts and the
presentation. The desktop facade `src-tauri/src/session/git.rs` finds the
repositories with the shared admission rules of the core and receives
`RepositoryGit`. `prometeu-git::NativeGit` runs the current native operations
with isolated arguments and without a shell. The core owns the IPC model types,
which do not change, and the check for an agent that operates before a pull or
a discard. The native adapter owns the caches, the sequence of changes and the
effects on the file system and on Git.

`src/diff.ts` puts the files of the scope one after the other in one scroll
area. It shows a unified diff or a side-by-side diff. It does this in Changes,
in Compare and in the commit view. The two presentations keep the line
numbers of each side. The side-by-side view aligns the blocks of removed lines and
added lines between context lines. The current viewer continues to edit full
files. A person opens it with **Open file** or with a double-click on the
header.

The panel header shows the repository, the branch and the remote, with the pull
and push counters and an action menu. Explicit tabs show Local files, Stage,
History and Compare. The commit form is only in Stage. Local files has an entry
to review the index. History lists the commits in the panel and shows the
selected commit in the center area. Compare lets the person edit the reference. The
groups keep their state when Git refreshes. The pt-BR labels keep the terms
branch, commit and stage. The tooltips tell what the operations do. The list
does not show the instructions again.

The branch and the agent status are different items. When a person selects a
branch, the app does not change the checkout of a session that operates. It
opens a current workspace or the launcher, with a mandatory worktree. If a
folder without a workspace in Prometeu has the branch, the app shows that
location and does not offer a checkout on it.

The launcher can make a worktree on a new branch, or on a current local or
remote branch. If a remote branch does not have a local branch, Git makes a
local branch from that remote ref. The launcher refuses a branch that a
different folder has checked out. Current branches stay after the worktree
cleanup. New workspace branches follow the usual cleanup rule. The selected
base for the comparison stays different from the remote ref that opened the worktree.

## IPC commands

Each command receives `id`, the local workspace identifier. Commands for one
repository also receive `repo`, an index into `Workspace.repos`. The backend
refuses a workspace that is removed, returned, in preparation or has a failure.
File paths are relative to the selected repository. The backend refuses these
paths:

- Absolute paths and traversals.
- `.git`.
- Parent folders with symlinks that go to a location that is not in the
  repository.

| Command | Additional arguments | Return |
| --- | --- | --- |
| `workspace_git_status` | none | `GitStatus[]` |
| `workspace_git_diff` | `repo`, `scope`, `path?`, `reference?` | `GitDiff` |
| `workspace_git_action` | `repo`, `operation`, `paths`, `message?`, `expected?`, `remote?` | empty or error |
| `workspace_git_history` | `repo` | a maximum of 100 `GitCommit` |
| `workspace_git_branches` | `repo` | `GitBranch[]` |
| `workspace_git_conflict` | `repo`, `path` | the current, ours and theirs versions |
| `workspace_git_resolve` | `repo`, `path`, `was`, `text` | empty or error |
| `tree_git_status` | none. `id` can also be a project. | `GitFile[]`, and no error |
| `tree_restore` | `rel`. `id` can also be a project. | empty or error |
| `file_base` | `rel`. `id` can also be a project. | the committed text, `""` or `null`, and no error |

The TypeScript types are in `src/types.ts`. `src/ipc.ts`, the Tauri registry and
`src/mock.ts` have the same commands. The board does not store a new field. The
selection and the drafts stay while the workspace is in the window.

### Status and diffs

`GitStatus` contains these items:

- The identity (`repo`, `name`, `branch`, `base`).
- The upstream and the remotes.
- The `ahead`/`behind` counters relative to the upstream.
- `has_head` and `merging`.
- The `index` token, which the UI does not parse.
- The `staged`/`changes`/`conflicts` groups.
- `error`.

Each file has `path` and `status`. A file with only some staged changes shows
in the two groups, with different diffs. A rename shows as a deleted file and
an added file. The tab counter counts the unique local paths in each repository.

The status uses the porcelain format with NUL separators. An error in one
repository does not hide the others. The UI keeps the last known status, shows
the error and disables changes in that repository. A project can be a folder
that is not a git repository. Then the workspace starts without a worktree and
without a branch. `list_branches` answers `git: false`, and the launcher locks
the two toggles. The panel shows the Git error, as for a repository that does
not answer. A previous response cannot replace the selection of a different
workspace or repository.

The backend shares one porcelain read at a time for each canonical worktree for
the Files marks. The full status has its own single-flight slot for each
worktree and base for the comparison. The full status includes the branch and upstream,
the remotes, the counters for commits before and after the upstream, merging and the index fingerprint. It always
reads its porcelain between the two fingerprint reads. Thus the `expected` token
agrees with the files that it lists. It does not use an older Files scan again.
The Files marks can use the porcelain that a status scan read immediately
before.

Git changes from the app and file writes, creations, renames and moves to the
trash make the cache of the changed worktree incorrect. The app removes that
cache before it publishes the result. If this occurs during a scan, the scan
discards its result and starts again. It continues until one scan is complete
without a change. Then the requests that wait receive the status.

If a scan has a failure, the requests that waited for it receive the failure.
Subsequent requests do not use that result. Different repositories use
different slots. The app does not hold the change lock while a status
consumer waits. The app uses a full status again for a maximum of two seconds
after its scan, and the raw porcelain for one second. The app discards expired
entries that it does not use. Thus removed worktrees do not keep their last
scan. A fallback that operates while the panel shows finds external edits after
those limits. It does not have to have a file system watcher.

The frontend refreshes the status:

- When the person goes into a workspace.
- When the list of repositories changes.
- When a file is saved.
- After Git actions.
- When a turn completes in the open workspace.
- When the window comes back to the foreground.
- While Changes shows on the screen.

Other redraws of the board or the chat do not request the status. A menu or a
rename input can stop the redraws for some time. During that time, the app
records the completion of a turn. It uses the record one time when the open
workspace can render again. While Changes shows, a fallback operates each five
seconds on AC electrical supply. It operates each ten seconds on battery or on
an unknown supply. Hidden windows and windows
without the focus do not scan. These are request intervals. The time of the Git
command adds to them.

### File tree marks

`tree_git_status` gives the colors of the side **Files** tree. It accepts the
same `id` as `list_dir`, which is different from the other commands. This value
can be a workspace or a project
(see [the root of the file commands](ipc.md#root-of-the-file-commands)). Paths
are relative to that tree root. Thus a grouping folder adds the folder of each
worktree before each path. A project that a person added on a subfolder
sees only its own changes.

The command changes `status` into one mark:

- `U` for conflicts.
- `D` for a file that is missing from the worktree. This includes a file
  that was added to the index and then deleted (`AD`).
- `A` for other untracked or added files.
- `D` for deletions in the index.
- `M` for all other files.

The command lists untracked files one by one, as the Changes pane does. Thus
ignored files in a new folder do not get a mark. The tree adds rows with a line
through the text for `D` paths. These paths are not on the disk, thus
`list_dir` does not return them. A folder that is not in Git, or a repository
with a failure, gives no marks and no error.

A workspace repository root gets its marks from the shared porcelain. It can
use a snapshot that Changes read in the last second. A project that a person
opened on a subfolder keeps the query that has a limit on its path. Thus only its
own files show. The command is async, thus the scan does not operate on the primary
thread.

While Files shows, the marks refresh each 15 seconds on AC electrical supply.
They refresh each 30 seconds on battery or on an unknown supply. They also refresh immediately after the cache is
removed and when the window comes back to the foreground. Hidden windows and
windows without the focus do not scan. The tree skips a tick while a previous
scan operates. Only the latest result paints the marks.

When a person saves an open file, the marks refresh in their rows. The other
file rows and their click targets stay. A full redraw is necessary only for
list changes. The tree lists the open folders again in these conditions:

- A refresh of the marks changes the set of new or deleted paths, for example
  a file that an agent made.
- A turn completes in the open workspace. This also includes ignored files
  without marks.

When the person opens the workspace on the screen again, the tree is listed
again. An explicit redraw cancels all previous redraws that wait for the same tree.

`tree_restore` writes such a row back to the disk. It finds the repository that
holds `rel` under the tree root. The app uses the index first, before `HEAD`:

- If the index holds the path, the command writes the file from the index, and
  the file stays staged. An example is a file with staged edits that the person
  removed from the disk subsequently (`MD`).
- If the deletion of the path is also staged, the file comes back from `HEAD`,
  in the index and on the disk.

For a folder, the command does this for each file. It refuses an entry that is
on the disk. Thus it cannot discard edits. It shares the Git change lock. Like
the scan, it is async.

Without an upstream, the counters are 0 and the action is **Publish branch**.
This does not mean that the commits are published. A detached HEAD has
`branch: null`. The person can continue to read, but commit, pull and push are
blocked.

`GitDiff` contains the resolved `base` and `head`, and the files in the `Change`
format that the review viewer uses. These are the scopes:

- `changes`: the index compared to the worktree. It includes untracked files.
- `staged`: HEAD compared to the index. It also operates before the first
  commit.
- `compare`: from the merge base of the selected reference and HEAD, to HEAD.
- `commit`: from the first parent to the selected commit. An initial commit uses
  the empty tree.

The comparison of branches and the history do not include local changes. An empty
patch can show a binary file, metadata or the limit of 400,000 bytes. The UI
tells the person about that limit. The diff shows a maximum of 2,500 lines for
each file. It builds the body of each file when the file comes into the area on the
screen.

Changes shows one scope at a time: `staged` or `changes`. The selected tab sets
the scope, also when the scope is empty. This is necessary because the same
file has two different diffs. Stage and Unstage do not change that tab. A click
on a file of the scope on the screen scrolls to the file. If the diff of the
file is closed, the click opens it. The other patches do not build again. The
path filter changes the list and the diffs. The **Stage everything** and
**Unstage everything** actions continue to operate on the full scope. This includes
files that the filter hides.

**Reviewed** records that the person read the file and closes the diff of the
file. **Mark all as seen** does this for all files. If the mark goes away, the
diff opens again. The mark goes away when the person removes it or when a new
patch comes. This also applies when the filter hid the file during that time.
A patch that agrees with a previous mark again does not close the diff. Reviewed
does not stage or discard content.

The current mark in localStorage continues to use the workspace, the
repository, the path and the patch fingerprint. A change to the patch cancels the
mark. The progress counter counts all files of the scope. **Next unreviewed**
removes the filter and opens the next file that is not reviewed. The layout and
the filter are temporary window state.

### Editor change gutter

`file_base` returns the `HEAD` text of an open file. The editor uses it to mark
these lines near the line numbers, with the colors of the Files tree:

- Added lines (green).
- Changed lines (yellow).
- The locations where lines were removed (a red notch).

Like `tree_git_status`, this command accepts a workspace or a project `id`. It
finds the repository that holds `rel` under the tree root. A file that Git does
not know at this time, untracked or only staged, returns `""`. Thus all lines show as
new. These files return `null`, and the editor shows no marks:

- An ignored file.
- A path that is not in Git.
- A binary file.
- A file larger than the 2 MiB limit of `read_file`.

`src/line-changes.ts` does the comparison against the live buffer. Thus edits
that are not saved move the marks immediately. After 500 edits, the changed
middle becomes one changed range. The viewer reads the base again when a
different file opens. It also reads the base on board events, a maximum of one
time in two seconds. This includes a commit that does not change the file.
Tests: `src/line-changes.test.ts` and `file_base_reads_the_committed_text_and_empty_for_new_files`.

### Review notes

Local files, Stage and Compare also have permanent notes on a line, a range in
one hunk or a file. Notes are independent of Reviewed. A person adds them
explicitly as one tag to the draft of a selected conversation. See the
[review contract](diff-review.md) for the position, the storage, the meaning of
the submission, the limits and the compatibility.

### Mutations

`operation` accepts `stage`, `unstage`, `discard`, `commit`, `fetch`, `pull`,
`push` and `publish`. Stage operates only on the selected paths, as literal
pathspecs. Unstage changes the index and keeps the files. It also operates
before the first commit. A commit does not stage files automatically.

Discard removes the work that is not staged on the selected paths. All of these
paths must be in the `changes` group, and none can be in `conflicts`. A path that Git
follows goes back to its index version (`git restore --worktree`). Thus the staged
content stays. The command removes an untracked path with `git clean`. This
does not touch ignored files or other untracked paths.

The backend refuses Discard while a tab of the workspace runs a turn, because
the agent can edit the same files. The UI offers Discard for each file. It first tells
the person to give a confirmation. Git cannot put the content back.

A commit must have a message and a branch. It must also have a prepared index,
or a merge that is not complete and has no conflicts. The `expected`
token identifies HEAD and the index entries. If the app sees changes between
the read and the operation, a new review is necessary. The read examines the
token before and after it builds the status.

Git keeps its own locks. External operations and hooks continue to use the
repository. The UI does not control these processes. The app does its changes
one at a time. It refuses a different operation while one operation continues.

Fetch updates the remotes. Pull must have these conditions:

- The worktree and the index are clean.
- The agent does not run a turn.
- The pull is a fast-forward, without rebase or autostash.

Push sends only HEAD to the configured upstream reference. Publish must have a
known remote, and it configures the upstream of the branch. Push and Publish
disable `followTags`. They do not force-push and do not publish other branches
or tags. After a failure, the draft and the selection stay. The UI refreshes the
status after the result.

### File menu

A right-click on a file row opens a context menu. The pure
`src/changes-menu.ts` builds the menu. It has these items:

1. **Open diff**, or **Resolve conflict** in the conflict group.
2. **Open file**. For a deleted file, this item is disabled and shows an
   explanation.
3. **Stage** or **Unstage**, and **Discard changes**, for each scope.
4. The groups that the Files tree also has, from `src/file-menu.ts`. These
   groups attach the file to the conversation and copy its paths.
   They also show the file in the file manager.

A deleted file offers only its paths. The Git items stay on the menu but are
disabled while a different operation runs. They are also disabled while an
agent operates, and they show `err.git.agent`. When a person selects a Git item,
the app does the same check again. An agent can start while the menu is open,
and the backend does not examine the agent before stage or unstage. In this
condition, the item does not do an action, and the panel reports `err.git.agent`. The
backend also refuses Discard while a tab runs. Conflicts have no Git item.
Their editor stages the resolution. The menu operates on one file. There is no
multiple selection.

The tree and this panel keep different builders. They share only those groups at
the end. The Git group, the scope and the agent state have no meaning for the
tree. A discriminated union for the two subjects makes each hook
conditional.

### Conflicts

The editor shows text versions. It does not remove whitespace or line breaks.
The draft stays different from the current file. The resolution compares `was`
with the text on the disk. It refuses changes that occurred at the same time and
conflict markers. Then it writes the result and stages the file. A commit does
not occur automatically. If the person selects ours, it is possible that the
index becomes equal to HEAD. `merging` lets the person complete that merge also
without staged files.

Binaries, symlinks and texts above the limit do not go through the merge
editor. A person can resolve them with external tools and stage them
explicitly. Rebase, cherry-pick and other complex operations continue to use
external Git. The interface does not offer a general action that completes
those sequencers.

### Archived worktree cleanup

Archive and finish stop the workspace processes and run the archive script to
the end. Then they apply the local cleanup preference for that workspace. The
desktop does archive, the move out of the archive and the admission of process launches one at a
time for each workspace.

The archive waits until each accepted launch installs and publishes its tab. It
stops the agents before the hook. Then it stops the last set of tabs before it
completes. New launches and resumed launches are rejected while the hook runs.
A person must explicitly move an archived workspace out of the archive
before a new launch. This gate
is only in memory. It also prevents a manual cleanup during a second archive
hook. Other workspaces stay available.

The desktop runs the hook on its command worker. It waits for the foreground
hook before it lets the cleanup start. Thus an automatic deletion cannot occur
while the hook continues to read the worktree. Hooks must not release resources in a
background process. The hook has no timeout. The current processing of an exit
code that is not 0 does not change.

By default, the scoped dialog shows a question: clean up or not. A cancel
keeps the worktree, the local branch, the transcript and the archived card.

Settings / General / Clean up worktrees has a control for automatic cleanup.
The control is off by default. It applies to manual archive and finish, in all
projects and with all agents. It removes the full eligible worktree. This
includes ignored files, for example `node_modules`, and the branches that the
workspace made. It does not remove only the dependencies.

When the person sets the control to on, the person accepts the loss of work that
is not committed or not merged. Thus the app cleans each listed row with
`force: true`, without a modal. With force, the app continues to refuse
workspaces that are not archived and initial clones. It keeps the saved
branches that were there before. Missing rows do not cause an action. Scan failures and
deletion failures report an error. Manual cleanup stays available, also when
the control is off. The preference is local to the webview, as
[persistence](persistence.md#automatic-worktree-cleanup) tells.

The scoped offer uses the same `cleanup_list` and `cleanup_worktree` commands as
the screen of archived workspaces. A safe worktree is selected at the start. A
worktree with changes that are not committed, or with a branch that is not
merged, is not selected at the start. It shows the cause. The person must
select it before the destructive confirmation can run with `force`.

The checks stay closed on a failure. If `git status` has a failure in the
worktree, the app does not know if the worktree is clean. Thus the row is
blocked and shows the message from Git. The cleanup removes the
worktree and the branches that the app made for that workspace. It keeps the
branches that were there before, the archived card and the transcript. Initial
clones are not eligible.

## Compatibility and evidence

The screen uses the `workspace_git_*` commands for each repository.
[ADR 0043](../decisions/0043-retire-unused-ipc.md) retired the
`workspace_diff` IPC, which had no consumer, and its mixed-diff implementation. The diff presentation
continues to use its current view model. There is no migration of the board,
the transcript or the collaboration protocol.

- `src-tauri/crates/git/src/tests.rs`: Git repositories on the disk, an index with
  only some changes, special paths, discard, commit, local remotes, conflicts
  and merge.
- `src/changes-menu.test.ts`: the file menu for each scope, for a deleted file,
  while an agent runs, and when an agent starts after the menu opened.
- `src/diff.test.ts`: the line numbers on the two sides, and how the view aligns
  replaced, added and deleted lines between hunks.
- `e2e/git.spec.ts`: typical review and commit flows, filters, the layout,
  drafts for each repository and protection against UI actions that are not current. These
  tests use the mock in Chromium. Some review, keyboard and layout scenarios
  also operate in WebKit.
- `e2e/critical-flows.spec.ts`: the navigation to the viewer, a large review
  and repositories that stay isolated.
- `e2e/audit-regressions.spec.ts`: the scoped cleanup offers after archive and
  after finish, and the life of the dialog during the deletion.
- `src-tauri/src/session.rs::tests::archive_stops_all_agents_before_the_hook_reads_resources`:
  the desktop stop sequence before the foreground release of resources. The
  admission tests near it examine that a new start is rejected, and the races
  between launch and publication.
- `src-tauri/crates/process/src/archive.rs`: the foreground hook completes
  before the archive and finish replies let the cleanup start.
- `src/cleanup.test.ts`: the compatibility of the preference, the storage of
  the control, the scoped automatic cleanup, blocked and missing rows, and scan
  and deletion failures.
- `src-tauri/tests/mock.rs`: the same IPC commands in the two paths.

## Preparation fetch execution

During the workspace preparation, the refresh of a remote base uses the
command runner port. The command has a limit of ten seconds, a closed stdin
and the output of the parent. If the command exits with a code that is not
0, the app can continue to use a correct local ref. A timeout keeps the current
error code. It also stops the owned process group and collects its processes.
Other Git commands of the repository keep their current adapters. The
preparation tests with repositories on the disk and the `fetch_port_tests` port
tests in `session.rs` examine this boundary. See
[bounded commands](application-core.md#processes-terminals-and-commands).
