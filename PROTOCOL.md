# WiPad Protocol

Status: Draft 0.1 — design contract, not yet frozen  
Scope: Discovery metadata, peer identity, session lifecycle, and controller-state semantics  
Platforms: Android 11+ (Kotlin) and Linux desktop (Rust)

This document defines the language-independent behavior both applications must implement. It deliberately does not prescribe Kotlin or Rust implementation details.

## 1. Protocol goals

- Discover WiPad peers automatically on the same local network.
- Require no manual IP address, port, or numeric PIN entry.
- Reconnect to a previously trusted peer without repeated user interaction.
- Keep discovery separate from trust, authentication, and the active input session.
- Carry controller state with low overhead and bounded memory use.
- Reject malformed, stale, incompatible, or unauthenticated traffic.
- Keep the protocol small enough to implement independently in Kotlin and Rust.

The initial product supports one Android phone, one Bluetooth controller, and one Linux desktop. The protocol does not negotiate multiple simultaneous controllers or desktops.

## 2. Terms and roles

| Term | Meaning |
| --- | --- |
| Phone | Android WiPad app; Bluetooth controller input source |
| Desktop | Linux WiPad app; creates the virtual gamepad |
| Peer | The other WiPad installation |
| Installation ID | Stable, random identifier for one app installation |
| Identity key | Persistent cryptographic key used to authenticate an installation |
| Session | An authenticated live connection between one phone and one desktop |
| Input snapshot | Complete current state of all supported controls |

The phone is the input producer. The desktop is the input consumer and virtual-device owner. Discovery is bidirectional: both applications advertise and browse.

## 3. Protocol versioning

The protocol has a major and minor version.

- A major version changes when wire compatibility breaks.
- A minor version adds optional capabilities or messages without changing existing field meanings.
- Implementations must reject unsupported major versions.
- Implementations may ignore unknown optional capabilities and optional message types.
- Required fields must never be silently ignored.

The initial protocol family is major version `1`. The controller-presence message is introduced in minor version `1`; implementations must reject peers advertising a lower minor version because they cannot safely process this session behavior. The discovery record advertises supported major versions before a session is attempted.

## 4. Automatic LAN discovery

### 4.1 Discovery mechanism

The initial discovery mechanism is DNS-Based Service Discovery (DNS-SD) over multicast DNS (mDNS). The apps announce service records on the local link and browse for the counterpart role.

Service types:

- Desktop service: `_wipad-desktop._tcp.local.`
- Phone service: `_wipad-phone._tcp.local.`

The service record provides the endpoint port. The resolved DNS-SD address provides the host address; users do not type either value.

Each installation uses a unique service instance name derived from its Installation ID. The human-readable device name is display metadata only and must not be treated as an identity.

### 4.2 TXT metadata

TXT records contain only public discovery metadata:

| Key | Meaning |
| --- | --- |
| `pv` | Supported protocol major versions, e.g. `1` |
| `id` | Installation ID |
| `role` | `phone` or `desktop` |
| `name` | Short display name |
| `caps` | Optional comma-separated capability identifiers |

TXT records must not contain private keys, passwords, pairing secrets, session keys, controller input, or authentication tokens.

Discovery data is untrusted. A matching service name, role, Installation ID, or TXT record does not authenticate a peer.

### 4.3 Discovery behavior

- Advertise while the app is available for a WiPad session.
- Browse continuously while disconnected, subject to platform lifecycle restrictions.
- Refresh registration and browsing after Wi-Fi/network changes.
- Treat duplicate, malformed, incompatible, or rapidly changing records defensively.
- Resolve the advertised service before attempting a session.
- Do not open a session solely because a beacon was received; apply the trust rules in section 5.

mDNS uses link-local multicast. It may not cross routers and may be blocked by guest Wi-Fi, client isolation, or access-point policy. If discovery fails, show a useful network diagnostic. A fallback discovery mechanism is a later decision, not part of this draft.

## 5. Trust, pairing, and authentication

### 5.1 No PIN or manual endpoint

WiPad does not use numeric PIN entry, manual IP entry, or manual port entry.

