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

The initial protocol family is major version `1`. The discovery record advertises supported major versions before a session is attempted.

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
2. Establish an authenticated-encryption handshake using a reviewed, interoperable cryptographic protocol.
3. Display the peer's device name and platform, and request a one-time approval.
4. On approval, persist the peer's public identity.
5. Automatically authenticate and reconnect to that same identity on later encounters.

This is approval-based pairing, not PIN pairing. After the initial approval, normal reconnects require no user action.

### 5.2 Identity rules

- Each installation creates a cryptographically random Installation ID and persistent identity key.
- The private key must remain local and must never be advertised.
- The trusted peer is identified by its cryptographic public key or a stable fingerprint derived from it, not by IP address, hostname, or display name.
- A changed identity for a known peer requires explicit re-approval.
- Provide a user-visible way to forget/revoke the trusted peer.
- Unknown peers must never inject controller state or create an active virtual gamepad session.

### 5.3 Cryptographic selection

The exact handshake and cipher suite are intentionally not frozen in this draft. Before implementation, select a reviewed protocol and maintained libraries that interoperate between Android and Rust. Do not invent a custom key exchange or encryption scheme.

The selected session security must provide peer authentication, confidentiality, integrity, replay protection, and fresh session keys. No input message is accepted before authentication completes.

## 6. Session lifecycle

The logical session has these states:

1. `DISCOVERING` — browsing for a compatible peer.
2. `PEER_FOUND` — a compatible service has been resolved.
3. `AWAITING_APPROVAL` — first-contact trust approval is required.
4. `AUTHENTICATING` — identity and session security are being established.
5. `READY` — authenticated session exists; no input is yet considered active.
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
| `HELLO` | Both | Exchange protocol version and public peer metadata |
| `AUTH` | Both | Carry the selected secure-handshake messages |
| `SESSION_READY` | Both | Confirm authenticated session setup |
| `INPUT_STATE` | Phone → Desktop | Send a complete controller-state snapshot |
| `PING` | Both | Liveness probe |
| `PONG` | Both | Liveness response |
| `CONTROLLER_STATUS` | Phone → Desktop | Report controller connected/disconnected state |
| `GOODBYE` | Both | Graceful session close |
| `ERROR` | Both | Report a bounded protocol/session error |

Implementations must reject unknown required message types, invalid lengths, invalid enum values, and messages that are not valid in the current session state.

### 7.2 Message envelope

The final byte-level envelope is pending transport and serialization selection. It must include:

- Protocol major and minor version.
- Message type.
- Payload length or an equivalent bounded framing mechanism.
- Session context sufficient to reject messages from an old session.
- A defined byte order and integer representation.

Maximum message sizes must be small and explicitly bounded. Discovery metadata and input messages must never permit unbounded allocation based on peer-provided lengths.

## 8. Controller-state semantics

### 8.1 Complete snapshots

`INPUT_STATE` represents the complete current state, not a list of button transitions. Each accepted snapshot replaces the previous state.

This allows the receiver to recover from a lost update without replaying an event history. A stale or out-of-order snapshot must not overwrite a newer accepted state.

### 8.2 Required fields

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

- The desktop enforces a receiver-side input timeout.
- If no valid input snapshot arrives before the timeout, release every virtual button and axis to neutral.
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
