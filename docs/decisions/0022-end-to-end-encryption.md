# ADR 0022 — end-to-end encryption in collaboration

Date: 2026-09-07
Status: Accepted

Defines the encrypted content boundary for
[ADR 0021](0021-cloud-organizations.md). Publication is a separate operational
step from local implementation and tests.

## Context

TLS terminates at the relay. The v3 protocol exposed the conversation, remote
messages, comments, quotes, inbox previews and titles to the operator. The
product choice is automatic configuration: trusting the server's directory on
first contact (trust on first use, TOFU), without mandatory code comparison.

## Decision

The [v4 protocol](../contracts/relay-v4.md) encrypts content on the device for
each recipient in the audience, with HPKE Auth from
[RFC 9180](https://www.rfc-editor.org/rfc/rfc9180.html): P-256, HKDF-SHA256 and
AES-256-GCM. We use `@hpke/core`, pinned at 1.9.0, over WebCrypto. We do not
implement the primitives or the KEM. The content format, TOFU, authorization,
replay control and persistence belong to Prometeu and require their own review.

Each local scope generates a private identity, independent of tickets and
enrollment credentials. The relay receives the public key and an ECDSA proof of
possession bound to the member and to that connection's challenge. Clients pin
each member's key before sending content. A changed peer key is persisted and
adopted automatically, without a block, review, comparable code or signed
transition, as defined in [ADR 0042](0042-automatic-key-rotation.md). A write
failure prevents use of the new key. Replacing the scope's own key still fails.

The scope includes the origin, the organization/team and the membership;
persistence also separates local Cloud accounts. Renewing a ticket,
reconnecting or restarting does not erase links. There is one active identity
per relay member. The first Mac keeps the membership identity; browsers and
additional Macs join as companion members with their own keys under
[ADRs 0027](0027-companion-devices.md) and
[0036](0036-second-mac-as-companion.md). Devices do not synchronize private keys
or automatically recover old comments addressed to another identity.

The owner is still the authority over their process and the local audience.
`watch` grants no access. Authenticated audience announcements have persisted
revisions; the relay cannot replace the owner or restore a revision already
superseded in the client. Remote messages have a two-minute deadline and a
receipt persisted before execution, including after a restart. They run only
for people granted the matching right in that workspace and only from a member
and key the owner approved on that Mac
([ADR 0090](0090-approved-remote-input.md)). After decryption, the validation
of `chat_control_remote` against requests open on the owner's Mac remains.

## Alternatives and costs

Encrypting at the relay or using the enrollment secret does not protect against
the operator. MLS was considered for group ratcheting, but it introduces group
state, commit distribution and offline recovery that the current transport does
not have. HPKE Auth allows independent envelopes for persisted comments and
stateless reconnections, without shared state between senders.

The cost is one encrypted copy per recipient, up to 64 members, and the absence
of forward secrecy and of automatic post-compromise recovery. Stealing a
recipient's private key allows opening old ciphertext recorded for that key.
This implementation is not equivalent to the Signal Protocol/WhatsApp. Adding a
ratchet requires another version of the contract, a migration and a security
review; swapping the cipher is not enough.

## Security limits

- A malicious server can replace keys, at first contact and afterwards: since
  ADR 0042 the change is adopted silently for content, so the server can read
  that person's new content. There is no external code comparison and no key
  transparency.
- Remote input is execution, not only disclosure: agents run tools without
  per-tool approval by default and `request.respond` answers the human gate.
  Messages and control therefore run only from member and key pairs approved on
  the owner's Mac ([ADR 0090](0090-approved-remote-input.md)). A changed key, a
  new device, including the owner's own, and a member that first appears in the
  directory keep receiving content, but their input is discarded until the
  owner approves them. Approval itself still trusts the keys the directory
  shows at that moment.
- The relay knows the organization, members, names, workspace/tab IDs,
  recipients, mentions, presence, timestamps, sizes and terminal dimensions. It
  can omit, delay or reorder messages and deny service. The cipher does not
  authenticate display names, delivery, history completeness or timestamps.
- The owner stops new content for a removed person according to their local
  audience. Comments sent by peers use the last authenticated announcement they
  received. A relay that omits the change may delay its application on those
  peers; there is no guarantee of instant global revocation.
- Opening a shared session sends the complete snapshot the Mac keeps, according
  to the existing consent to share the conversation. Persisted comments open
  only with a box addressed to that identity. New participants do not
  automatically gain boxes for old comments.
- Content already received cannot be revoked. Authors outside the current
  audience, replaced keys or history without a readable box are omitted from
  reading.
- E2EE does not protect a compromised Mac/webview, backups of private keys or
  content sent to the providers. Local transcripts keep their format. Data sent
  in v3 does not become retroactively private.
- Local tests do not constitute an independent cryptographic audit.

## Compatibility and publication

The client requires v4 and `e2ee: 1`; there is no fallback to plaintext. Legacy
credentials and enrollments stay valid. The Worker now reads/writes
collaboration only under `v4:`. v3 rows stay intact, but do not appear in the v4
client. There is no automatic conversion and no v3 comment viewer.

Publish relay v4 before distributing desktop v4. Local work continues if the
negotiation fails. A rollback must preserve both namespaces and the private
security file. Restoring a v3 relay/desktop goes back to v3's plaintext
guarantees and cannot be announced as a rollback that keeps E2EE.
Do not publish releases and do not migrate production during the local
validation.

## Evidence

- `src/team-crypto.test.ts`: real encryption, authentication, context, tampering
  and key validation; `src/team-security.test.ts`: TOFU, persistence and
  failures.
- `src/team-channel.test.ts`: two clients, snapshot/live, messages, comments,
  inbox, replay, audience and downgrade rejection.
- `src/team.test.ts` and `src/team-organizations.test.ts`: the app's transport,
  encryption, scopes and reconnection; `relay/src/worker.integration.test.ts`: a
  local Worker, ciphertext, identity, audience and persistence.
- `src-tauri/src/team.rs`: the private file, atomic writing and corruption;
  `e2e/critical-flows.spec.ts`: comments with encrypted peers in
  Chromium/WebKit.
- The [provider matrix](../quality/provider-matrix.md) declares the same
  protection for Claude and Codex. Publication and an external audit are not
  part of this evidence.
