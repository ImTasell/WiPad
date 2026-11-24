# WiPad Development Guide

Status: Initial contributor/build guide  
Repository: `ImTasell/WiPad`

This guide describes the expected development workflow for the Android and Linux applications. It intentionally avoids pinning library versions or choosing networking/cryptography dependencies before those decisions are tested and documented.

## 1. Project targets

| Component | Technology | Initial target |
| --- | --- | --- |
| Android app | Kotlin, Jetpack Compose, Material 3 | Android 11 (API 30) and newer |
| Android ABI | Android application toolchain | `armeabi-v7a` (32-bit ARM) |
| Linux desktop | Rust, Slint | Linux `x86_64` |
| Virtual controller | Linux `uinput` | One virtual gamepad |
| Inter-app communication | Versioned WiPad protocol | Local network |

There is no shared native library, JNI, or FFI boundary. Android platform code stays in Kotlin; Linux platform code stays in Rust. The protocol specification is the contract between them.

## 2. Repository layout

Keep the first implementation small. Add modules and crates only when a boundary becomes useful.

```text
WiPad/
├── android/
│   └── app/                 # Kotlin + Compose application
├── desktop/
│   ├── Cargo.toml
│   └── src/                 # Rust + Slint application
├── docs/                    # Supporting design/implementation notes
├── ARCHITECTURE.md
├── DESIGN_SYSTEM.md
├── PROTOCOL.md
├── DEVELOPMENT.md
└── LICENSE
```

The Android build currently uses Gradle 9.6.0, Android Gradle Plugin 9.5.0, Kotlin 2.4.10, JDK 17, and Compose BOM 2026.09.00. The desktop uses Rust 1.95+ and Slint 1.19.0. These are initial pinned versions and should be upgraded deliberately. A Gradle wrapper has not been checked in yet; CI installs the pinned Gradle version directly. The desktop `Cargo.lock` should be generated and committed once the first dependency resolution succeeds.

## 3. Android development

### Requirements

- JDK 17.
- Android SDK Platform for the configured compile SDK.
- Android SDK Build-Tools and platform tools.
- Android Gradle Plugin 9.5.0 and Gradle 9.6.0.
- An Android 11+ device or emulator for testing.

The minimum supported Android version is API 30; the current target and compile SDK are both API 36. The app explicitly filters to `armeabi-v7a` for the initial 32-bit ARM target.

### Build and test

From the repository root, once the Android project has been scaffolded:

```bash
cd android
./gradlew :app:assembleDebug
./gradlew :app:testDebugUnitTest
./gradlew :app:lintDebug
```

The exact task names may change if the project later adds Gradle modules or flavors. The Gradle wrapper is the canonical build entry point; do not depend on a globally installed Gradle version.

### Android compatibility checks

- Keep the application compatible with Android 11/API 30.
- Handle Android 12+ Bluetooth runtime permission differences explicitly.
- Request only permissions required for the active feature.
- Test Bluetooth scanning and connection on a real device; emulator Bluetooth behavior is not a substitute.
- Test app lifecycle behavior when backgrounded, screen-off, process recreation, and network changes.
- Build and inspect the `armeabi-v7a` APK on actual 32-bit ARM hardware.
- Keep background discovery and reconnect behavior within Android's execution and foreground-service rules.

Do not assume a foreground service can run without a visible notification or without the required user action. The service lifecycle must be designed against the Android version and target SDK actually selected.

## 4. Linux desktop development

### Requirements

- Rust stable toolchain with `rustup`.
- Cargo (installed with Rust).
- Linux development libraries required by the selected Slint backend and window-system integration.
- Access to a Linux system for testing `uinput`.

Install Rust using the official rustup distribution for your system. Keep the toolchain on stable unless a documented Slint or platform requirement forces a different channel.

### Build, format, lint, and test

From the repository root, once the desktop package exists:

```bash
cd desktop
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
```

Use `cargo fmt` to apply formatting. Do not suppress Clippy warnings globally; resolve them or document a narrowly scoped exception.

### Linux virtual gamepad access

The desktop app must check whether `/dev/uinput` exists and is accessible before claiming that the virtual controller is ready.

- Do not run the whole GUI application as root.
- Do not silently change system permissions.
- Document the supported distribution-specific permission setup after it has been tested.
- Keep a clear diagnostic for missing kernel support, missing device nodes, or insufficient permissions.
- Test device creation, button/axis updates, neutralization, and cleanup.
- Always release all controls on disconnect, timeout, shutdown, or fatal input errors.

A development environment without `uinput` access can still build and run UI/protocol tests. Hardware integration tests should report a clear skip or failure reason rather than pretending the virtual device exists.

