---
type: Contract
title: "Relay protocol v4"
description: "Encrypted collaboration and trust on first use (TOFU)."
status: stable
---

# Relay protocol v4

This is the current contract.
[ADR 0022](../decisions/0022-end-to-end-encryption.md) records the decision.
The executable source is `relay/src/protocol.ts`. The app and the Worker import
it. The Git history keeps the retired v3 specification. This contract tells
below how v4 operates with the stored v3 data. The relay does not accept v3
connections.

## Boundary and authentication

The webview encrypts and decrypts. Rust keeps the identity in private storage
and does only authorized actions. The relay sends the ciphertext to the
recipients. It keeps the audience, the quotas, the presence, the comments and
the inbox. The agent stays on the Mac of the owner.

The `pm2` enrollment, the individual credentials and the Cloud tickets keep
their formats. The WebSocket must have `p=4`. `welcome` announces `e2ee: 1`,
`comments: 1` and a random challenge for each socket. Before the client sends a
different message, it answers with `identity { key, proof }`. The proof is an
ECDSA P-256/SHA-256 signature of the UTF-8 JSON
`["prometeu-identity-v4", member, challenge]`. The client makes it with the
private identity.

The Worker makes sure that the client has the private key. Then it publishes
`Member.key` and lets traffic go through. The handshake is complete when the client gets a
presence message with the key that the Worker examined. The Worker does not send other updates to a
socket that did not identify itself.

