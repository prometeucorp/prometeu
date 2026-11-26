---
type: Decision
title: "ADR 0090 — Remote input only from approved identities, with separate rights"
description: "Remote input comes only from member and key pairs that the owner approved on the Mac, with different rights to see, send messages and control."
status: stable
---

# ADR 0090 — Remote input only from approved identities, with separate rights

Date: 2026-10-08

This ADR decreases the trust that [ADR 0022](0022-end-to-end-encryption.md)
and [ADR 0042](0042-automatic-key-rotation.md) give to the key of a peer when
that key sends input. It adds to the remote control of
[ADR 0030](0030-remote-control.md). It also adds to the companion devices of
[ADRs 0027](0027-companion-devices.md) and
[0036](0036-second-mac-as-companion.md). The implementation came in three
changes. See [Rollout](#rollout).

## Context

Before this decision, a person in the audience of a shared workspace got three
rights together:

- to see the conversation and write comments
- to send messages that run as prompts in the agent of the owner
- to control the agent with `turn.interrupt` and `request.respond` (accept,
  reject or answer)

The devices of the owner must also have `remote_control`. Teammates must have
only the audience, and `audience: null` includes each member with a key.

By default, agents run tools without an approval for each tool. They have the
access of the local user to files and processes. Thus a remote prompt is an
execution on the Mac of the owner. A remote `request.respond` answers the
approvals, which are there as a human gate. ADRs 0022 and 0042 accepted a key
that the server replaced as a cost to confidentiality. With that key, the
server can read the new content of that person. For input, the same key authorizes commands.

The relay directory also tells which persons are members and which person owns
each device (`Member.person`). Thus input in these three conditions got to the
agent without an action from the owner:

- a changed key of a known member
- a member that shows in the directory for the first time, through
  `audience: null`
- a new device with the owner as its `person`, through `remote_control`. The text
  of this device also comes without the "Message from" prefix.

The Mac is not part of device pairing. Phones and other Macs get
companion tickets from the Cloud. The Mac gets their keys only from the relay
directory. A malicious server controls this same source.

## Options considered

1. Continue to trust the directory for input. This adds no work for the
   person. But the relay operator, or an attacker who controls the relay, can
   run commands on the Mac of the owner.
2. Stop input only after a pinned key changes. This does not include new
   members and new devices of the person of the owner. The directory also
   controls these.
3. Run input only from member and key pairs that the owner approved on the
   Mac. Keep the rights to see and the rights to do actions apart for each
   workspace. Content keeps the automatic adoption of ADR 0042.
4. Codes that persons compare, or key transparency. These also keep content
   safe. But persons must then compare keys manually again, and ADR 0042
   removed that step. This
   stays future work.

## Decision

Prometeu selected option 3.

### Approved identities

- **Rule.** The owner runs remote input (prompts, `turn.interrupt` and
  `request.respond`) only from a member if the owner approved the current key of
  that member on that Mac. Viewing and comments (`watch`, snapshots, the live stream and
  comments) continue to follow ADR 0042. Thus content continues to move while
  input waits.
- **Scope.** An approval is a member and key pair. Prometeu stores it for each
  scope in `team-security.json`. When the owner gives approval to a device, Prometeu
  releases its input in each workspace of the scope where its person has the right. Each
  member has one approved key. These are not approved: a changed key, an
  expired key that comes back, a new member and a new device. The identity of the owner
  is always approved.
- **Approval moments.** Each moment gives approval to the current keys of all
  the devices of the person. It tells how many keys it approved. Thus the owner
  can see an unknown device. These are the moments:
  - when the owner turns on remote control, for the devices of the owner
  - when the owner gives Send messages or Control to a person
  - when the owner answers the notice below

  Pairing does not give approval to keys, because the Mac is not part of it.
  "Everyone in this organization" and a mention in a comment give approval to
  no person.
- **Blocked input.** The channel decrypts the message and uses its replay
  receipt as before. Then the owner discards the input and does not run it.
  Thus the input cannot run at a subsequent time. The Mac shows a notice above
  the composer of each conversation in the workspace that got the input. This
  includes desk tiles. The notice is "One of {name}'s devices changed or is
  new. Allow its input again?", with Allow and Not now. For the devices of the
  owner, it shows "One of your devices". The notice names the person, not the
  device ([ADR 0041](0041-members-list-shows-people.md)). Prometeu stores it
  without content. It stays after the app starts again. After Not now, it comes back with
  the next discarded input. The sender does not get a receipt.
- **Writes.** Prometeu writes approvals and paused input on the encrypted queue
  of the connection, as it does for receipts and links. Thus a new connection
  or a change of organization does not load the store again while a write from
  the previous connection continues. An approval applies to the connection where
  the owner did the action. If the owner changes the organization while
  Prometeu saves the consent, the approval does not apply to the next
  organization. If the owner answers while a new connection loads the store
  again, the answer has no effect. Then the notice stays, and the owner can
  answer again.

### Rights per workspace

| Right | Operations | Teammates | Owner's devices |
| --- | --- | --- | --- |
| View and comment | watch, snapshots, live stream, comments | the audience: selected persons or the full organization | with `remote_control` |
| Send messages | prompts and `turn.interrupt` | given to each person | with `remote_control` |
| Control | `request.respond`: approvals and answers | given to each person | with `remote_control` |

Prometeu does not give Send messages and Control to the full organization.
These rights do not expand from `audience: null`. An interruption stops work
but cannot start work. Thus it stays with Send messages.

- **Board.** `Workspace.rights` contains `{ send, control }`, two lists of
  persons, adjacent to `audience` and `remote_control`. The two lists name only
  persons who can view. If the owner gives a right to a person who is not in an
  explicit audience, Prometeu adds that person to the audience. If the owner
  removes a person from the audience, Prometeu removes the rights of that
  person. A change to a right also removes the persons who left the
  organization. Thus a list is not larger than the directory, and peers read
  each right that the owner gave.
- **Enforcement.** The owner examines the right before the approval. If the
  person does not have the right, Prometeu discards the input without a notice,
  because no approval can help. Only a person with the right can cause
  the notice above.
- **Share menu.** It continues to select the persons who view and write
  comments. It adds Send messages and Control submenus, which list only the
  persons who view. When the owner selects a person for the audience or
  writes a mention of the person, this gives no approval.
- **Conversation header.** A chip adjacent to the viewers names the persons who
  can do actions. Its title lists each right. Remote conversations show the persons
  that their owner announced.

### Migration

- After the upgrade, the keys that Prometeu pinned in `peers` before become
  approved. A scope without approvals gives approval to its links one time. Teams and
  the devices of the owner with `remote_control` continue to operate.
- Current shares become View and comment. They load with `rights: null`, which
  gives no right. Each share that the owner saves after that records its
  rights. The owner sees a notice one time above the conversations of those
  shares. The notice tells about the new rights. "Got it" closes it on that Mac.

### Owner confirmation

- This option is on by default. It is an option for each workspace: "Confirm
  each message before it runs" in the Send messages submenu.
  `Workspace.confirm_messages` contains the selection. Boards from before this
  option load with the confirmation on. When the owner stops the share, the
  selection stays.
- Only a message from a teammate waits. Interruptions and answers must have their own
  right and an approved device at this time. The devices of the owner do
  actions for the owner. Thus these run immediately.
- The message becomes a pending item above the composer of its conversation.
  This includes desk tiles. The item shows as an alert card with Send, Edit and
  Discard. It names the person, not the device.
- The pending item is only in memory while the app operates. It stays after a
  connection stops, but not after a change of organization or after the app
  stops. There is no offline inbox. Prometeu does not write the content of a
  third party to the disk.
- Send keeps the "Message from {name}" prefix and operates as the composer
  does. The item goes away immediately. If a failure occurs, Prometeu only
  reports it. The cause is that a stopped agent can put the text in its queue
  before a failure occurs when it starts again. Edit stops dictation, if it operates. It moves the
  text into the draft of the owner, after the text that the owner typed before.
  When the owner sends that text, it is a message from the owner.
- Prometeu discards the pending item in these conditions:
  - its workspace or conversation goes away, or the owner stops its share on
    this Mac
  - the sender does not have Send messages after a change

  When the owner turns off the option, Prometeu does not release the messages
  that wait.
- A maximum of 20 messages wait for each conversation. If more messages come,
  Prometeu discards the oldest messages first.

### Announcement and phone

- The rights go in the authenticated share announcement as a new `rights`
  field. This field is adjacent to `revision` in the encrypted payload.
  `relay/src/protocol.ts` does not change, because `parseShare` removes unknown
  fields in the share.
- Peers do not show the controls that they do not have. On the desktop, a
  viewer without Send messages gets no composer, and thus no Stop button. A
  line tells the cause. Without Control, request cards show that they wait for
  an answer. The owner makes peers obey the rights also when a peer ignores the
  announcement.
- An announcement without `rights` comes from an owner from before this change.
  Thus peers keep the controls of that owner.
- The phone shows the rights only in its current controls. It does not show the
  composer without Send messages. It does not get approval buttons.

## Rollout

The decision came in three changes, in this sequence:

1. **Approved identities.** Complete: the rule, the notice, the approval
   moments and the migration of approvals.
2. **Rights per workspace.** Complete: View and comment, Send messages and
   Control. The board, the share menu and the conversation header contain
   them. The announcement, the desktop and phone controls, and the migration
   also apply them.
3. **Owner confirmation.** Complete: the option for each workspace, the
   pending item with its buttons, and the discard rules.

Each change updates this section and the contracts when it is complete.

## Consequences

- After a new installation or on a new device, the owner must answer one time
  before its messages run again. Prometeu discards the message that showed the
  change. In this version, its sender gets no information about this.
- A key that the server replaced without a message can continue to read new
  content (ADR 0042). But it cannot run commands on the Mac of the owner.
- The approval keeps trust on first use at the time of the approval. An approval of a person applies to all the keys that the directory
  shows for the devices of that person at that time. The count of devices is
  the only sign of an unknown device.
- The migration trusts links that the server can have replaced before, as ADR
  0042 accepts.
- A new Mac gives approval to no person until the owner gives a right, turns on remote
  control or answers a notice.
- After the upgrade, teammates who sent prompts to a shared agent cannot do
  this until the owner gives them Send messages or Control. Prometeu discards
  their messages without information to them. The notice that shows one time
  tells this to the owner.
- With the confirmation on, a message from a teammate runs only after the owner
  sends it. If the app stops while a message waits, Prometeu discards that
  message. Its sender gets no receipt in the two conditions.
- `team-security.json` gets the new `approved` and `paused` fields for each
  scope. A version without these fields removes them at its next write. A new
  upgrade gives approval to the links that are there at that time. The relay protocol,
  the receipts and the Rust validation do not change.
- `board.json` gets `confirm_messages` for each workspace, and IPC gets
  `set_confirm_messages`. A previous version removes the field at its next
  board write. A new upgrade turns the confirmation on again.

## Evidence

- [Channel security](../../src/team-security.test.ts): an approval for each
  member and key, which Prometeu writes before use. The migration from files
  without approvals. A key that comes back must have an approval. Paused input
  stays after the app starts again. Prometeu rejects corrupt approvals and does not make a
  new identity.
- [Owner input](../../src/team-owner.test.ts): the portable core without Tauri
  discards input in three conditions. Snapshots continue to move in these
  conditions:
  - a changed key
  - a member that shows for the first time under `audience: null`
  - a new device of the person of the owner, also with remote control

  Prompts and interruptions must have Send messages, and answers must have
  Control, also when the full organization views. Shares from before the
  rights can only view and write comments. The tests also examine these items:
  - approval through the notice, remote control and rights
  - new starts of the app and used replay receipts
  - approvals that Prometeu discards when their connection stops. This includes
    a change of organization while Prometeu saves the consent, and an answer
    during a new connection.
- [Rights](../../src/team-rights.test.ts): the right that is necessary for each frame, the
  parser that accepts unknown values, and rights that remove persons who left
  the organization. The tests also examine what a viewer can do from an
  announcement, from an older owner or from its own devices.
- [Encrypted channel](../../src/team-channel.test.ts): the rights go in the
  authenticated announcement. The relay does not see them. An announcement
  without them comes from an older owner. The
  [browser core](../../src/team-member.test.ts) shows them for a viewer.
- [Owner confirmation](../../src/team-owner.test.ts): through the portable core,
  a message from a teammate waits. Send keeps the prefix. Edit gives back only
  the text. Discard removes it. If a send has a failure, Prometeu reports it and does not
  keep the message again. Prometeu discards the message with its conversation, its
  workspace, its share or the right of the sender. The message stays after a
  connection stops, but not after a change of organization. If more messages
  come, Prometeu keeps the latest 20. Interruptions, answers and the devices of
  the owner run immediately. Boards from before the option keep messages.
  No data goes into the security store.
- [Organizations](../../src/team-organizations.test.ts): through the desktop
  facade, companion input keeps its author. A colleague who views can do actions only
  after the owner gives Send messages, and after the owner sends the message
  that waits.
- `share_tests` in `src-tauri/crates/core/src/workspace_lifecycle.rs` and
  `fixtures/backend-contract.json`: boards from before the rights load with
  `rights: null`. New consent records them and removes them. Boards from before
  the owner confirmation load with it on. The selection stays after the share
  stops, and the fixture contains the two values.
- [Critical flows](../../e2e/critical-flows.spec.ts): content continues to
  move after a peer installs again, without a review.
