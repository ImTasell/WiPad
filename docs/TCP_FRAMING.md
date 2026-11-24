# WiPad TCP Framing

Status: Draft 0.1
Scope: Shared byte-stream framing for Android and Linux

Each application message is carried in one TCP frame:

| Field | Size | Encoding |
| --- | ---: | --- |
| Payload length | 4 bytes | Unsigned big-endian integer |
| Payload | 1–4096 bytes | Opaque message bytes |

The length excludes the four-byte prefix. A receiver must reject a zero length, a length greater than 4096, or a truncated payload before parsing the message. It must not allocate based on an unchecked length. One TCP connection carries a sequence of frames; TCP read boundaries have no protocol meaning.

The same framing applies to Noise handshake records and encrypted transport records. Authentication and message validation happen after framing; a valid frame is not evidence that its sender is trusted.

Shared conformance vector:

- Payload: 10 20 30
- Encoded frame: 00 00 00 03 10 20 30

The Rust and Kotlin unit tests both assert this exact vector, as well as empty, oversized, and truncated-frame rejection.