The client records the link between each member and key before it uses the key.
A changed key replaces the link automatically. A local acceptance is not
necessary
([ADR 0042](../decisions/0042-automatic-key-rotation.md)). Members without a key
receive no content, and the client does not erase their links. A ticket renewal
does not reset TOFU. Content follows the link. Remote input also must have the
approval of the owner for that member and key. See
[Authority and replay](#authority-and-replay). See
[organizations](cloud-organizations.md) for leases and Cloud authorization.

After the identity, organization sockets receive `lease { expires_in }` (1 to
60,000 ms). `renew { ticket }` uses one more 256-bit Cloud ticket, encoded in 43
base64url characters. The ticket must have the same organization and member as
before. The relay refuses a renewal in these conditions:

- Before the identity.
- After the expiry.
- With a ticket that is not correct, that the relay used before or that has the
  membership of a different person.

A renewal keeps the key, the challenge, the watchers and the stream. If the
roster does not change, the relay sends no presence. The confirmation is one
more `lease`. v4 adds these controls. Clients from before ignore the
capability. New clients continue to connect again when a relay from before
does not announce it.

## Envelope and content

`Encrypted = { id, boxes: { [member]: { enc, ct } } }`. IDs are random. `enc`
and `ct` use canonical base64url without padding. Each box uses HPKE Auth,
DHKEM(P-256, HKDF-SHA256), HKDF-SHA256, AES-256-GCM. The `info` binds the JSON
`["prometeu-e2ee-v4", scope, author, recipient, id]`. `scope` is the JSON of
`["organization", cloudOrigin, organizationId]` or
`["team", relayOrigin, teamId]`. The key of the author comes from the local
link. It does not come from a free field of the envelope at any time.

The plaintext contains `{ frame: Up }` or `{ binary: base64url }`. `share`
includes these fields:

- `revision`, a monotonic value that the owner stores.
- `rights` `{ send, control }`, the persons who can send messages and who can
  control the conversation
  ([ADR 0090](../decisions/0090-approved-remote-input.md)).

The rights go only in the authenticated payload. They do not go in the outer
frame at any time. Thus `relay/src/protocol.ts` does not contain them. Older
clients ignore the field. An announcement without this field comes from an owner
that is older than the rights. Peers continue to show the controls of that
owner. `write` includes `expires`, a maximum of two minutes in the future. After
the client opens the box, it validates the payload again. It examines the
workspace, the tab, the author, the recipient and the audience.

An encrypted tab announcement can contain `decision_surface: "terminal"`. This
value means that the person must answer approvals and questions in the native
terminal of the owner. This rule also applies when a viewer has Control. The
outer routing frame does not include this field. If the value is missing, the
application controls from before stay. Clients from before can ignore the field. Thus
the owner and the provider boundary reject responses that they cannot accept.
They do this independently. Prompts, interruption and notes keep their own
rights. The first consumers are the terminal modes of Cursor and Antigravity.
Their current Chat transports keep their capabilities.

An encrypted tab announcement also contains `conversation_mode: "terminal"` for
each native provider. Viewers do not show provider slash suggestions or
permission-mode changes. They reject slash input before they send it. The owner
also rejects this input independently, before it adds a teammate prefix or holds
the input for confirmation. This includes input from clients from before. The outer
routing metadata does not include the mode. If the value is missing, the Chat
behavior from before stays.

The command grammar is this sequence:

1. A `/` at the start of the text. The grammar ignores whitespace before it.
2. Zero or more characters that are not whitespace and not a slash.
3. Whitespace or the end of the text.

Thus `/`, `/compact` and `/plugin:command` are commands. `/Users/file` stays
usual prompt content. Usual prompts, interruption and the answers to individual
requests that the provider can accept stay available. The compatibility tests
and the encrypted round-trip tests are in `relay/src/protocol.test.ts` and
`src/team-channel.test.ts`.

In the outer frame:

- `share`: The titles, the repository and the branch are empty. The retired
  `stage` field stays empty only for compatibility with deployed v4 relays and
  older clients. The issue is null. The tab text and the tokens are null or
  empty, and the status is constant. The IDs, the active tab, the dimensions and
  the explicit audience stay visible. Before the client encrypts, it expands
  `audience: null` to the members that have an available key.
- `note`/`note_reply`: The text is empty. The quotes and the anchors are null.
  The stored ID is the ID of the envelope, which is unique in the team. The
  mentions and the parent links stay visible.
- `note_resolve`: The envelope authenticates the workspace and the root. The
  storage keeps the original ciphertext and adds
  `resolution: { author, encrypted }`.
- `write`: The data is empty. The envelope is only for the owner.
- Binary: The envelope goes through an outer unicast `SNAPSHOT` frame, with
  sequence zero and `more=false`. The box contains the complete original binary
  frame, with the kind, the sequence and the destination. The recipient accepts
  only its own attached tab.

`room.ts` refuses cleartext content fields, incorrect envelopes and binary
frames from before v4. Domain parsers continue to accept cleartext payloads for
internal validation after decryption. This does not let the Worker receive
cleartext payloads on the socket. `downForMember` sends only the box of that
member. This also applies to the `welcome`, `notes` and `inbox` aggregates. The
relay does not have private keys.

## Persistent comments

Comments use flat threads:

- `note` makes a root.
- `note_reply` answers an open root.
- `note_resolve` resolves the root.

Replies get the workspace and the tab of the root. `notes` returns a snapshot
of the workspace. A downstream `note` adds or replaces the comment with the same
ID. This includes a resolved root. The encrypted payload contains the text, the
quote, the tab and the optional `Piece.key` anchor. If the excerpt is not
available, the quote gives context that a person can read. The quote gives no
authority.

Mentions make inbox assignments to the root. When a person opens an assignment,
the app opens the thread. When a person resolves the root, the relay removes its
assignments for all persons. All collaborators with access to the workspace can
resolve it. When `comments: 1` is missing, the client keeps the `inbox_read`
fallback. But it continues to require the v4 E2EE handshake. Current relays
announce that they can use threads.

In the domain parser, if the `tab`, `anchor`, `parent` and `resolved` fields are
missing, the comment becomes a general open root. This does not import v3
storage or let plaintext go on the socket. The relay can see the thread
structure and the retention. The content and the authenticity of the resolution
use the envelope above.

## Authority and replay

The owner uses its board to authorize the snapshot, the live stream and a
received message. When the relay sends an announcement back, the local audience
does not change. For remote shares, the client stores the owner, the key, the
revision and the ID of the last accepted announcement. It rejects a change of
owner and earlier revisions. It accepts the same announcement again after a new
connection. After the client stores the automatic adoption of a new peer key,
the sequence of that owner can start again. The owner ID cannot change.

Before Prometeu does the action of a remote message, it stores an ID and a time limit for
the message. These conditions stop the action:

- A repetition.
- An expiry.
- A write failure.
- A clock that is earlier than the last use.

The receipts stay after a new connection and after a restart. The local
validation of the card continues to be mandatory. Conversation sequences remove
duplicates between the snapshot and the live stream. But the relay can continue
to remove content or show a history that is not complete.

Remote input must have a right for the person who sends it. The owner applies
this right from its board. What the peer shows has no effect:

- Prompts and `turn.interrupt` must have Send messages.
- `request.respond` must have Control.

The devices of the owner have the two rights through `remote_control`. If input
does not have the right, the owner discards it silently.

The owner then runs remote input only from a member with a current key that the
owner approved locally ([ADR 0090](../decisions/0090-approved-remote-input.md)).
Each approval applies to one scope and to one member and key. The relay does not
receive or apply rights or approvals. For all other senders, the owner opens
the envelope and uses the receipt as usual. Then it discards the input without
running it, and it records a notice with the name of the person. These
conditions cause this result:

- A changed key.
- A member that is new in the directory.
- A new device. This includes a device with the owner as its `person`.

`watch`, snapshots, the live stream and comments do not use rights or approvals.

A companion device can send an authenticated and approved message, and its
`person` can be the owner of the conversation. In this condition, the message
goes to the agent with the original text, as a message from the owner. Peers and
legacy teams keep the authorship prefix of the team. This difference uses the
authorized roster. It does not use the name or a free field in the message at
any time.

A message from a peer waits in memory on the Mac of the owner. It waits until the
owner sends, edits or discards it. If the workspace disabled owner confirmation,
the message does not wait. Interruptions and answers do not wait. In all these
conditions, the sender does not receive a receipt.

A viewer goes out of the share of a different owner with `leave { ws }`. The
relay does these steps, also when the owner is offline:

1. It removes all devices of the person of the viewer from the stored audience.
2. It records the person in the `left` field of the entry.
3. It disconnects the devices.
4. It sends `unshare`.
5. It discards the inbox items of the share.

Subsequent `share` announcements cannot add that person again. The relay removes
the devices of the person until the owner sends `readmit { ws, person }`. Only
the owner can send this message. `Shared.left` goes to the owner. The relay does
not include it when no person left. The channel of the owner does not include
those persons in its recipients. It discards their watch messages and their
input. When the owner stops the share, the relay deletes `left` with the entry.
The roster removes persons who left the organization.
[ADR 0094](../decisions/0094-leaving-a-share.md) records the decision.

The comments of peers use the last authenticated audience that they received. If
the relay does not send an update, the revocation for those senders can occur
late. A person cannot revoke content that a peer received or snapshots that the
owner authorized before.

## Companion devices

`Member.person` is optional. It links a companion device to the membership of
the person. Primary members and legacy teams do not have it. The Cloud sends the
field in the roster. The relay validates it: the ID must be in the same roster,
without chains. The relay stores it under `member:` and sends it again in
`welcome` and `presence`. Parsers from before ignore the field.

Audiences, the local `share.audience` and mentions identify persons. Before the
client encrypts, it expands each person into the devices of that person that
have a key. Thus the boxes, the audience that the relay publishes and the
`mentions` of the frame list devices. The relay applies `watch`, `attach`,
`write` and the inbox to each device without knowledge of the rule.

The owner accepts `watch` and `write` from a device through the person of that
device. It runs that `write` only after it gives approval to the key of the device.
Each device has its own key links, approvals and receipts.
[ADR 0027](../decisions/0027-companion-devices.md) records the decision and the
limits.

The companion devices of the owner go into the list of recipients only when the
local `Workspace.remote_control` is active. That permission does not go through
the protocol. The outer frame contains an explicit audience of devices before this step.
An empty local audience lets the owner make a share only for the devices of the
owner. See [ADR 0030](../decisions/0030-remote-control.md).

## Persistence, limits and compatibility

Credentials and enrollment metadata keep their storage keys. The collaboration
state of v4 uses the `v4:` prefix. The client loads only data with that prefix.
The v3 data stays in storage, and the new client does not see it. When the
client connects again, it announces again, encrypted, the local shares that the
owner gave consent for before. The client does not change or send again the comments
from before automatically.

The protocol contains these limits:

- A maximum of 64 members.
- 2 MiB for input JSON.
- 1 MiB for binary data.
- 16 MiB for output aggregates.
- 16 MiB for each window of 10 seconds.
- 1 MiB of ciphertext for each box.

The cryptographic queue of the app has a limit of 16 MiB. Snapshots use segments of
128 Ki characters. The client examines the binary limits after the expansion.
The TTL of comments continues to be 90 days, with the current quotas for the
thread retention and the count.

The limits of stored ciphertext for each team are these:

- 1536 KiB for each share or comment, which includes the resolution.
- 4 MiB for all the shares.
- 8 MiB for all the comments.

The inbox stores only the box of the recipient. These limits control the
expansion for each recipient. They also keep space for metadata below the row
limit of the
[Durable Object SQLite storage](https://developers.cloudflare.com/durable-objects/platform/limits/).

V3 and v4 do not negotiate a downgrade. Publish a compatible Worker before you
publish the desktop. A v3 rollback keeps the v4 data, but v3 collaboration
sends cleartext content again. ADR 0022 gives the limits and the procedure.

## Evidence

`team-crypto.test.ts`, `team-security.test.ts` and `team-channel.test.ts` examine
the boundary of the client with real cryptography. `team-owner.test.ts` makes
sure that input runs only from approved identities and that content continues
to go through. `protocol.test.ts` and `logic.test.ts` examine the contracts and
the rules of the relay. This includes the exit from a share and the readmission.
`relay/src/worker.integration.test.ts` uses the real local Worker. The web mock
uses the same encrypted channel for the E2E flows. No independent security audit
occurred.

`team-organizations.test.ts` examines these items:

- The renewal on the same socket.
- The discard of data after a change of organization.
- The authorship of companion devices.

The tests with the real Worker examine the expiry and the rejection of renewal
tickets that are not correct.

## Bounded HTTP bodies

`/init` and `/enroll` use `relay/src/http.ts::smallJson` with a body limit of
1,024 bytes. The helper counts the bytes while it receives the body. It cancels the
reader immediately when the body is more than the limit. It does this also
without `Content-Length`, or when that header gives a length that is too small.
If the declared length is too large, the helper rejects the body before it reads
it. The incremental UTF-8 decoder keeps characters that go across two or more
chunks. Incorrect JSON returns 400. Input that is too large returns 413. The
endpoint schemas, the authentication and the encryption requirements of the v4
WebSocket do not change.

The public enrollment route applies the same limit. Then it sends the original
bounded JSON text to the Durable Object. It does not send the live request
stream. A rejection downstream before the end of the stream can keep a read open
after the public response. This causes a runtime error, and the subsequent
requests have a failure. The Durable Object keeps its own limit for internal callers. The original
text prevents an expansion of compact JSON numbers above that limit during a new
serialization.

`http.test.ts` examines the cancellation, inaccurate headers, incorrect JSON and
UTF-8 chunk boundaries. The Worker integration test examines these items:

- The streamed enrollment.
- The recovery after 413.
- The explicit rejections with 401 and 426 upgrade.
- A correct WebSocket welcome.

Transport errors and HTTP 500 are not authorization rejections at any time.
