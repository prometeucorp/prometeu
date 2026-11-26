---
type: Decision
title: "ADR 0042 — A peer's new key is adopted automatically"
description: "The application uses the new key of a peer automatically."
status: stable
---

# ADR 0042 — A peer's new key is adopted automatically

Date: 2026-09-10

This ADR specifies how a new key of a peer replaces the previous key. The cipher,
the scopes, the authorization, the replay protection and the limits stay as
[ADR 0022](0022-end-to-end-encryption.md) specifies them. The list of members in
[ADR 0041](0041-members-list-shows-people.md) has no controls to examine keys.

## Context

The previous policy kept the first key of each member. It stopped the content
until a person accepted a replacement key in Settings. But the key changes for
usual causes:

- A person installs Prometeu again.
- A person formats the Mac.
- A person changes to a different phone.
- A person pairs a different device.

Each of these changes stopped the shared content with that person for all
peers. Each peer then had to open Settings and accept the new key. The persons
who had this problem learned to click "Accept" and did not examine the key. Thus
the block had a high cost and gave no real security.

## Options considered

1. Keep the block and make the text better.
2. Use the new key and show a passive notice that stays in Settings.
3. Use the new key and show no notice.

## Decision

Adopt option 3. `observe()` writes the new key over the previous link before
the application uses the key. The content continues to go to the peer, and a
person does not do an action. The application writes the key before it uses the key. If
the save fails, the new key is not available, and the application encrypts
nothing for it.

These items leave the product:

- The block on a changed key.
- The review of the key in Settings.
- The explicit acceptance of the key.
- The security code for each member. ADR 0041 removed it before.

These items stay:

- The scope keeps its own identity.
- A change to the own identity key of the scope continues to fail with
  `Own identity key changed`.
- A missing key or a directory that is not valid continues to stop the
  application from using the link.
- Receipts, revisions of announcements and the validation of content do not
  change.

## Consequences

When a person installs the app again, changes devices or formats the machine,
the team continues to operate. No screen asks a person to confirm a key.

The cost is clear. Without a manual acceptance, the relay can replace the key of
a member. It can then read the new content of that person, and the interface
shows no signal. ADR 0022 accepted this replacement at first contact, and it
accepted that there is no key transparency. Now this risk also applies to
changes after the first contact. A person who must verify a key outside the
application must wait for a future feature, for example a code to compare or key
transparency. A block that nobody reads does not give this verification.

The automatic replacement of the key applies only to content. Input from a peer is
an operation on the Mac of the owner. By default, agents run tools without an
approval for each tool. `request.respond` answers the approvals that the agent
asks for, and a person must give these approvals. Since
[ADR 0090](0090-approved-remote-input.md), a replaced key continues to get
content automatically. But the Mac ignores messages and control from that key
until the owner approves that device on the Mac. Thus a hidden replacement
can show new content to the relay, but it cannot run commands.

The `peers` field of `team-security.json` continues to store one key for each
member. The application writes over this key when it changes. ADR 0090 adds the
input approvals of the owner adjacent to it. The links keep their meaning.

## Evidence

- [Channel security](../../src/team-security.test.ts): the new key replaces the
  link and becomes active only after the save. The own identity continues to
  refuse a change.
- [Encrypted channel](../../src/team-channel.test.ts): the application continues
  to encrypt content for the peer whose key changed.
- [Critical flows](../../e2e/critical-flows.spec.ts): when the device of a peer
  installs the application again, the application does not ask for a review,
  and the collaboration continues.
