# ADR 0090 — Remote input only from approved identities, with separate rights

Date: 2026-10-08
Status: Accepted

Narrows the trust that [ADR 0022](0022-end-to-end-encryption.md) and
[ADR 0042](0042-automatic-key-rotation.md) give a peer's key when that key sends
input. It complements remote control in [ADR 0030](0030-remote-control.md) and
companion devices in [ADRs 0027](0027-companion-devices.md) and
[0036](0036-second-mac-as-companion.md). The implementation lands in three
changes; see [Rollout](#rollout).

## Context

Being in a shared workspace's audience grants three powers at once: seeing and
commenting on the conversation, sending messages that run as prompts in the
owner's agent, and controlling the agent with `turn.interrupt` and
`request.respond` (allow, deny or answer). The owner's own devices also need
`remote_control`; teammates only need the audience, and `audience: null`
reaches every member with a key.

Agents run tools without per-tool approval by default, with the local user's
access to files and processes. A remote prompt is therefore execution on the
owner's Mac, and a remote `request.respond` answers the approvals that exist as
a human gate. ADRs 0022 and 0042 weighed a server-replaced key as a
confidentiality cost: the server reads that person's new content. For input,
the same key authorizes commands.

The relay directory also decides who is a member and which person each device
belongs to (`Member.person`). So three cases reached the agent without any
action from the owner:

- a changed key of a known member;
- a member that first appears in the directory, through `audience: null`;
- a new device whose `person` is the owner, through `remote_control`, whose
  text even arrives without the "Message from" prefix.

The Mac does not take part in device pairing. Phones and other Macs obtain
companion tickets from the Cloud, and the Mac learns their keys only from the
relay directory, the same source a malicious server controls.

## Options considered

1. Keep trusting the directory for input. No friction, but the relay operator,
   or whoever compromises it, can run commands on the owner's Mac.
2. Pause input only after a pinned key changes. It misses new members and new
   devices of the owner's person, which the directory also controls.
3. Run input only from member and key pairs approved on the owner's Mac, and
   split viewing from acting per workspace. Content keeps ADR 0042's automatic
   adoption.
4. Comparable codes or key transparency. They would also protect content, but
   bring back the manual comparison ADR 0042 removed; still future work.

## Decision

Option 3.

### Approved identities

- **Rule.** The owner runs remote input (prompts, `turn.interrupt` and
  `request.respond`) only from a member whose current key it approved on that
  Mac. Viewing and commenting (`watch`, snapshots, the live stream and
  comments) keep following ADR 0042, so content flows while input waits.
- **Scope.** An approval is a member and key pair stored per scope in
  `team-security.json`. Approving a device releases its input in every
  workspace of the scope where its person has the right. Each member has one
  approved key: a changed key, an old key seen again, a new member and a new
  device are all unapproved. The owner's own identity is always approved.
- **Approval moments.** Each one approves the current keys of every device of
  the person and says how many became approved, so an unexpected device is
  visible:
  - turning remote control on, for the owner's devices;
  - granting Send messages or Control to a person;
  - answering the notice below.

  There is no approval at pairing, because the Mac is not part of it. Choosing
  "Everyone in this organization" and mentioning someone in a comment approve
  nobody.
- **Blocked input.** The channel decrypts the message and spends its replay
  receipt exactly as before; the owner then discards the input without running
  it, so it can never run later either. The Mac shows a notice above the
  composer of each conversation in the workspace where the input arrived,
  including desk tiles: "One of {name}'s devices changed or is new. Allow its
  input again?", with Allow and Not now. For the owner's own devices it says
  "One of your devices". The notice names the person, never the device
  ([ADR 0041](0041-members-list-shows-people.md)). It is stored without
  content, survives restarts and returns with the next discarded input after
  Not now. The sender receives no receipt.
- **Writes.** Approvals and paused input are written on the connection's
  encrypted queue, like receipts and links, so a reconnect or an organization
  switch never reloads the store under a write from the previous connection.

### Rights per workspace

| Right | Allows | Teammates | Owner's devices |
| --- | --- | --- | --- |
| View and comment | watch, snapshots, live stream, comments | the audience: chosen people or the whole organization | with `remote_control` |
| Send messages | prompts and `turn.interrupt` | granted per person | with `remote_control` |
| Control | `request.respond`: approvals and answers | granted per person | with `remote_control` |

Send messages and Control are never granted to the whole organization and do
not expand from `audience: null`. Interrupting stops work but cannot start
any, so it stays with Send messages.

- **Board.** `Workspace.rights` holds `{ send, control }`, two lists of people
  next to `audience` and `remote_control`. Both only name people who can view:
  granting a right to someone outside an explicit audience adds them to it, and
  removing someone from the audience drops their rights.
- **Enforcement.** The owner checks the right before the approval: input
  without the right is discarded silently, since there is nothing to approve,
  and only a person with the right can leave the notice above.
- **Share menu.** It keeps choosing who views and comments, and adds Send
  messages and Control submenus that list only the people who view. Choosing
  someone for the audience or mentioning them approves nothing.
- **Conversation header.** A chip next to the viewers names who may act, and
  its title lists each right. Remote conversations show the people their
  owner announced.

### Migration

- On upgrade, the keys already pinned in `peers` become approved: a scope
  without approvals approves its links once. Teams and the owner's devices with
  `remote_control` keep working.
- Existing shares become View and comment: they load with `rights: null`,
  which grants nothing, and every share saved afterwards records its rights.
  The owner sees a one-time notice above those shares' conversations explaining
  the new rights. "Got it" dismisses it on that Mac.

### Owner confirmation

- Enabled by default, as a per-workspace option. A teammate's message becomes a
  pending item in the owner's composer with Send, Edit and Discard.
- The pending item lives only in memory while the app is open. There is no
  offline inbox, and a third party's content is never written to disk.
- An edited message is sent as the owner's own. An unedited one keeps the
  "Message from {name}" prefix.
- The pending item is dropped when its workspace, tab or the sender's right
  disappears.

### Announcement and phone

- The rights travel in the authenticated share announcement as an additive
  `rights` field next to `revision` in the encrypted payload.
  `relay/src/protocol.ts` does not change: `parseShare` drops unknown fields
  inside the share itself.
- Peers hide the controls they lack. On the desktop, a viewer without Send
  messages gets no composer, and therefore no Stop, but a line explaining why.
  Without Control, request cards show as waiting for an answer. The owner
  enforces the rights even when a peer ignores the announcement.
- An announcement without `rights` comes from an owner that predates them, so
  peers keep that owner's controls.
- The phone only reflects the rights in its existing controls: it hides the
  composer without Send messages. It gains no approval buttons.

## Rollout

The decision lands in three changes, in this order:

1. **Approved identities.** Implemented: the rule, the notice, the approval
   moments and the migration of approvals.
2. **Rights per workspace.** Implemented: View and comment, Send messages and
   Control in the board, the share menu, the conversation header, the
   announcement, the desktop and phone controls and the migration of existing
   shares.
3. **Owner confirmation.** Not implemented yet: until it lands, a granted
   teammate's message runs as soon as it arrives.

Each change updates this section and the contracts when it lands.

## Consequences

- A reinstall or a new device needs one answer from the owner before its
  messages run again. The message that revealed it is lost, and its sender gets
  no feedback in this version.
- A silently replaced key can still read new content (ADR 0042), but it can no
  longer run commands on the owner's Mac.
- Approval keeps trust on first use at the moment of approval: approving a
  person approves whatever keys the directory shows for that person's devices
  then. The device count is the only hint of an unexpected device.
- The migration trusts links that ADR 0042 may already have replaced silently.
- A new Mac approves nobody until the owner grants a right, turns remote
  control on or answers a notice.
- After the upgrade, teammates who used to prompt a shared agent lose that
  until the owner grants them Send messages or Control; their messages are
  discarded without feedback, which the one-time notice explains to the owner.
- `team-security.json` gains the additive `approved` and `paused` fields per
  scope. A rollback to a version without them drops both at its next write;
  upgrading again approves the links present then. The relay protocol, IPC,
  receipts and Rust validation do not change.

## Evidence

- [Channel security](../../src/team-security.test.ts): approval per member and
  key, written before use; migration from files without approvals; a key seen
  again needs approval; paused input survives restarts; corrupt approvals are
  rejected without regenerating the identity.
- [Owner input](../../src/team-owner.test.ts): the portable core without Tauri
  discards input from a changed key while snapshots still flow, from a member
  first seen under `audience: null` and from a new device of the owner's
  person despite remote control. It needs Send messages for prompts and
  interrupts and Control for answers, even when the whole organization views,
  and lets shares from before rights only view and comment. It also covers
  approval through the notice, remote control and grants, restarts, spent
  replay receipts and approvals dropped once their connection is gone.
- [Rights](../../src/team-rights.test.ts): the right each frame needs, tolerant
  parsing and what a viewer may do from an announcement, an older owner or its
  own devices.
- [Encrypted channel](../../src/team-channel.test.ts): rights travel inside the
  authenticated announcement and never in the relay's view; an announcement
  without them reads as an older owner. The
  [browser core](../../src/team-member.test.ts) reflects them for a viewer.
- [Organizations](../../src/team-organizations.test.ts): through the desktop
  facade, companion input keeps its authorship, and a colleague who views acts
  only once granted Send messages.
- `share_tests` in `src-tauri/crates/core/src/workspace_lifecycle.rs` and
  `fixtures/backend-contract.json`: boards from before rights load with
  `rights: null`, and new consent records and clears them.
- [Critical flows](../../e2e/critical-flows.spec.ts): content keeps flowing
  after a peer reinstalls, without a review.
