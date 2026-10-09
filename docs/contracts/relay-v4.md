# Relay protocol v4

Status: current contract; decision in
[ADR 0022](../decisions/0022-end-to-end-encryption.md).
Executable source: `relay/src/protocol.ts`, imported by the app and the Worker.
The retired v3 specification is available in Git history. Compatibility with
its stored data is described below; v3 connections are not supported.

## Boundary and authentication

The webview encrypts and decrypts; Rust persists the identity privately and runs
only authorized actions. The relay forwards ciphertext and keeps audience,
quotas, presence, comments and inbox. The agent stays on the owner's Mac.

`pm2` enrollment, individual credentials and Cloud tickets keep their formats.
The WebSocket requires `p=4`. `welcome` announces `e2ee: 1`, `comments: 1` and a
random per-socket challenge. Before any other send, the client answers
`identity { key, proof }`. The proof is ECDSA P-256/SHA-256 over the UTF-8 JSON
`["prometeu-identity-v4", member, challenge]`, using the private identity.
The Worker verifies possession before publishing `Member.key` or allowing
traffic. Presence with the confirmed key ends the handshake. The Worker does not
send other updates to a socket that has not identified itself yet.

The client records each member/key link before use. A changed key replaces the
link automatically, with no local acceptance ([ADR 0042](../decisions/0042-automatic-key-rotation.md)). Missing keys receive no
content and do not erase links. Renewing tickets does not reset TOFU. Content
follows the link; remote input additionally needs the owner's approval of that
member and key, described under [Authority and replay](#authority-and-replay).
See [organizations](cloud-organizations.md) for leases and Cloud authorization.

After the identity, organization sockets receive `lease { expires_in }` (1 to
60,000 ms). `renew { ticket }` consumes another 256-bit Cloud ticket, encoded in
43 base64url characters, and requires the original organization and member. The
relay refuses renewal before the identity, after expiration or with an invalid,
consumed or foreign-membership ticket. A renewal preserves the key, challenge,
watchers and streaming; an identical roster generates no presence. The
confirmation is another `lease`. These are additive controls in v4: old clients
ignore the capability, and new clients keep reconnecting when an old relay does
not announce it.

## Envelope and content

`Encrypted = { id, boxes: { [member]: { enc, ct } } }`. IDs are random; `enc` and
`ct` use canonical base64url without padding. Each box uses HPKE Auth,
DHKEM(P-256, HKDF-SHA256), HKDF-SHA256, AES-256-GCM. The `info` binds the JSON
`["prometeu-e2ee-v4", scope, author, recipient, id]`. `scope` is the JSON of
`["organization", cloudOrigin, organizationId]` or
`["team", relayOrigin, teamId]`. The author's key comes from the local link,
never from a free field of the envelope.

The plaintext contains `{ frame: Up }` or `{ binary: base64url }`. `share`
includes `revision`, monotonic and persisted by the owner, and `rights`
`{ send, control }`, the people allowed to send messages and to control
([ADR 0090](../decisions/0090-approved-remote-input.md)). Rights travel only
inside the authenticated payload, never in the outer frame, so
`relay/src/protocol.ts` does not carry them. Older clients ignore the field; an
announcement without it comes from an owner that predates rights, and peers keep
showing its controls. `write` includes `expires`, at most two minutes ahead. The
client validates the payload again after opening the box and checks the
workspace, tab, author, recipient and audience.

In the outer frame:

- `share`: titles, repository, branch and stage empty; issue null; tab text and
  tokens null/empty and a constant status. IDs, the active tab, dimensions and
  the explicit audience stay visible. `audience: null` is expanded to members
  with an available key before encrypting.
- `note`/`note_reply`: empty text; quotes and anchors null. The persisted ID is
  the envelope's ID, unique within the team. Mentions and parentage stay
  visible.
- `note_resolve`: the envelope authenticates the workspace and the root. Storage
  keeps the original ciphertext and adds `resolution: { author, encrypted }`.
- `write`: empty data, envelope for the owner only.
- Binary: the envelope travels through an outer unicast `SNAPSHOT` frame, with
  sequence zero and `more=false`. The box contains the complete original binary
  frame, including kind, sequence and destination. The recipient accepts only
  its own attached tab.

`room.ts` refuses cleartext content fields, invalid envelopes and legacy binary.
Domain parsers still accept cleartext payloads for internal validation after
decryption; that does not authorize the Worker to receive them on the socket.
`downForMember` sends only that member's box, including in the `welcome`,
`notes` and `inbox` aggregates. There are no private keys in the relay.

## Persistent comments

Comments use flat threads: `note` creates a root, `note_reply` replies to an
open root and `note_resolve` resolves it. Replies inherit the workspace and
tab. `notes` returns a workspace snapshot; a downstream `note` upserts by ID,
including a resolved root. The encrypted payload carries text, quote, tab and
the optional `Piece.key` anchor; a quote provides readable context if the
excerpt is unavailable and grants no authority.

Mentions create inbox assignments to the root. Opening an assignment navigates
to the thread; resolving the root removes its assignments for everyone. Any
collaborator with workspace access can resolve it. The client retains the
`inbox_read` fallback when `comments: 1` is absent, but still requires the v4
E2EE handshake. Current relays advertise thread support.

Missing `tab`, `anchor`, `parent` and `resolved` fields normalize to a general,
open root in the domain parser. This does not import v3 storage or permit
plaintext on the socket. Thread structure and retention remain visible to the
relay; content and resolution authenticity use the envelope described above.

## Authority and replay

The owner uses their board to authorize the snapshot, the live stream and a
received message. An announcement echoed by the relay does not change the local
audience. For remote shares, the client persists the owner, key, revision and ID
of the last accepted announcement; it rejects an owner change and old revisions,
while allowing the same announcement to be repeated on reconnection. After a
persisted automatic adoption of a new peer key, that owner's sequence may
restart. The owner ID cannot change.

Remote messages persist an ID and a deadline before executing. A repetition,
expiration, write failure and a clock earlier than the last consumption block
the action. The receipts survive reconnection and restart. Local card validation
is still mandatory. Conversation sequences handle snapshot/live duplicates; the
relay may still omit content or present an incomplete history.

Remote input needs a right on the sender's person, enforced by the owner from
its board whatever the peer shows: prompts and `turn.interrupt` need Send
messages, and `request.respond` needs Control. The owner's devices hold both
through `remote_control`. Input without the right is discarded silently.

The owner then runs remote input only from a member whose current key it
approved locally ([ADR 0090](../decisions/0090-approved-remote-input.md)).
Approvals are per scope and per member and key; the relay neither receives nor
enforces rights or approvals. For any other sender, the owner opens the
envelope and spends the receipt as usual, then discards the input without
running it and records a notice that names the person. A changed key, a member
that first appears in the directory and a new device, including one whose
`person` is the owner, fall in that case. `watch`, snapshots, the live stream
and comments depend on neither.

An authenticated and approved message from a companion whose `person` is the
conversation's owner reaches the agent with the original text, as a message
from the person themselves. Peers and legacy teams keep the team's authorship
prefix. That distinction uses the authorized roster, never the name or a field
sent freely in the message.

Peers' comments use the last authenticated audience they received. If the relay
omits an update, revocation may be delayed for those senders. Content already
received and previously authorized snapshots are not revocable.

## Companion devices

`Member.person` is optional and links a companion device to the person's
membership; primary members and legacy teams do not have it. The Cloud delivers
the field in the roster and the relay validates it (an ID present in the same
roster, without chains), persists it under `member:` and re-emits it in
`welcome` and `presence`. Old parsers ignore the field.

Audiences, the local `share.audience` and mentions name people. Before
encrypting, the client expands each person into their devices that have a key:
the boxes, the audience published in the relay and the frame's `mentions` come
to list devices, so the relay applies `watch`, `attach`, `write` and inbox per
device without knowing the rule. The owner accepts `watch` and `write` from a
device through the person it belongs to, and runs that `write` only once the
device's key is approved. Key links, approvals and receipts stay per device.
Decision and limits in [ADR 0027](../decisions/0027-companion-devices.md).

The owner's companion devices enter the recipient list only when the local
`Workspace.remote_control` is active. That permission does not cross the
protocol: the outer frame already contains an explicit per-device audience. An
empty local audience allows a share aimed only at the owner's devices. See
[ADR 0030](../decisions/0030-remote-control.md).

## Persistence, limits and compatibility

Credentials and enrollment metadata preserve their storage keys. v4
collaboration state uses the `v4:` prefix; only that prefix is hydrated. v3 data
stays preserved and invisible to the new client. Local shares already consented
to are announced encrypted on reconnection. Old comments are not converted or
redistributed automatically.

The limits live in the protocol: up to 64 members, a 2 MiB input JSON, 1 MiB
binary, 16 MiB output aggregates and 16 MiB per 10-second window. Each box has a
1 MiB limit of ciphertext. The app's cryptographic queue is limited to 16 MiB.
Snapshots use 128 Ki-character parts, with binary limits checked after
expansion. The comment TTL is still 90 days, with the existing thread retention
and count quotas. Persisted ciphertext is limited to 1536 KiB per
share/comment (including resolution), 4 MiB across the set of shares and 8 MiB
across the set of comments per team. The inbox stores only the recipient's own
box. These limits contain per-recipient expansion and leave room for metadata
below the row limit of the
[Durable Object SQLite storage](https://developers.cloudflare.com/durable-objects/platform/limits/).

V3 and v4 do not negotiate a downgrade. Publishing a compatible Worker precedes
distributing the desktop. A v3 rollback keeps v4 data, but restores cleartext
content in v3 collaboration; see the limits and the operation in ADR 0022.

## Evidence

`team-crypto.test.ts`, `team-security.test.ts` and `team-channel.test.ts` check
the client's boundary with real cryptography; `team-owner.test.ts` checks that
input runs only from approved identities while content keeps flowing.
`protocol.test.ts` and `logic.test.ts` check the relay's contracts and rules. The real local Worker is
exercised in `relay/src/worker.integration.test.ts`, and the web mock uses the
same encrypted channel for the E2E flows. There has been no independent security
audit. `team-organizations.test.ts` covers renewal on the same socket, discarding
after an organization switch and companion authorship; the real Worker covers
expiration and the rejection of invalid renewal tickets.

## Bounded HTTP bodies

`/init` and `/enroll` use `relay/src/http.ts::smallJson` with a 1,024-byte body
limit. The helper counts bytes as the body arrives and cancels the reader as
soon as the limit is exceeded, even without `Content-Length` or when that header
understates the body. An oversized declared length is rejected before reading.
Incremental UTF-8 decoding preserves characters split across chunks. Invalid
JSON returns 400; oversized input returns 413. Endpoint schemas, authentication,
and the v4 WebSocket encryption requirements remain unchanged.

The public enrollment route applies the same bound before forwarding the original bounded
JSON text to the Durable Object. Forwarding the live request stream would allow
an early downstream rejection to leave a read pending after the public response,
causing a runtime error and breaking subsequent requests. The Durable Object
retains its own bound for internal callers. Keeping the original text avoids
expanding compact JSON numbers beyond that bound during reserialization.

`http.test.ts` covers cancellation, inaccurate headers, malformed JSON, and
UTF-8 chunk boundaries. The Worker integration test covers streamed enrollment,
recovery after 413, explicit 401/426 upgrade rejections and a valid WebSocket
welcome; transport errors and HTTP 500 never count as authorization rejections.
