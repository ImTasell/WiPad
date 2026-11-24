# WiPad Design System

Status: Initial design specification  
Scope: Android app (Kotlin + Jetpack Compose) and Linux desktop app (Rust + Slint)

WiPad uses a Material-inspired visual language with a blue primary identity, cyan supporting accent, calm neutral surfaces, and clear connection-state feedback. The Android implementation follows Material 3. The desktop implementation uses Slint's Material style where practical, with WiPad tokens providing cross-platform consistency.

## 1. Design principles

- **Clarity first:** the current controller, connection state, and next useful action should be immediately obvious.
- **One active connection:** the interface is designed around one phone, one controller, and one desktop session.
- **State is explicit:** Bluetooth, Wi-Fi, pairing, and virtual-controller status must not be conflated.
- **Calm, purposeful motion:** animate meaningful state changes; avoid decorative or continuous animation.
- **Accessible by default:** maintain readable contrast, clear labels, keyboard/focus support on desktop, and screen-reader semantics on Android.
- **Lightweight:** avoid unnecessary visual effects, large animated assets, and expensive background work.

## 2. Brand palette

The initial brand palette is:

| Token | Hex | Intended use |
| --- | --- | --- |
| Blue 800 | `#1565C0` | Strong primary actions and emphasis |
| Blue 500 | `#2196F3` | Primary brand accent |
| Cyan 500 | `#00BCD4` | Supporting accent and highlights |
| Cyan 200 | `#80DEEA` | Subtle accent surfaces and decorative details |
| Navy 950 | `#0D1B2A` | Deep dark-theme base reference |

These are brand anchors, not permission to use raw hex values throughout the UI. Components should consume semantic theme roles.

### Semantic color roles

Map the brand palette into the platform's semantic color system:

- **Primary:** prominent actions, active navigation, selected controls.
- **On-primary:** text and icons placed on primary-colored components.
- **Primary container / on-primary-container:** lower-emphasis primary actions and selected surfaces.
- **Secondary:** supporting emphasis; cyan is the preferred supporting brand hue.
- **Tertiary:** occasional contrast accent, not a second primary.
- **Surface / surface container:** cards, panels, sheets, and grouped content.
- **Background:** the app's main canvas.
- **Outline / outline variant:** borders, dividers, and subdued boundaries.
- **Error:** failed operations and actionable faults.
- **Success (WiPad semantic token):** connected/ready state.
- **Warning (WiPad semantic token):** degraded or attention-needed state.

Status must never be communicated by color alone. Pair it with text and, where useful, an icon.

### Light and dark themes

Both themes are first-class and must be tested.

- Light theme: soft neutral background, white or near-white elevated surfaces, dark readable text.
- Dark theme: deep slate/navy background, lighter neutral surface layers, high-contrast text.
- Avoid pure black as the default dark background and avoid using the brand blue as a large page background.
- Use tonal surface hierarchy before adding shadows.
- Ensure every foreground/background pairing meets WCAG AA contrast where applicable.

Android 11 (API 30) is the baseline, so WiPad must ship complete static light and dark color schemes. Android 12+ may optionally offer wallpaper-based dynamic color, but dynamic color must not be required for correct branding or usability.

## 3. Typography

Use the platform's native Material typography where available. Keep hierarchy compact and scannable.

| Role | Typical use |
| --- | --- |
| Display | Rare, large hero value only |
| Headline | Screen title or major status |
| Title | Section and card headings |
| Body | Explanations, descriptions, and settings |
| Label | Buttons, tabs, compact metadata, and status labels |

Guidance:
- Prefer body and title styles for most of the application.
- Use tabular numerals for latency, packet rate, and other changing measurements when supported.
- Do not use font size alone to communicate importance; combine size, weight, and spacing.
- Respect Android font scaling and desktop text scaling.

## 4. Shape and spacing

Use a restrained Material shape scale. Small controls should not look like oversized pills; reserve fully rounded shapes for components whose role calls for them, such as compact chips.

Use a **4 dp base spacing unit** on Android and an equivalent logical-pixel rhythm on desktop.

Suggested spacing tokens:

| Token | Size | Use |
| --- | ---: | --- |
| space-1 | 4 | Tight icon/text gaps |
| space-2 | 8 | Compact component gaps |
| space-3 | 12 | Related content |
| space-4 | 16 | Standard padding |
| space-6 | 24 | Section spacing |
| space-8 | 32 | Major separation |
| space-12 | 48 | Large layout separation |

Do not force every component to use the same padding. Use these values as a consistent scale.

## 5. Components and interaction

Prefer standard Material 3 components on Android instead of recreating their behavior.

