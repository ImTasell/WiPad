# WiPad Controller State

Status: Draft 0.1  
Wire message: \`INPUT_STATE\` (`0x10`)  
Platforms: Android 11+ (Kotlin) and Linux (Rust)

An \`INPUT_STATE\` message is a complete snapshot. It replaces the previously applied state; it is not a sequence of button-down/up events. The desktop accepts only strictly increasing sequence numbers within the current authenticated session.

## Payload layout

All multi-byte integers are big-endian. The payload is exactly 34 bytes:

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 1 | Message type | \`0x10\` |
| 1 | 8 | Sequence | Unsigned 64-bit; starts at 1 for each session |
| 9 | 8 | Sample time | Unsigned 64-bit microseconds since the phone's session start |
| 17 | 4 | Buttons | Unsigned 32-bit bitset |
| 21 | 1 | D-pad hat | 0–8 |
| 22 | 2 | Left X | Signed 16-bit |
| 24 | 2 | Left Y | Signed 16-bit |
| 26 | 2 | Right X | Signed 16-bit |
| 28 | 2 | Right Y | Signed 16-bit |
| 30 | 2 | Left trigger | Unsigned 16-bit |
| 32 | 2 | Right trigger | Unsigned 16-bit |

### Button bit assignments

| Bit | Meaning | Linux evdev mapping |
| ---: | --- | --- |
| 0 | South / A | \`BTN_SOUTH\` |
| 1 | East / B | \`BTN_EAST\` |
| 2 | West / X | \`BTN_WEST\` |
| 3 | North / Y | \`BTN_NORTH\` |
| 4 | Left bumper | \`BTN_TL\` |
| 5 | Right bumper | \`BTN_TR\` |
| 6 | Back / Select | \`BTN_SELECT\` |
| 7 | Start | \`BTN_START\` |
| 8 | Guide | \`BTN_MODE\` |
| 9 | Left-stick click | \`BTN_THUMBL\` |
| 10 | Right-stick click | \`BTN_THUMBR\` |

Bits 11–31 are reserved and must be zero. Android key codes never cross the network; Kotlin maps platform events to these logical bits.

### Hat encoding

| Value | Direction |
| ---: | --- |
| 0 | Center |
| 1 | Up |
| 2 | Up-right |
| 3 | Right |
| 4 | Down-right |
| 5 | Down |
| 6 | Down-left |
| 7 | Left |
| 8 | Up-left |

The Linux receiver maps the hat to \`ABS_HAT0X\` and \`ABS_HAT0Y\`, using -1, 0 and 1.

### Axis conventions

- Stick axes are signed 16-bit values. Zero is center; the negative and positive endpoints are -32768 and 32767.
- Stick Y follows the Linux gamepad convention: negative is up, positive is down.
- Trigger axes are unsigned 16-bit values. Zero is released; 65535 is fully pressed.
- Unknown button bits, invalid hat values, zero sequence numbers, malformed lengths and unknown message types are rejected.
- Sample time is diagnostic metadata only. The receiver uses its own monotonic clock for freshness and timeout enforcement.

## Update cadence and receiver freshness

The phone initially sends complete snapshots at 20 Hz (one frame every 50 ms), including unchanged states. This avoids a held button becoming stale simply because the player is not moving. The desktop uses its own monotonic clock and releases all controls after 750 ms without a valid, strictly newer snapshot. Session start and session teardown both begin/end in a neutral state.

## Shared conformance vector

Both implementations encode and decode the following payload byte-for-byte:

\`10 00 00 00 00 00 00 00 01 00 00 00 00 00 00 00 02 00 00 00 09 00 12 34 FF FE 00 00 80 00 FF FF 00 00\`

Decoded values: sequence 1, sample time 2 µs, south+north buttons, centered hat, left stick (4660, -2), right stick (0, -32768), left trigger 65535, right trigger 0.

The Rust and Kotlin test suites load this vector and shared malformed-input fixtures from [protocol/golden-vectors.txt](../protocol/golden-vectors.txt). Keep the canonical bytes in that file rather than duplicating them in platform tests.
