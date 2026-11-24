use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, UinputAbsSetup,
};

use crate::controller::{
    BUTTON_BACK, BUTTON_EAST, BUTTON_GUIDE, BUTTON_LEFT_BUMPER, BUTTON_LEFT_STICK, BUTTON_NORTH,
    BUTTON_RIGHT_BUMPER, BUTTON_RIGHT_STICK, BUTTON_SOUTH, BUTTON_START, BUTTON_WEST,
    ControllerState,
};

const BUTTON_MAPPING: [(u32, KeyCode); 11] = [
    (BUTTON_SOUTH, KeyCode::BTN_SOUTH),
    (BUTTON_EAST, KeyCode::BTN_EAST),
    (BUTTON_WEST, KeyCode::BTN_WEST),
    (BUTTON_NORTH, KeyCode::BTN_NORTH),
    (BUTTON_LEFT_BUMPER, KeyCode::BTN_TL),
    (BUTTON_RIGHT_BUMPER, KeyCode::BTN_TR),
    (BUTTON_BACK, KeyCode::BTN_SELECT),
    (BUTTON_START, KeyCode::BTN_START),
    (BUTTON_GUIDE, KeyCode::BTN_MODE),
    (BUTTON_LEFT_STICK, KeyCode::BTN_THUMBL),
    (BUTTON_RIGHT_STICK, KeyCode::BTN_THUMBR),
];

/// One Linux virtual gamepad with a fixed, versioned capability set.
///
/// The device is created only when requested. Dropping it destroys the uinput
/// device; callers should release controls before dropping an active session.
pub struct VirtualGamepad {
    device: VirtualDevice,
}

impl VirtualGamepad {
    pub fn create() -> io::Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for (_, code) in BUTTON_MAPPING {
            keys.insert(code);
        }

        let builder = VirtualDevice::builder().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "cannot open /dev/uinput; check kernel support and user permissions: {error}"
                ),
            )
        })?;
        let builder = builder.name(&"WiPad Virtual Gamepad").with_keys(&keys)?;

        let builder = builder
            .with_absolute_axis(&axis(
                AbsoluteAxisCode::ABS_X,
                i16::MIN as i32,
                i16::MAX as i32,
            ))?
            .with_absolute_axis(&axis(
                AbsoluteAxisCode::ABS_Y,
                i16::MIN as i32,
                i16::MAX as i32,
            ))?
            .with_absolute_axis(&axis(
                AbsoluteAxisCode::ABS_RX,
                i16::MIN as i32,
                i16::MAX as i32,
            ))?
            .with_absolute_axis(&axis(
                AbsoluteAxisCode::ABS_RY,
                i16::MIN as i32,
                i16::MAX as i32,
            ))?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_Z, 0, u16::MAX as i32))?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_RZ, 0, u16::MAX as i32))?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_HAT0X, -1, 1))?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_HAT0Y, -1, 1))?;

        Ok(Self {
            device: builder.build()?,
        })
    }

    /// Applies the complete normalized state. Every button and axis is emitted,
    /// so the kernel's state is always overwritten by one authoritative snapshot.
    pub fn apply_state(&mut self, state: &ControllerState) -> io::Result<()> {
        let events = state_events(state)?
            .into_iter()
            .map(|(event_type, code, value)| InputEvent::new(event_type, code, value))
            .collect::<Vec<_>>();
        self.device.emit(&events)
    }

    /// Releases every button and returns every axis to neutral.
    pub fn release_all(&mut self) -> io::Result<()> {
        self.apply_state(&ControllerState::neutral(1, 0))
    }

    pub fn syspath(&mut self) -> io::Result<std::path::PathBuf> {
        self.device.get_syspath()
    }
}

impl Drop for VirtualGamepad {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}

fn state_events(state: &ControllerState) -> io::Result<Vec<(u16, u16, i32)>> {
    state.validate()?;
    let (hat_x, hat_y) = hat_axes(state.hat);
    let mut events = Vec::with_capacity(BUTTON_MAPPING.len() + 8);

    for (bit, key) in BUTTON_MAPPING {
        events.push((EventType::KEY.0, key.0, i32::from(state.buttons & bit != 0)));
    }

    events.extend([
        (
            EventType::ABSOLUTE.0,
            AbsoluteAxisCode::ABS_X.0,
            state.left_x as i32,
        ),
        (
            EventType::ABSOLUTE.0,
            AbsoluteAxisCode::ABS_Y.0,
            state.left_y as i32,
        ),
        (
            EventType::ABSOLUTE.0,
            AbsoluteAxisCode::ABS_RX.0,
            state.right_x as i32,
        ),
        (
            EventType::ABSOLUTE.0,
            AbsoluteAxisCode::ABS_RY.0,
            state.right_y as i32,
        ),
        (
            EventType::ABSOLUTE.0,
            AbsoluteAxisCode::ABS_Z.0,
            state.left_trigger as i32,
        ),
        (
            EventType::ABSOLUTE.0,
            AbsoluteAxisCode::ABS_RZ.0,
            state.right_trigger as i32,
        ),
        (EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_HAT0X.0, hat_x),
        (EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_HAT0Y.0, hat_y),
    ]);

    Ok(events)
}

