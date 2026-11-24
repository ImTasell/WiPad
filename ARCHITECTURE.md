# WiPad Architecture

Status: Backend implementation baseline (hardware verification and UI work remain)  
Repository: `ImTasell/WiPad`  
Platforms: Android 11+ (Kotlin + Jetpack Compose) and Linux desktop (Rust + Slint)

## 1. Product shape

WiPad turns one Bluetooth controller connected to an Android phone into one native Linux game controller.

The intended path is:

```text
Bluetooth controller
       |
       v
Android phone (WiPad)
  - Bluetooth input
  - controller state
  - LAN discovery
  - authenticated session
       |
       | local Wi-Fi network
       v
Linux desktop (WiPad)
  - LAN discovery
  - authenticated session
  - input validation
  - Linux uinput virtual gamepad
       |
       v
Games and applications
```

WiPad does not stream video or audio. It transports controller state only.

The product is intentionally one phone, one controller, and one desktop. Multi-controller and multi-desktop management are not goals for the initial implementation.

## 2. Core design goals

1. Discover the other WiPad app automatically on the same local network.
2. Avoid manual IP addresses, port entry, and PIN entry.
3. Remember a trusted peer and reconnect automatically when it returns.
4. Keep discovery separate from the authenticated input session.
5. Keep Android-specific code in Kotlin and Linux-specific code in Rust.
6. Use a small, versioned, language-independent wire protocol.
7. Validate all received data before it can affect the virtual controller.
8. Recover cleanly from app restarts, Wi-Fi changes, and temporary disconnects.

## 3. Application boundaries

### Android application

Technology: Kotlin, Jetpack Compose, Android Bluetooth APIs, Android network APIs.

Responsibilities:
- Scan for and connect to the selected Bluetooth controller.
- Read controller input and normalize it into WiPad's controller-state model.
- Advertise WiPad's presence on the local network.
- Discover compatible desktop peers.
- Establish and maintain the authenticated Wi-Fi session.
- Send controller-state updates and connection health information.
- Store the trusted desktop identity and user preferences.
- Expose clear Bluetooth, discovery, and session status to the UI.

Android must not depend on the desktop Rust implementation or a shared native library. No JNI or FFI is required.

### Linux desktop application

Technology: Rust, Slint, Linux networking, Linux `uinput`.

Responsibilities:
- Advertise WiPad's presence and discover Android peers.
- Establish and maintain the authenticated Wi-Fi session.
- Validate and consume controller-state updates.
- Create and manage one virtual gamepad through `/dev/uinput`.
- Release all pressed buttons and axes safely when a session ends.
- Store the trusted phone identity and user preferences.
- Expose discovery, session, and virtual-device status to the UI.

The desktop app should report clearly when access to `/dev/uinput` is unavailable. It must not silently claim that a virtual controller is active when device creation failed.

## 4. Automatic discovery

### Discovery behavior

Both apps participate in discovery. Neither app requires the user to type the other's address.

The initial design uses **mDNS / DNS-SD on the local network**:
- Android advertises a WiPad phone service and browses for the desktop service.
- Linux advertises a WiPad desktop service and browses for the phone service.
- Service records identify the WiPad protocol family, protocol version compatibility, and the endpoint needed to begin a session.
- Discovery announcements contain no controller input, credentials, private keys, or session secrets.
- The apps continue to browse while disconnected and refresh their presence when network connectivity changes.

mDNS uses local-link multicast. It is not a way to discover devices across the public internet or across networks that block multicast. Some Wi-Fi networks enable client/AP isolation; in that case devices may not be able to communicate even when both have Wi-Fi access. The UI should explain this instead of retrying forever without feedback.

Implementation direction:
- Android: evaluate Android's `NsdManager` APIs for service registration and discovery.
- Linux: use `mdns-sd` with automatically refreshed IPv4 addresses across active interfaces.
- Android: use `NsdManager`, refresh browsing when the default network changes, and discard duplicate or stale resolutions.
- The Android connection manager follows the latest resolved desktop endpoint instead of retrying a stale IP address.
- Keep discovery behind a small platform-specific interface so the protocol and UI do not depend on a particular mDNS library.

If real-device testing shows mDNS is unreliable on target networks, evaluate a bounded UDP multicast/broadcast fallback. Do not implement multiple discovery transports before the basic mDNS path has been tested.

### Discovery states

