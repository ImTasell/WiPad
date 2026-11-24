use std::io;

pub const MESSAGE_INPUT_STATE: u8 = 0x10;
pub const INPUT_STATE_PAYLOAD_LENGTH: usize = 34;

pub const BUTTON_SOUTH: u32 = 1 << 0;
pub const BUTTON_EAST: u32 = 1 << 1;
pub const BUTTON_WEST: u32 = 1 << 2;
pub const BUTTON_NORTH: u32 = 1 << 3;
pub const BUTTON_LEFT_BUMPER: u32 = 1 << 4;
pub const BUTTON_RIGHT_BUMPER: u32 = 1 << 5;
pub const BUTTON_BACK: u32 = 1 << 6;
pub const BUTTON_START: u32 = 1 << 7;
pub const BUTTON_GUIDE: u32 = 1 << 8;
pub const BUTTON_LEFT_STICK: u32 = 1 << 9;
pub const BUTTON_RIGHT_STICK: u32 = 1 << 10;

pub const BUTTON_MASK: u32 = (1 << 11) - 1;

/// A complete normalized controller snapshot. Axis signs and ranges are platform-independent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControllerState {
    pub sequence: u64,
    pub sample_time_micros: u64,
    pub buttons: u32,
    /// 0=center, then clockwise: up, up-right, right, down-right, down, down-left, left, up-left.
    pub hat: u8,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
    pub left_trigger: u16,
    pub right_trigger: u16,
}