First contact still requires a deliberate trust decision. A broadcast is visible to other devices on the LAN and can be spoofed. Automatically trusting the first device that claims to be WiPad would permit impersonation.

First-contact flow:

1. Discover a compatible peer.
2. Establish mutual TLS 1.3 and verify both certificate signatures and P-256 public keys.
3. Derive the same comparison string from the phone certificate and desktop certificate, always in that order.
4. Display the peer's name, platform, and comparison string on both devices. Each user checks that the strings match and explicitly approves.
5. Exchange an encrypted `PAIRING_DECISION` from each peer. A rejection closes the session.
6. Persist the remote certificate fingerprint only after the local user has approved and the remote approval has been received.
7. Exchange `SESSION_READY`; only then may controller input be accepted.

A previously stored matching peer fingerprint counts as that device's local approval on reconnect. It does not count as the other device's approval. Both sides must still prove that they recognize the other identity. This is approval-based pairing, not PIN pairing; the comparison string is displayed, never typed. After both sides have stored matching identities, normal reconnects require no user action.

### 5.2 Identity rules

- Each installation creates a cryptographically random Installation ID and persistent identity key.
- The private key must remain local and must never be advertised.
- The trusted peer is identified by its cryptographic public key or a stable fingerprint derived from it, not by IP address, hostname, or display name.
- A changed identity for a known peer requires explicit re-approval.
- Provide a user-visible way to forget/revoke the trusted peer.
- Unknown peers must never inject controller state or create an active virtual gamepad session.

### 5.3 Cryptographic selection

The initial implementation uses mutual TLS 1.3 with ECDSA P-256 identities and a TLS 1.3 AEAD cipher suite negotiated by the platform libraries.

- The desktop is the TLS server; the phone is the TLS client.
- Both peers present a client/server certificate and prove possession of the corresponding private key during the TLS 1.3 handshake.
- Android stores its non-exportable P-256 private key in Android Keystore. The Linux desktop stores its PKCS#8 private key in its private WiPad configuration directory with owner-only permissions.
- Each certificate must contain a currently valid P-256 public key. TLS 1.3 CertificateVerify signatures are verified by the TLS implementation.
- The first connection may authenticate an as-yet-unpinned certificate only for the purpose of first-contact pairing. No application messages carrying input are accepted at this point.
- Both peers derive the same short comparison string from SHA-256 over a domain separator and the ordered phone and desktop certificate fingerprints. Both users must compare the displayed string and explicitly approve the peer before the fingerprint is persisted.
- The stored SHA-256 certificate fingerprint is the peer identity for subsequent reconnects. A mismatch is an identity change, not an ordinary reconnect, and requires re-approval.
- Every reconnect performs a new TLS 1.3 handshake with fresh traffic keys. The stored pin is checked during certificate verification.

The short comparison string is for out-of-band comparison on both screens. It is not entered as a PIN and is not a substitute for TLS authentication. A successful TLS handshake does not by itself authorize controller input: the peer-trust and approval state must be complete first.

Implementations must not disable TLS signature verification, accept arbitrary cipher suites, or persist a peer fingerprint automatically just because discovery or a TLS handshake succeeded.

## 6. Session lifecycle

The logical session has these states:

1. `DISCOVERING` — browsing for a compatible peer.
2. `PEER_FOUND` — a compatible service has been resolved.
3. `TLS_HANDSHAKE` — mutual TLS 1.3 is being established.
4. `AWAITING_APPROVAL` — one or both peers still need to approve the presented identity.
5. `READY` — both approvals are complete and `SESSION_READY` has been exchanged; input may now be enabled.
6. `STREAMING` — valid controller snapshots are being applied.
7. `RECONNECTING` — the trusted peer is temporarily unavailable.
8. `CLOSED` — session ended or was rejected.

A successful authentication does not imply that the Bluetooth controller is connected or that the Linux virtual device is ready. Those are separate subsystem states.

Only one active session is permitted. If multiple candidate peers are discovered, do not connect to an arbitrary untrusted peer. Prefer the stored trusted identity; otherwise present candidates for first-contact approval.