The UI should distinguish:
- **Scanning:** browsing for compatible peers.
- **Peer found:** a compatible peer is visible on the LAN.
- **Connecting:** a session handshake is in progress.
- **Trusted / connected:** the peer identity is trusted and the session is active.
- **Unavailable:** discovery is blocked, the peer disappeared, or the local network changed.

Discovery is not proof of identity. A discovered service must never be treated as trusted solely because its name, address, or TXT records look correct.

## 5. Pairing without PINs

WiPad will not require users to enter a numeric PIN or manually copy an IP address.

However, discovery beacons are public to devices on the local network. They cannot safely authenticate a peer. A malicious or unrelated device could advertise a fake WiPad service.

Initial trust therefore uses a **one-time approval flow, not a PIN flow**:
1. A device discovers a compatible peer.
2. The apps establish an encrypted handshake and obtain the peer's stable cryptographic identity.
3. The UI presents the peer's device name and platform and asks the user to approve the first connection.
4. On approval, the peer identity is stored as trusted.
5. Later connections authenticate against that stored identity and can reconnect without asking again.

No controller input should be accepted before the authenticated session is established.

The first-contact approval is an intentional security boundary. A future fully unattended first connection would need another reliable trust mechanism; simply auto-accepting any device that broadcasts a WiPad beacon would allow nearby devices on the LAN to impersonate a peer.

### Identity and session security

**Implemented security baseline:** mutual TLS 1.3 over the local TCP session, with a persistent per-installation ECDSA P-256 identity on each platform. TLS and certificate validation are provided by the platform libraries; WiPad does not implement cryptographic primitives itself.

Requirements:
- Each installation has a persistent cryptographic identity.
- First contact requires an explicit approval on both devices; each side displays the same short comparison string.
- Approval requests are bounded to 120 seconds and fail closed on timeout, cancellation, or a second concurrent request.
- The first approved identity is stored as the trusted peer only after both approvals and both `SESSION_READY` messages.
- Reconnection verifies the same peer identity; a changed identity is rejected until the user explicitly revokes the stored trust.
- Session keys are fresh and are not sent in discovery records.
- Controller packets are encrypted and integrity-protected.
- Untrusted peers cannot create a virtual controller session merely by discovering the desktop.
- Provide a way to revoke the trusted peer from settings. Backend revocation hooks exist on both platforms; the settings UI is still pending.

The wire-level pairing and session messages are documented in `PROTOCOL.md`. The platform TLS stacks select the negotiated TLS 1.3 cipher suite. The remaining security work is UI integration and real-device validation of trust, revocation, and identity-change behavior.

## 6. Automatic reconnection

Auto-reconnect is a core behavior, not an optional convenience.

Expected behavior:
- While the app is active, continue discovery when no session is connected.
- When a previously trusted peer reappears, attempt authentication automatically.
- Use bounded exponential backoff (1–30 seconds) with up to 500 ms jitter after failed connection attempts.
- Reset the retry delay after a session has been established.
- Detect network changes, refresh mDNS browsing/advertisements, and reconnect using the latest resolved endpoint.
- Do not create multiple simultaneous sessions.
- If the Bluetooth controller disconnects, keep the desktop session state distinct and report that controller input is unavailable.
- If the Wi-Fi session drops, stop forwarding input and release all virtual-gamepad buttons/axes on the desktop.
- On successful reconnection, begin from a neutral controller state before applying new input.

Suggested retry schedule (implementation may tune this after testing):
- Initial retry: approximately 1 second.
- Increase delay after repeated failures, capped around 15–30 seconds.
- Add random jitter so repeated retries do not synchronize unnecessarily.
- Discovery remains active between session attempts.

The UI should show whether it is scanning, reconnecting, connected, or waiting for the trusted peer. It should not require the user to repeatedly press Connect.

Android background execution is constrained by the OS. The initial implementation should define and test its foreground/background behavior explicitly rather than promising uninterrupted discovery when Android has suspended or force-stopped the app. A foreground service may be needed for a user-requested active controller session; its notification and lifecycle behavior must follow current Android requirements.

## 7. Session transport

**Implemented baseline:** a length-prefixed TCP stream protected by mutual TLS 1.3. The application protocol is versioned independently of TLS. Controller snapshots are sent at approximately 60 Hz.

The transport must support:
- Low-latency delivery of frequent controller-state updates.
- Authentication and encryption.
- Session liveness detection (desktop PING every 2 seconds; missing PONG after 7 seconds closes the session).
- Version negotiation (protocol major 1, minor 2).
- Bounded packet sizes and strict validation.
- Clean disconnect and reconnect behavior.