- **Primary button:** one clear next action per screen or section.
- **Secondary / outlined button:** alternative actions.
- **Text button:** low-emphasis actions.
- **Cards and surface containers:** group related information; avoid nesting cards without a clear reason.
- **Chips:** compact filters or small state labels, not substitutes for full status explanations.
- **Dialogs:** reserve for decisions that need focused attention.
- **Snackbars:** brief confirmation or recoverable feedback; do not use them for persistent connection state.

Use filled, tonal, outlined, and text button emphasis consistently. Do not make every action visually primary.

### Touch, keyboard, and assistive technology

- Android interactive targets should be at least 48 × 48 dp.
- Give icon-only controls meaningful content descriptions.
- Preserve logical TalkBack traversal and visible focus indication.
- Desktop controls must be keyboard reachable and have clear focus states.
- Do not rely on color, animation, or icon shape alone to communicate meaning.

## 6. Navigation and responsive layout

Navigation adapts to available space rather than a specific device label.

### Android

- Compact widths: use a Material 3 navigation bar for the top-level destinations (keep the destination count small).
- Wider layouts: consider a navigation rail when it improves use of available space.
- Keep the primary connection action and current connection state easy to find.
- Avoid duplicating the same navigation destinations in multiple navigation components.

### Linux desktop

- Use a persistent navigation sidebar or rail for top-level sections.
- Keep the main content area focused on the selected section.
- Use a secondary details area only when it adds useful context; do not make it mandatory.
- Design for resizable windows and avoid fixed widths that make the UI unusable on smaller displays.

Suggested top-level information architecture (subject to refinement during implementation):
1. **Home:** overall readiness and the next action.
2. **Controller:** Bluetooth controller discovery and controller status.
3. **Connection:** desktop pairing/session and Wi-Fi link status.
4. **Settings:** network, behavior, appearance, and diagnostics.

Do not present these as separate connection technologies if the user only needs one guided setup flow. The information architecture is a starting point, not a requirement to add unnecessary screens.

## 7. Connection and runtime states

Use consistent state names and explanations across both apps:

| State | Meaning | Presentation |
| --- | --- | --- |
| Disconnected | No active WiPad session | Neutral status; explain how to connect |
| Discovering | Looking for the configured peer/controller | Progress indicator with a concise label |
| Pairing | Establishing or confirming a relationship | Explicit pairing step and clear cancel path |
| Connecting | Establishing the Wi-Fi session | Progress indicator; avoid false success |
| Connected | Session is active | Success color plus visible text |
| Degraded | Connected, but quality or delivery is impaired | Warning color plus actionable explanation |
| Reconnecting | Recovering an interrupted session | Progress feedback and current recovery status |
| Error | An operation failed | Error explanation and a recovery action |

Bluetooth controller status and Wi-Fi session status are distinct facts. The UI must not show the whole system as connected merely because the controller is paired over Bluetooth.

## 8. Motion

Motion should explain what changed:
- Animate a state transition when it helps users follow connection progress.
- Use short, restrained transitions for navigation and showing/hiding supporting content.
- Avoid perpetual pulsing, animated backgrounds, and motion that does not communicate useful information.
- Respect reduced-motion preferences where available.
- Keep animations optional and inexpensive on lower-end Android devices.

## 9. Platform implementation notes

### Android — Kotlin + Jetpack Compose

- Use Material 3 Compose components and a centralized `MaterialTheme`.
- Define complete light and dark `ColorScheme` values for the Android 11 baseline.
- Keep WiPad-specific semantic tokens in one theme layer; do not scatter color literals through composables.
- Use adaptive layouts based on available window size.
- Dynamic color is optional on Android 12+ only.

### Linux desktop — Rust + Slint

- Start with Slint's Material style as the visual baseline.
- Use the light/dark Material style variants where supported by the chosen Slint version.
- Centralize WiPad-specific colors, spacing, and component conventions rather than hardcoding values in individual views.
- Match the Android hierarchy and semantics, not necessarily every platform-specific pixel or component behavior.
- Prefer native desktop interaction patterns where they improve keyboard and pointer usability.

Slint's Material style is a starting point; it should not be assumed to provide exact Compose Material 3 parity.

## 10. Out of scope

This document does not define:
- The network transport or wire protocol.
- Bluetooth discovery and controller mapping behavior.
- Pairing security or authentication.
- Exact screen contents or final navigation labels.
- Final accessibility audit results.

Those belong in the architecture, protocol, and implementation specifications.

## 11. Reference documentation

- [Material 3 in Compose](https://developer.android.com/develop/ui/compose/designsystems/material3)
- [Material 3 adaptive navigation](https://developer.android.com/develop/ui/compose/layouts/adaptive)
- [Compose accessibility](https://developer.android.com/develop/ui/compose/accessibility)
- [Slint built-in styles](https://slint.dev/releases/)

---
This is the initial shared design contract. Update it when implementation reveals a concrete cross-platform need; avoid changing tokens casually or introducing one-off styles without a reason.