## 7. Message model

The protocol defines logical messages independently of the eventual transport. The transport decision must preserve message boundaries, ordering semantics, bounded message sizes, and authenticated delivery.

### 7.1 Required logical messages

| Message | Direction | Purpose |
| --- | --- | --- |
| `PAIRING_HELLO` | Both | Exchange WiPad protocol version, role, and display metadata inside TLS |
| `PAIRING_DECISION` | Both | Approve or reject the peer identity after comparing the displayed string |
| `SESSION_READY` | Both | Confirm both peers approved and the session may accept input |
| `INPUT_STATE` | Phone → Desktop | Send a complete controller-state snapshot |
| `CONTROLLER_STATUS` | Phone → Desktop | Report whether a physical controller is connected |
| `CONTROLLER_STATUS` | Phone → Desktop | Report whether a physical controller is connected |
| `PING` | Both | Liveness probe |
| `PONG` | Both | Liveness response |
| `CONTROLLER_STATUS` | Phone → Desktop | Report controller connected/disconnected state |
| `GOODBYE` | Both | Graceful session close |
| `ERROR` | Both | Report a bounded protocol/session error |

Implementations must reject unknown required message types, invalid lengths, invalid enum values, and messages that are not valid in the current session state.

### 7.2 Message envelope and TCP framing

The initial session transport is one TCP connection per active session. TCP is a byte stream: read and write boundaries do not correspond to application messages.

The mutual TLS 1.3 handshake runs directly over TCP. Once TLS is established, every WiPad application message is carried in a bounded frame:

- Four-byte unsigned payload length, big-endian.
- Payload follows immediately; the length excludes the four-byte prefix.
- Valid payload size is 1–4096 bytes.
- Zero-length, oversized, and truncated frames are rejected before message parsing.
- A frame decoder must not allocate from an unchecked peer-provided length.

The application payload begins with a one-byte message type. Message payload definitions:

- `PAIRING_HELLO`: protocol major (u8), protocol minor (u8), role (u8: 1 phone, 2 desktop), display-name length (u8, at most 64), UTF-8 display name.
- `PAIRING_DECISION`: one byte, `0` reject or `1` approve. Each side sends at most one decision per connection.
- `SESSION_READY`: empty payload. Each peer sends it only after its local and remote approval flags are both true.
- `CONTROLLER_STATUS`: one byte after the type (`0` disconnected, `1` connected). The phone sends this immediately after session readiness and whenever physical controller presence changes. On `0`, the desktop releases all virtual controls immediately and ignores subsequent input snapshots until a `1` status arrives.
- `INPUT_STATE`, `PING`, `PONG`, `GOODBYE`, and `ERROR`: their message-specific payloads are defined by the protocol sections below.

The Rust and Kotlin implementations share the same frame conformance vector and malformed-frame tests. See [docs/TCP_FRAMING.md](docs/TCP_FRAMING.md).

Framing does not provide authentication or confidentiality. Application frames travel only inside the authenticated TLS 1.3 stream. A valid TLS connection is not yet an input session: reject `INPUT_STATE` until both approvals and `SESSION_READY` are complete.

## 8. Controller-state semantics

### 8.1 Complete snapshots

`INPUT_STATE` represents the complete current state, not a list of button transitions. Each accepted snapshot replaces the previous state.

This allows the receiver to recover from a lost update without replaying an event history. A stale or out-of-order snapshot must not overwrite a newer accepted state.

### 8.2 Required fields

The byte-level version 0.1 payload, exact button assignments, hat encoding, and shared test vector are specified in [docs/CONTROLLER_STATE.md](docs/CONTROLLER_STATE.md). The message type is `INPUT_STATE` (`0x10`), with a 64-bit sequence, 64-bit session-relative sample time, a 32-bit button bitset, an 8-way hat, four signed 16-bit stick axes, and two unsigned 16-bit trigger values.

The initial controller-state model contains:

| Field | Meaning |
| --- | --- |
| `sequence` | Monotonically increasing unsigned sequence number for this session |
| `buttons` | Fixed, versioned button bitset |
| `hat` | D-pad/hat state |
| `left_x`, `left_y` | Left stick axes |
| `right_x`, `right_y` | Right stick axes |
| `left_trigger`, `right_trigger` | Trigger values |
| `sample_time` | Monotonic sample time or session-relative timestamp |

Axis conventions:

- Stick axes use signed 16-bit normalized values.
- Center is zero; negative and positive values represent opposite directions.
- Trigger values use unsigned 16-bit normalized values, where zero is released.
- The exact button bit assignments and hat encoding must be enumerated in the implementation appendix before protocol freeze.
- Android key codes must never appear on the wire.

### 8.3 Input freshness and safety

- The desktop enforces a 2-second receiver-side input freshness timeout. If no valid input snapshot arrives before the timeout, release every virtual button and axis to neutral.
- If the stream remains idle for 15 seconds, terminate the session so the phone can reconnect; do not keep a dead session reserved indefinitely.
- On session start, apply a neutral state before the first live snapshot.
- On controller disconnect, the phone reports the state and sends a neutral snapshot when possible.
- On session close, timeout, or fatal validation error, the desktop releases all controls.
- Clamp or reject values outside the specified range; never pass unchecked peer values to `uinput`.

## 9. Ordering, loss, and rate

Sequence numbers are scoped to a session and increase for each produced snapshot. The receiver must reject duplicates and older snapshots, including wraparound cases handled according to the final integer-width rule.

Input snapshots are replaceable state. The protocol should prioritize the newest complete state rather than queueing an unbounded backlog of old snapshots.

The target update rate and transport are not frozen here. They must be measured on real Android 11 and Linux hardware. Any queue between Bluetooth input and network transmission must be bounded; when congested, replace an unsent snapshot with the newest state.

## 10. Reconnection

- Reconnection targets the stored trusted peer identity, not a remembered IP address.
- Discovery remains active between connection attempts whenever the OS allows it.
- Use bounded exponential backoff with jitter.
- Re-authenticate every new session; do not reuse old session keys.
- Reset sequence and session-relative time state when a new session is established.
- The desktop neutralizes the virtual device immediately when the prior session becomes invalid.
- A changed peer identity is not an ordinary reconnect and requires approval.

## 11. Error handling

Errors should be machine-readable internally and mapped to clear user-facing messages.

Initial error categories:

- `UNSUPPORTED_VERSION`
- `MALFORMED_MESSAGE`
- `MESSAGE_TOO_LARGE`
- `AUTHENTICATION_FAILED`
- `IDENTITY_CHANGED`
- `PEER_NOT_TRUSTED`
- `SESSION_BUSY`
- `SESSION_TIMEOUT`
- `INPUT_INVALID`
- `TRANSPORT_UNAVAILABLE`

Do not expose cryptographic internals or sensitive implementation details in user-facing error messages. Log enough diagnostic context to debug failures without logging keys, secrets, or controller input unnecessarily.

## 12. Compatibility and testing

Both implementations must share protocol test vectors for:

- Version compatibility and rejection.
- Discovery metadata parsing and malformed records.
- Message framing and length limits.
- Button/axis serialization and boundary values.
- Sequence ordering, duplicates, and stale snapshots.
- Session-state transition validity.
- Timeout and neutral-state behavior.
- Identity mismatch and untrusted-peer rejection.

Test across actual Android 11 and Linux devices on the same Wi-Fi network. Include Wi-Fi loss, access-point changes, app restarts, duplicate advertisements, multicast-blocked networks, and a peer that disappears mid-session.

## 13. Decisions required before protocol freeze

- Exact authenticated handshake, key format, and cipher suite.
- Exact transport for control and input data.
- Binary serialization and message framing.
- Button bit assignments and D-pad/hat encoding.
- Input update rate and timeout values.
- Whether first-contact approval is shown on the desktop, phone, or both.
- Android background/foreground session lifecycle.
- Whether a discovery fallback is necessary on multicast-blocked networks.

---

This draft defines shared behavior, not a finalized wire encoding. Freeze the byte-level format only after transport, cryptographic interoperability, and controller capabilities have been tested on the target devices.
