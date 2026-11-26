---
type: Decision
title: "ADR 0022 — end-to-end encryption in collaboration"
description: "Collaboration content has end-to-end encryption."
status: stable
---

# ADR 0022 — end-to-end encryption in collaboration

Date: 2026-09-07

This ADR specifies the encrypted content boundary for [ADR
0021](0021-cloud-organizations.md). Publication is a different step from the
local implementation and the tests.

## Context

TLS stops at the relay. The v3 protocol let the operator read these items: the
conversation, remote messages, comments, quoted text, inbox previews and titles.
The product selected automatic configuration. The client trusts the directory of
the server at the first contact (trust on first use, TOFU). It is not necessary
for the person to compare codes.

## Decision

The [v4 protocol](../contracts/relay-v4.md) encrypts the content on the device
for each recipient in the audience. It uses HPKE Auth from [RFC
9180](https://www.rfc-editor.org/rfc/rfc9180.html): P-256, HKDF-SHA256 and
AES-256-GCM. We use `@hpke/core` at the constant version 1.9.0, on WebCrypto. We
do not write the primitives or the KEM. Prometeu owns the content format, TOFU,
authorization, replay control and storage. These items must have their own
review.

Each local scope makes a private identity. This identity is not related to
tickets and enrollment credentials. The relay receives the public key and an
ECDSA proof of possession. The proof is bound to the member and to the challenge
of that connection. Clients pin the key of each member before they send content.
When the key of a peer changes, the client stores and uses the new key
automatically. There is no stop, review, comparable code or signed change,
as [ADR 0042](0042-automatic-key-rotation.md) specifies. If the client
cannot write the new key, it does not use the new key. The client continues to
reject a replacement of the own key of the scope.

The scope includes the origin, the organization or team and the membership. The
storage also keeps local Cloud accounts isolated. A new ticket, a new connection
or a new start of the app does not delete links. Each relay member has one
active identity. The first Mac keeps the membership identity. Browsers and other
Macs connect as companion members with their own keys under [ADRs
0027](0027-companion-devices.md) and [0036](0036-second-mac-as-companion.md).
Devices do not copy private keys to other devices. They do not automatically get
previous comments for a different identity.

The owner continues to be the authority for the process and the local audience.
`watch` does not give access. Authenticated audience announcements have stored
revisions. The relay cannot replace the owner or put back a revision that the
client replaced before.

Remote messages have a deadline of two minutes. The client stores a receipt
before it operates the message. This also applies after a new start of the app.
A remote message operates only for persons who have the related right in that
workspace. It must also come from a member and key with the approval of the
owner on that Mac ([ADR 0090](0090-approved-remote-input.md)). After decryption,
the validation of `chat_control_remote` against the open requests on the Mac of
the owner stays.

## Alternatives and costs

Encryption at the relay or with the enrollment secret does not give protection
against the operator. We thought about MLS for a group ratchet. But MLS adds
group state, the supply of commits and offline recovery, and the current
transport does not have these items. HPKE Auth gives each stored comment its own
envelope. It also lets a new connection operate without stored state. Senders do
not share state.

The cost is one encrypted copy for each recipient, up to 64 members. There is no
forward secrecy and no automatic recovery after a compromise. If a person gets
the private key of a recipient, that person can open previous ciphertext for
that key. This implementation is not equivalent to the Signal Protocol or
WhatsApp. To add a ratchet, the team must make a new version of the contract, a
migration and a review of the cryptographic protection. A different cipher alone
is not sufficient.

## Security limits

- A malicious server can replace keys at the first contact and after it. Since
  ADR 0042, the client uses the changed key for content without a message. Thus
  the server can read the new content of that person. There is no external
  procedure to compare codes and no key transparency.
- Remote input operates actions. It does not only show data. By default, agents
  operate tools without approval for each tool, and `request.respond` answers
  the human gate. Thus messages and control operate only from pairs of member
  and key with the approval of the owner on the Mac ([ADR
  0090](0090-approved-remote-input.md)). Some senders continue to receive
  content, but the client ignores their input until the owner gives approval.
  These senders are a changed key, a new device (also a device of the owner) and
  a member that is new in the directory. The approval also trusts the keys that
  the directory shows at that time.
- The relay knows the organization, members, names, workspace and tab IDs,
  recipients, mentions, online status, timestamps, data quantities and terminal
  dimensions. It can ignore messages, send them late or change their sequence,
  and it can stop the service. The cipher does not authenticate display names,
  delivery, the completeness of the history or timestamps.
- The owner stops new content for a removed person through the local audience.
  Comments from peers use the last authenticated announcement that the peers
  received. If a relay does not send the change, the peers can apply it late.
  There is no guarantee that revocation occurs immediately on all devices.
- When a person opens a shared session, the Mac sends the complete snapshot that
  it keeps. This agrees with the current consent to share the conversation. A
  stored comment opens only with a box for that identity. New participants do
  not automatically get boxes for previous comments.
- It is not possible to revoke content that a client received before. The client
  does not show items from authors that are not in the current audience. It also
  does not show items from replaced keys or history without a box that it can
  read.
- E2EE does not give protection for a compromised Mac or webview, copies of
  private keys or content that goes to the providers. Local transcripts keep
  their format. Data that the client sent in v3 does not become private after
  this change.
- Local tests are not a cryptographic audit by an external team.

## Compatibility and publication

The client must have v4 and `e2ee: 1`. There is no fallback to plaintext. Legacy
credentials and enrollments stay applicable. The Worker reads and writes
collaboration data only under `v4:`. The v3 rows stay, but the v4 client does
not show them. There is no automatic conversion and no viewer for v3 comments.

Publish relay v4 before you supply desktop v4. Local work continues if the
negotiation has a failure. A rollback must keep the two namespaces and the
private file with the keys and the TOFU links. A v3 relay or desktop gives the
plaintext guarantees of v3 again. Do not call it a rollback that keeps E2EE. Do
not publish releases and do not migrate production during the local validation.

## Evidence

- `src/team-crypto.test.ts` has tests for encryption without a mock,
  authentication, context, changed ciphertext and key validation.
  `src/team-security.test.ts` has tests for TOFU, storage and failures.
- `src/team-channel.test.ts` has tests for two clients, snapshot and live
  data, messages, comments, inbox, replay, audience and a rejected downgrade.
- `src/team.test.ts` and `src/team-organizations.test.ts` have tests for the
  transport of the app, encryption, scopes and new connections.
  `relay/src/worker.integration.test.ts` has tests for a local Worker,
  ciphertext, identity, audience and storage.
- `src-tauri/src/team.rs` has tests for the private file, atomic write
  operations and corruption. `e2e/critical-flows.spec.ts` has tests for comments
  with encrypted peers in Chromium and WebKit.
- The [provider matrix](../quality/provider-matrix.md) gives the same
  protection for Claude and Codex. Publication and an external audit are not
  part of this evidence.
