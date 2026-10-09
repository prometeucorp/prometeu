# ADR 0042 — A peer's new key is adopted automatically

Date: 2026-09-10
Status: Accepted

Defines peer key replacement within the cipher, scopes, authorization, replay
and limits of [ADR 0022](0022-end-to-end-encryption.md). The members list in
[ADR 0041](0041-members-list-shows-people.md) has no key-review controls.

## Context

The previous policy pinned each member's first key and blocked content until
someone accepted a replacement in Settings. In practice the key changes for
common reasons: reinstalling Prometeu, formatting the Mac, switching phones,
pairing another device. Each of these events cuts sharing with that person for
every peer and requires each one to open the Settings and accept. Whoever lives
with this learns to click "Accept" without checking anything — the block costs a
lot and buys no real security.

## Options considered

1. Keep the block and improve the copy.
2. Adopt the new key and show a persisted passive notice in the Settings.
3. Adopt the new key silently.

## Decision

Option 3. `observe()` writes the new key over the previous link before using it,
and content keeps flowing without intervention. The write still comes before
use: if persistence fails, the new key is not available and nothing is
encrypted for it.

The block on a changed key, the review in the Settings, the explicit acceptance
and the per-member security code (already removed in 0041) leave the product.

What remains: the scope keeps its own identity; a change of the scope's own
identity still fails with `Own identity key changed`; a missing key or an
invalid directory still prevent using the link; receipts, announcement revisions
and content validation stay the same.

## Consequences

Reinstalling the app, switching devices or formatting the machine no longer
interrupts the team. There is no screen asking to confirm a key anymore.

The cost is explicit: without manual acceptance, the relay can replace a
member's key and read that person's new content with no visible signal in the
interface. 0022 already admitted this replacement at first contact and the
absence of key transparency; now it also applies to later changes. Whoever needs
external verification depends on a future feature — a comparable code or key
transparency — and not on a block nobody reads.

The silent adoption covers content only. Input is execution on the owner's Mac:
agents run tools without per-tool approval by default, and `request.respond`
answers the approvals that exist as a human gate. Since
[ADR 0090](0090-approved-remote-input.md), a replaced key keeps receiving
content without intervention, but messages and control from it are discarded
until the owner approves that device on the Mac. The cost of a silent
replacement is the disclosure of new content, not the execution of commands.

The `peers` field of `team-security.json` still stores one key per member and
is overwritten on a change. ADR 0090 adds the owner's input approvals next to
it; the links keep their meaning.

## Evidence

- [Channel security](../../src/team-security.test.ts): the new key replaces the
  link, only takes effect after persisting, and the own identity still refuses a
  change.
- [Encrypted channel](../../src/team-channel.test.ts): content is still
  encrypted for the peer whose key changed.
- [Critical flows](../../e2e/critical-flows.spec.ts): reinstalling the peer's
  device does not ask for a review and collaboration continues.