A datagram-based input path is not part of the current implementation. Reconsider it only if real-device latency measurements show the reliable stream is insufficient. Discovery remains mDNS and is separate from the authenticated session.

For controller state, newer complete state snapshots are generally more useful than replaying a long backlog of stale input. The protocol should include sequence numbers and define how to handle reordering, duplication, loss, and stale packets.

## 8. Controller state model

The Android Bluetooth layer translates device-specific input into a platform-independent WiPad state.

The normalized state should represent:
- Button states.
- D-pad directions.
- Analog stick axes.
- Trigger axes or trigger values.
- A monotonically increasing sequence number.
- A session-relative timestamp or equivalent freshness information.

The exact button and axis layout belongs in `PROTOCOL.md`. It must be explicit, versioned, and independent of Android key-code values.

The desktop maps this normalized state to Linux gamepad capabilities through `uinput`. Device capabilities should be stable for the lifetime of a virtual device; changes that require a different capability set should recreate the device in a controlled way.

## 9. Linux virtual gamepad lifecycle

The desktop owns the virtual device.

Lifecycle:
1. Confirm that `/dev/uinput` is available and writable.
2. Create a virtual gamepad with the agreed buttons and axes.
3. Mark the virtual controller ready only after device creation succeeds.
4. Apply validated controller-state updates.
5. On disconnect, timeout, application shutdown, or fatal input error, release all controls and destroy the device cleanly.

A lost session must never leave a button logically held down. The desktop releases controls after 2 seconds without a fresh input snapshot and closes an idle session after 15 seconds. A separate 7-second heartbeat timeout detects a dead peer even while controller snapshots are flowing.

## 10. Proposed repository structure

Keep the initial repository understandable and avoid premature crate/module fragmentation.

```text
WiPad/
├── android/
│   └── app/
├── desktop/
│   ├── Cargo.toml
│   └── src/
├── docs/
│   └── (future implementation notes)
├── ARCHITECTURE.md
├── DESIGN_SYSTEM.md
├── PROTOCOL.md
├── DEVELOPMENT.md
└── LICENSE
```

The Android Gradle project can grow into multiple modules if Bluetooth, discovery, protocol, and UI boundaries become substantial. The Rust desktop can begin as one workspace/package and split into crates only when there is a concrete benefit.

Do not share implementation code across Kotlin and Rust. Share the protocol specification and test vectors instead.

## 11. Implementation milestones

### Milestone 1 — platform skeletons (complete)
- Create the Android Compose app and Rust/Slint desktop app.
- Establish build and test workflows.
- Add platform capability checks and basic status screens.

### Milestone 2 — discovery prototype (implementation complete; hardware verification pending)
- Implement mDNS registration and browsing on both platforms.
- Display discovered peers without connecting.
- Test on real Android 11 and Linux devices on the same Wi-Fi network.
- Test network changes, app restarts, and multicast-blocked networks.

### Milestone 3 — trust and authenticated session (backend complete; approval UI pending)
- Specify the threat model and select reviewed cryptographic libraries.
- Implement first-time approval and persistent peer identity.
- Verify that unknown peers cannot send controller input.
- Test trusted reconnection and identity changes.

### Milestone 4 — controller pipeline (implementation complete; hardware verification pending)
- Connect the Bluetooth controller on Android.
- Normalize input into the protocol state model.
- Send state updates to the desktop.
- Validate updates and expose them through a Linux `uinput` device.

### Milestone 5 — reliability and polish (backend reliability implemented; end-to-end verification pending)
- Add retry/backoff, liveness checks, stale-input release, and network-change recovery.
- Test controller disconnects, Wi-Fi loss, app restarts, and desktop shutdown.
- Finish accessible status UI and diagnostics.

## 12. Decisions still open

These must be resolved before the wire protocol and implementation are frozen:
- Android foreground-service behavior and supported background scenarios.
- Hardware compatibility and real-device timing across target Bluetooth controllers and Wi-Fi networks.
- Android foreground-service behavior and supported background scenarios.
- Whether a unicast fallback is needed on networks that block mDNS.

## 13. Non-goals

The initial WiPad release does not include:
- Video or audio streaming.
- Internet-wide device discovery or remote relay.
- Multiple simultaneous phones, controllers, or desktops.
- A custom Android native library or JNI bridge.
- Automatic acceptance of arbitrary LAN devices without a trust decision.

---
This document describes the system boundaries and intended behavior. The protocol specification must define the exact wire format and security handshake before either app implements networking.