fn axis(code: AbsoluteAxisCode, minimum: i32, maximum: i32) -> UinputAbsSetup {
    UinputAbsSetup::new(code, AbsInfo::new(0, minimum, maximum, 0, 0, 0))
}

fn hat_axes(hat: u8) -> (i32, i32) {
    match hat {
        1 => (0, -1),
        2 => (1, -1),
        3 => (1, 0),
        4 => (1, 1),
        5 => (0, 1),
        6 => (-1, 1),
        7 => (-1, 0),
        8 => (-1, -1),
        _ => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_snapshots_emit_every_button_and_axis() {
        use crate::controller::{BUTTON_RIGHT_BUMPER, BUTTON_SOUTH};

        let state = ControllerState {
            sequence: 7,
            sample_time_micros: 99,
            buttons: BUTTON_SOUTH | BUTTON_RIGHT_BUMPER,
            hat: 2,
            left_x: -123,
            left_y: 456,
            right_x: i16::MIN,
            right_y: i16::MAX,
            left_trigger: 12_345,
            right_trigger: u16::MAX,
        };
        let events = state_events(&state).unwrap();

        assert_eq!(events.len(), BUTTON_MAPPING.len() + 8);
        assert_eq!(events[0], (EventType::KEY.0, KeyCode::BTN_SOUTH.0, 1));
        assert_eq!(events[1], (EventType::KEY.0, KeyCode::BTN_EAST.0, 0));
        assert_eq!(events[5], (EventType::KEY.0, KeyCode::BTN_TR.0, 1));
        assert_eq!(
            events[BUTTON_MAPPING.len()],
            (EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, -123)
        );
        assert_eq!(
            events[BUTTON_MAPPING.len() + 1],
            (EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, 456)
        );
        assert_eq!(
            events[BUTTON_MAPPING.len() + 4],
            (EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Z.0, 12_345)
        );
        assert_eq!(
            events[BUTTON_MAPPING.len() + 5],
            (
                EventType::ABSOLUTE.0,
                AbsoluteAxisCode::ABS_RZ.0,
                u16::MAX as i32
            )
        );
        assert_eq!(
            events[BUTTON_MAPPING.len() + 6],
            (EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_HAT0X.0, 1)
        );
        assert_eq!(
            events[BUTTON_MAPPING.len() + 7],
            (EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_HAT0Y.0, -1)
        );
    }

    #[test]
    fn invalid_controller_state_never_produces_kernel_events() {
        let invalid = ControllerState::neutral(0, 0);
        assert!(state_events(&invalid).is_err());
    }

    #[test]
    fn maps_all_hat_directions_to_linux_axis_values() {
        let expected = [
            (0, 0),
            (0, -1),
            (1, -1),
            (1, 0),
            (1, 1),
            (0, 1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
        ];

        for (hat, axes) in expected.into_iter().enumerate() {
            assert_eq!(hat_axes(hat as u8), axes);
        }
    }

    #[test]
    fn button_mapping_covers_the_entire_wire_button_set() {
        let mapped_mask = BUTTON_MAPPING
            .iter()
            .fold(0_u32, |mask, (bit, _)| mask | *bit);

        assert_eq!(mapped_mask, crate::controller::BUTTON_MASK);
    }

    #[test]
    fn all_capability_axes_have_the_expected_ranges() {
        let stick = axis(AbsoluteAxisCode::ABS_X, i16::MIN as i32, i16::MAX as i32);
        let trigger = axis(AbsoluteAxisCode::ABS_Z, 0, u16::MAX as i32);
        let hat = axis(AbsoluteAxisCode::ABS_HAT0X, -1, 1);

        assert_eq!(stick.absinfo().minimum(), i16::MIN as i32);
        assert_eq!(stick.absinfo().maximum(), i16::MAX as i32);
        assert_eq!(trigger.absinfo().minimum(), 0);
        assert_eq!(trigger.absinfo().maximum(), u16::MAX as i32);
        assert_eq!(hat.absinfo().minimum(), -1);
        assert_eq!(hat.absinfo().maximum(), 1);
    }
}