impl ControllerState {
    pub fn neutral(sequence: u64, sample_time_micros: u64) -> Self {
        Self {
            sequence,
            sample_time_micros,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.sequence == 0 {
            return Err(invalid("controller sequence must start at 1"));
        }
        if self.buttons & !BUTTON_MASK != 0 {
            return Err(invalid("controller snapshot contains unknown button bits"));
        }
        if self.hat > 8 {
            return Err(invalid("controller hat value must be in 0..=8"));
        }
        Ok(())
    }

    pub fn encode_payload(&self) -> io::Result<Vec<u8>> {
        self.validate()?;

        let mut payload = Vec::with_capacity(INPUT_STATE_PAYLOAD_LENGTH);
        payload.push(MESSAGE_INPUT_STATE);
        payload.extend_from_slice(&self.sequence.to_be_bytes());
        payload.extend_from_slice(&self.sample_time_micros.to_be_bytes());
        payload.extend_from_slice(&self.buttons.to_be_bytes());
        payload.push(self.hat);
        payload.extend_from_slice(&self.left_x.to_be_bytes());
        payload.extend_from_slice(&self.left_y.to_be_bytes());
        payload.extend_from_slice(&self.right_x.to_be_bytes());
        payload.extend_from_slice(&self.right_y.to_be_bytes());
        payload.extend_from_slice(&self.left_trigger.to_be_bytes());
        payload.extend_from_slice(&self.right_trigger.to_be_bytes());
        debug_assert_eq!(payload.len(), INPUT_STATE_PAYLOAD_LENGTH);
        Ok(payload)
    }

    pub fn decode_payload(payload: &[u8]) -> io::Result<Self> {
        if payload.len() != INPUT_STATE_PAYLOAD_LENGTH || payload[0] != MESSAGE_INPUT_STATE {
            return Err(invalid(
                "controller snapshot length or message type is invalid",
            ));
        }

        let state = Self {
            sequence: read_u64(payload, 1),
            sample_time_micros: read_u64(payload, 9),
            buttons: u32::from_be_bytes(payload[17..21].try_into().expect("fixed slice")),
            hat: payload[21],
            left_x: i16::from_be_bytes(payload[22..24].try_into().expect("fixed slice")),
            left_y: i16::from_be_bytes(payload[24..26].try_into().expect("fixed slice")),
            right_x: i16::from_be_bytes(payload[26..28].try_into().expect("fixed slice")),
            right_y: i16::from_be_bytes(payload[28..30].try_into().expect("fixed slice")),
            left_trigger: u16::from_be_bytes(payload[30..32].try_into().expect("fixed slice")),
            right_trigger: u16::from_be_bytes(payload[32..34].try_into().expect("fixed slice")),
        };
        state.validate()?;
        Ok(state)
    }
}

fn read_u64(payload: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(payload[offset..offset + 8].try_into().expect("fixed slice"))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Rejects duplicate and older snapshots within one authenticated session.
#[derive(Default, Debug)]
pub struct InputSequenceTracker {
    last_sequence: Option<u64>,
}

impl InputSequenceTracker {
    pub fn accept(&mut self, state: &ControllerState) -> io::Result<bool> {
        state.validate()?;
        if self
            .last_sequence
            .is_some_and(|previous| state.sequence <= previous)
        {
            return Ok(false);
        }

        self.last_sequence = Some(state.sequence);
        Ok(true)
    }

    pub fn reset(&mut self) {
        self.last_sequence = None;
    }

    pub fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARED_VECTOR: [u8; INPUT_STATE_PAYLOAD_LENGTH] = [
        0x10, // INPUT_STATE
        0, 0, 0, 0, 0, 0, 0, 1, // sequence = 1
        0, 0, 0, 0, 0, 0, 0, 2, // sample_time_micros = 2
        0, 0, 0, 9, // buttons = south + north
        0, // hat = centered
        0x12, 0x34, // left_x = 4660
        0xff, 0xfe, // left_y = -2
        0, 0, // right_x = 0
        0x80, 0, // right_y = -32768
        0xff, 0xff, // left_trigger = 65535
        0, 0, // right_trigger = 0
    ];

    fn sample(sequence: u64) -> ControllerState {
        ControllerState {
            sequence,
            sample_time_micros: 2,
            buttons: BUTTON_SOUTH | BUTTON_NORTH,
            hat: 0,
            left_x: 0x1234,
            left_y: -2,
            right_x: 0,
            right_y: i16::MIN,
            left_trigger: u16::MAX,
            right_trigger: 0,
        }
    }

    #[test]
    fn encodes_the_shared_controller_state_vector() {
        assert_eq!(sample(1).encode_payload().unwrap(), SHARED_VECTOR);
    }

    #[test]
    fn decodes_the_shared_controller_state_vector() {
        assert_eq!(
            ControllerState::decode_payload(&SHARED_VECTOR).unwrap(),
            sample(1)
        );
    }

    #[test]
    fn rejects_invalid_snapshot_fields() {
        let mut state = sample(1);
        state.sequence = 0;
        assert!(state.encode_payload().is_err());

        state = sample(1);
        state.buttons = BUTTON_MASK + 1;
        assert!(state.encode_payload().is_err());

        state = sample(1);
        state.hat = 9;
        assert!(state.encode_payload().is_err());

        let mut malformed = SHARED_VECTOR;
        malformed[21] = 9;
        assert!(ControllerState::decode_payload(&malformed).is_err());
    }

    #[test]
    fn rejects_wrong_lengths_and_message_types() {
        assert!(ControllerState::decode_payload(&SHARED_VECTOR[..33]).is_err());

        let mut malformed = SHARED_VECTOR;
        malformed[0] = 0x11;
        assert!(ControllerState::decode_payload(&malformed).is_err());
    }

    #[test]
    fn sequence_tracker_rejects_duplicates_and_out_of_order_states() {
        let mut tracker = InputSequenceTracker::default();

        assert!(tracker.accept(&sample(1)).unwrap());
        assert!(!tracker.accept(&sample(1)).unwrap());
        assert!(tracker.accept(&sample(3)).unwrap());
        assert!(!tracker.accept(&sample(2)).unwrap());
        assert_eq!(tracker.last_sequence(), Some(3));

        tracker.reset();
        assert!(tracker.accept(&sample(1)).unwrap());
    }
}