## 5. Local development on constrained hardware

WiPad should remain practical to build on modest machines.

- Prefer command-line builds over requiring a heavyweight IDE.
- Use Gradle's daemon and build cache where appropriate, but avoid parallel build settings that exhaust available RAM.
- Use Cargo's incremental compilation during development.
- Do not add large dependencies without a concrete need.
- Keep generated build outputs out of Git.
- Use CI for expensive or cross-ABI builds when local disk, memory, or bandwidth is limited.
- Avoid repeatedly downloading Android SDK images or toolchains; pin versions and use CI caches.

The project should not require AVX2 or other newer x86 CPU features for its Linux x86_64 build. Do not enable `target-cpu=native` in release CI artifacts.

## 6. Continuous integration

GitHub Actions should validate both platform projects independently, then run shared protocol checks.

### Android CI

- Set up the pinned JDK.
- Set up the configured Android SDK platform and build tools.
- Enable Gradle dependency/build caching.
- Run unit tests, lint, and debug assembly.
- Build the required `armeabi-v7a` artifact.
- Upload APKs as workflow artifacts for testing; do not publish releases automatically from ordinary pull requests.

### Linux CI

- Use a supported Ubuntu runner for the initial Linux build.
- Install only the system packages required by the selected Slint backend.
- Install the current stable Rust toolchain (1.95+).
- Cache Cargo registry, Git dependencies, and build artifacts.
- Run formatting, check, tests, Clippy, and release build.
- Keep hardware-dependent `uinput` tests separate from ordinary CI unless the runner can provide a safe test environment.

### Protocol CI

- Keep language-neutral test vectors under a documented location such as `protocol/test-vectors/`.
- Validate the same vectors from Kotlin and Rust.
- Test malformed lengths, unsupported versions, invalid values, duplicate/stale sequences, and session-state violations.
- Never use production private keys or real user credentials in CI.

CI should use least-privilege permissions. Pin third-party Actions to reviewed versions (prefer immutable commit SHAs for security-sensitive workflows) and do not expose repository secrets to untrusted pull-request code.

## 7. Code and dependency conventions

- Keep Kotlin formatting and lint configuration consistent across the Android project.
- Keep Rust formatted with `rustfmt` and linted with Clippy.
- Pin toolchain/plugin/dependency versions in project configuration.
- Commit `Cargo.lock` for the desktop application.
- Do not commit local SDK paths, signing keys, keystores, private identity keys, or generated device data.
- Keep secrets out of logs and crash reports.
- Prefer maintained libraries with clear license, security, and Android/Linux support.
- Do not implement custom cryptography.
- Do not introduce JNI/FFI to share code between the two applications.

## 8. Testing strategy

### Unit tests

Test pure logic without requiring Bluetooth, Wi-Fi, or `uinput` hardware:

- Protocol serialization and parsing.
- Version negotiation and capability handling.
- Controller normalization and axis ranges.
- Sequence-number ordering and stale-packet rejection.
- Reconnect/backoff state transitions.
- Peer trust and identity-change decisions.
- Neutral controller-state generation.

### Integration tests

Test platform boundaries independently:

- Android Bluetooth discovery, pairing, reconnect, and controller disconnect.
- Android LAN service registration/browsing and network changes.
- Linux mDNS discovery and session lifecycle.
- Linux `uinput` device creation, updates, neutralization, and teardown.
- Android-to-Linux protocol interoperability on a real local network.

### Failure scenarios

Every milestone should test the failure path, not only the happy path:

- Phone or desktop process exits unexpectedly.
- Wi-Fi disconnects or changes access point.
- The peer disappears during input.
- The Bluetooth controller powers off or leaves range.
- A previously trusted identity changes.
- A malformed or unauthenticated peer sends data.
- The desktop cannot access `/dev/uinput`.
- Discovery is blocked by client isolation or multicast filtering.

The virtual gamepad must return to neutral when its session is no longer valid.

## 9. Decisions intentionally left open

Do not add dependencies or implementation assumptions for these until they are evaluated and recorded:

- Exact Android Gradle Plugin, Gradle, Kotlin, and compile/target SDK versions.
- Exact Rust/Slint versions and selected window/rendering backend.
- Android Bluetooth controller API strategy for the target controller.
- mDNS/DNS-SD libraries for Kotlin and Rust.
- Authenticated handshake, key storage, and cryptographic libraries.
- Session transport and serialization format.
- Input update rate and timeout values.
- Distribution-specific `uinput` permissions and packaging.

When one of these decisions is made, update the relevant design document and pin the chosen version in the project.

---

This guide is a starting point for a reproducible, lightweight development workflow. Keep it aligned with the actual build files as the applications are scaffolded.
