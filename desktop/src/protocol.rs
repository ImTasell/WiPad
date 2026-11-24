use std::io::{self, Read, Write};

use crate::controller::ControllerState;

/// Maximum payload size for one WiPad TCP frame.
pub const MAX_FRAME_LENGTH: usize = 4 * 1024;

/// Reads one length-prefixed frame from a byte stream.
///
/// The four-byte unsigned length is big-endian and excludes the prefix.
pub fn read_frame<R: Read>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;

    let length = u32::from_be_bytes(header) as usize;
    validate_length(length, io::ErrorKind::InvalidData)?;

    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

/// Incremental frame decoder that preserves partial headers and payloads across timeouts.
///
/// Only bytes required for the current frame are read, keeping buffered memory bounded.
#[derive(Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
    expected_payload_length: Option<usize>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns None only for a clean EOF between frames.
    ///
    /// TimedOut and WouldBlock errors leave the partial frame intact for the
    /// next call, which is essential when a session loop uses a short read timeout.
    pub fn read_next<R: Read>(&mut self, reader: &mut R) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(expected_length) = self.expected_payload_length {
                if self.buffer.len() == expected_length {
                    let payload = std::mem::take(&mut self.buffer);
                    self.expected_payload_length = None;
                    self.buffer = Vec::with_capacity(MAX_FRAME_LENGTH);
                    return Ok(Some(payload));
                }
            } else if self.buffer.len() == 4 {
                let length = u32::from_be_bytes(
                    self.buffer
                        .as_slice()
                        .try_into()
                        .expect("four-byte frame header"),
                ) as usize;
                self.buffer.clear();
                validate_length(length, io::ErrorKind::InvalidData)?;
                self.expected_payload_length = Some(length);
                continue;
            }

            let remaining = match self.expected_payload_length {
                Some(length) => length - self.buffer.len(),
                None => 4 - self.buffer.len(),
            };
            let mut chunk = [0_u8; MAX_FRAME_LENGTH];
            match reader.read(&mut chunk[..remaining]) {
                Ok(0) if self.buffer.is_empty() && self.expected_payload_length.is_none() => {
                    return Ok(None);
                }
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "WiPad frame ended before its declared length",
                    ));
                }
                Ok(read) => self.buffer.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub fn has_partial_frame(&self) -> bool {
        !self.buffer.is_empty() || self.expected_payload_length.is_some()
    }
}

/// Writes one length-prefixed frame to a byte stream.
///
/// The caller owns flushing and closing the stream.
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    validate_length(payload.len(), io::ErrorKind::InvalidInput)?;

    let length = u32::try_from(payload.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "WiPad frame length is not representable",
        )
    })?;

    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(payload)
}

fn validate_length(length: usize, kind: io::ErrorKind) -> io::Result<()> {
    if length == 0 || length > MAX_FRAME_LENGTH {
        return Err(io::Error::new(
            kind,
            format!("WiPad frame length must be between 1 and {MAX_FRAME_LENGTH} bytes"),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // Shared wire vector: payload 10 20 30 -> 00 00 00 03 10 20 30.
    const PAYLOAD: [u8; 3] = [0x10, 0x20, 0x30];
    const ENCODED: [u8; 7] = [0x00, 0x00, 0x00, 0x03, 0x10, 0x20, 0x30];

    #[test]
    fn writes_the_shared_big_endian_vector() {
        let mut encoded = Vec::new();

        write_frame(&mut encoded, &PAYLOAD).unwrap();

        assert_eq!(encoded, ENCODED);
    }

    #[test]
    fn reads_the_shared_big_endian_vector() {
        let mut encoded = Cursor::new(ENCODED);

        assert_eq!(read_frame(&mut encoded).unwrap(), PAYLOAD);
    }

    #[test]
    fn rejects_empty_frames() {
        let mut encoded = Cursor::new([0_u8; 4]);

        assert_eq!(
            read_frame(&mut encoded).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            write_frame(&mut Vec::new(), &[]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn rejects_frames_larger_than_the_limit_before_allocating() {
        let length = (MAX_FRAME_LENGTH as u32 + 1).to_be_bytes();
        let mut encoded = Cursor::new(length);

        assert_eq!(
            read_frame(&mut encoded).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let payload = vec![0_u8; MAX_FRAME_LENGTH + 1];
        assert_eq!(
            write_frame(&mut Vec::new(), &payload).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn frame_reader_preserves_partial_data_across_timeouts() {
        struct TimeoutReader {
            bytes: Vec<u8>,
            offset: usize,
            max_chunk: usize,
            timeout_after_offset: usize,
            timeouts_remaining: usize,
        }

        impl Read for TimeoutReader {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.offset >= self.timeout_after_offset && self.timeouts_remaining > 0 {
                    self.timeouts_remaining -= 1;
                    return Err(io::Error::new(io::ErrorKind::WouldBlock, "try again"));
                }

                if self.offset == self.bytes.len() {
                    return Ok(0);
                }

                let count = output
                    .len()
                    .min(self.max_chunk)
                    .min(self.bytes.len() - self.offset);
                output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
                self.offset += count;
                Ok(count)
            }
        }

        let mut reader = TimeoutReader {
            bytes: ENCODED.to_vec(),
            offset: 0,
            max_chunk: 2,
            timeout_after_offset: 2,
            timeouts_remaining: 2,
        };
        let mut decoder = FrameReader::new();

        assert_eq!(
            decoder.read_next(&mut reader).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(decoder.has_partial_frame());
        assert_eq!(
            decoder.read_next(&mut reader).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(decoder.has_partial_frame());
        assert_eq!(decoder.read_next(&mut reader).unwrap().unwrap(), PAYLOAD);
        assert!(!decoder.has_partial_frame());
        assert_eq!(decoder.read_next(&mut reader).unwrap(), None);
    }

    #[test]
    fn frame_reader_preserves_partial_payload_across_timeouts() {
        struct TimeoutReader {
            bytes: Vec<u8>,
            offset: usize,
            max_chunk: usize,
            timed_out: bool,
        }

        impl Read for TimeoutReader {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.offset >= 5 && !self.timed_out {
                    self.timed_out = true;
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "try again"));
                }
                if self.offset == self.bytes.len() {
                    return Ok(0);
                }

                let count = output
                    .len()
                    .min(self.max_chunk)
                    .min(self.bytes.len() - self.offset);
                output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
                self.offset += count;
                Ok(count)
            }
        }

        let mut reader = TimeoutReader {
            bytes: ENCODED.to_vec(),
            offset: 0,
            max_chunk: 1,
            timed_out: false,
        };
        let mut decoder = FrameReader::new();

        assert_eq!(
            decoder.read_next(&mut reader).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(decoder.has_partial_frame());
        assert_eq!(decoder.read_next(&mut reader).unwrap().unwrap(), PAYLOAD);
        assert!(!decoder.has_partial_frame());
    }

    #[test]
    fn frame_reader_rejects_oversized_header_before_payload_allocation() {
        let header = (MAX_FRAME_LENGTH as u32 + 1).to_be_bytes();
        let mut reader = Cursor::new(header);
        let mut decoder = FrameReader::new();

        assert_eq!(
            decoder.read_next(&mut reader).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn reports_a_truncated_payload() {
        let mut encoded = Cursor::new([0x00, 0x00, 0x00, 0x03, 0x10, 0x20]);

        assert_eq!(
            read_frame(&mut encoded).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn encodes_the_shared_phone_pairing_hello_vector() {
        let message = ApplicationMessage::PairingHello {
            major: 1,
            minor: 0,
            role: PeerRole::Phone,
            display_name: "Phone".to_owned(),
        };

        assert_eq!(
            encode_application_message(&message).unwrap(),
            [0x01, 0x01, 0x00, 0x01, 0x05, b'P', b'h', b'o', b'n', b'e']
        );
    }

    #[test]
    fn decodes_the_shared_phone_pairing_hello_vector() {
        let encoded = [0x01, 0x01, 0x00, 0x01, 0x05, b'P', b'h', b'o', b'n', b'e'];

        assert_eq!(
            decode_application_message(&encoded).unwrap(),
            ApplicationMessage::PairingHello {
                major: 1,
                minor: 0,
                role: PeerRole::Phone,
                display_name: "Phone".to_owned(),
            }
        );
    }

    #[test]
    fn encodes_and_decodes_controller_input_state() {
        use crate::controller::{BUTTON_NORTH, BUTTON_SOUTH, ControllerState};

        let state = ControllerState {
            sequence: 1,
            sample_time_micros: 2,
            buttons: BUTTON_SOUTH | BUTTON_NORTH,
            hat: 0,
            left_x: 0x1234,
            left_y: -2,
            right_x: 0,
            right_y: i16::MIN,
            left_trigger: u16::MAX,
            right_trigger: 0,
        };
        let message = ApplicationMessage::InputState(state);
        let expected = [
            0x10, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 9, 0, 0x12, 0x34, 0xff,
            0xfe, 0, 0, 0x80, 0, 0xff, 0xff, 0, 0,
        ];

        assert_eq!(encode_application_message(&message).unwrap(), expected);
        assert_eq!(decode_application_message(&expected).unwrap(), message);
    }

    #[test]
    fn encodes_and_decodes_controller_presence_status() {
        for (connected, expected) in [(false, [0x11, 0]), (true, [0x11, 1])] {
            let message = ApplicationMessage::ControllerStatus { connected };
            assert_eq!(encode_application_message(&message).unwrap(), expected);
            assert_eq!(decode_application_message(&expected).unwrap(), message);
        }
    }

    #[test]
    fn rejects_malformed_controller_presence_status() {
        for malformed in [&[0x11][..], &[0x11, 2][..], &[0x11, 1, 0][..]] {
            assert_eq!(
                decode_application_message(malformed).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn encodes_and_decodes_pairing_decisions_and_session_ready() {
        for approved in [false, true] {
            let message = ApplicationMessage::PairingDecision { approved };
            let expected = [MESSAGE_PAIRING_DECISION, u8::from(approved)];
            assert_eq!(encode_application_message(&message).unwrap(), expected);
            assert_eq!(decode_application_message(&expected).unwrap(), message);
        }

        assert_eq!(
            encode_application_message(&ApplicationMessage::SessionReady).unwrap(),
            [MESSAGE_SESSION_READY]
        );
        assert_eq!(
            decode_application_message(&[MESSAGE_SESSION_READY]).unwrap(),
            ApplicationMessage::SessionReady
        );
    }

    #[test]
    fn rejects_malformed_pairing_messages() {
        for payload in [
            &[MESSAGE_PAIRING_HELLO][..],
            &[MESSAGE_PAIRING_HELLO, 2, 0, 1, 1, b'P'],
            &[MESSAGE_PAIRING_HELLO, 1, 0, 9, 1, b'P'],
            &[MESSAGE_PAIRING_HELLO, 1, 0, 1, 0][..],
            &[MESSAGE_PAIRING_DECISION, 2],
            &[MESSAGE_PAIRING_DECISION, 1, 0],
            &[MESSAGE_SESSION_READY, 0],
            &[0xFF],
        ] {
            assert!(decode_application_message(payload).is_err());
        }
    }

    #[test]
    fn rejects_invalid_display_names() {
        let too_long = "x".repeat(MAX_DISPLAY_NAME_LENGTH + 1);
        let too_long_message = ApplicationMessage::PairingHello {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
            role: PeerRole::Desktop,
            display_name: too_long,
        };
        assert!(encode_application_message(&too_long_message).is_err());

        let control_name = ApplicationMessage::PairingHello {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
            role: PeerRole::Desktop,
            display_name: "WiPad\nDesktop".to_owned(),
        };
        assert!(encode_application_message(&control_name).is_err());
    }
}

pub const PROTOCOL_MAJOR: u8 = 1;
pub const PROTOCOL_MINOR: u8 = 0;
pub const MAX_DISPLAY_NAME_LENGTH: usize = 64;

const MESSAGE_PAIRING_HELLO: u8 = 0x01;
const MESSAGE_PAIRING_DECISION: u8 = 0x02;
const MESSAGE_SESSION_READY: u8 = 0x03;
const MESSAGE_INPUT_STATE: u8 = crate::controller::MESSAGE_INPUT_STATE;
const MESSAGE_CONTROLLER_STATUS: u8 = 0x11;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerRole {
    Phone,
    Desktop,
}

impl PeerRole {
    fn to_wire(self) -> u8 {
        match self {
            Self::Phone => 1,
            Self::Desktop => 2,
        }
    }

    fn from_wire(value: u8) -> io::Result<Self> {
        match value {
            1 => Ok(Self::Phone),
            2 => Ok(Self::Desktop),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad peer role is invalid",
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationMessage {
    PairingHello {
        major: u8,
        minor: u8,
        role: PeerRole,
        display_name: String,
    },
    PairingDecision {
        approved: bool,
    },
    SessionReady,
    InputState(ControllerState),
    ControllerStatus { connected: bool },
}

pub fn encode_application_message(message: &ApplicationMessage) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();

    match message {
        ApplicationMessage::PairingHello {
            major,
            minor,
            role,
            display_name,
        } => {
            let name = display_name.as_bytes();
            if *major != PROTOCOL_MAJOR
                || name.is_empty()
                || name.len() > MAX_DISPLAY_NAME_LENGTH
                || display_name.chars().any(char::is_control)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "WiPad pairing hello contains invalid version or display name",
                ));
            }

            payload.reserve(5 + name.len());
            payload.push(MESSAGE_PAIRING_HELLO);
            payload.push(*major);
            payload.push(*minor);
            payload.push(role.to_wire());
            payload.push(name.len() as u8);
            payload.extend_from_slice(name);
        }
        ApplicationMessage::InputState(state) => return state.encode_payload(),
        ApplicationMessage::ControllerStatus { connected } => {
            payload.push(MESSAGE_CONTROLLER_STATUS);
            payload.push(u8::from(*connected));
        }
        ApplicationMessage::PairingDecision { approved } => {
            payload.push(MESSAGE_PAIRING_DECISION);
            payload.push(u8::from(*approved));
        }
        ApplicationMessage::SessionReady => {
            payload.push(MESSAGE_SESSION_READY);
        }
    }

    if payload.len() > MAX_FRAME_LENGTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "WiPad application message is too large",
        ));
    }

    Ok(payload)
}

pub fn decode_application_message(payload: &[u8]) -> io::Result<ApplicationMessage> {
    if payload.is_empty() || payload.len() > MAX_FRAME_LENGTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WiPad application message length is invalid",
        ));
    }

    match payload[0] {
        MESSAGE_PAIRING_HELLO => {
            if payload.len() < 6 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WiPad pairing hello is truncated",
                ));
            }

            let major = payload[1];
            let minor = payload[2];
            if major != PROTOCOL_MAJOR {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WiPad protocol major version is unsupported",
                ));
            }

            let role = PeerRole::from_wire(payload[3])?;
            let name_length = payload[4] as usize;
            if name_length == 0
                || name_length > MAX_DISPLAY_NAME_LENGTH
                || payload.len() != 5 + name_length
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WiPad pairing hello display-name length is invalid",
                ));
            }

            let display_name = std::str::from_utf8(&payload[5..])
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "display name is not UTF-8")
                })?
                .to_owned();
            if display_name.chars().any(char::is_control) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WiPad display name contains control characters",
                ));
            }

            Ok(ApplicationMessage::PairingHello {
                major,
                minor,
                role,
                display_name,
            })
        }
        MESSAGE_INPUT_STATE => Ok(ApplicationMessage::InputState(
            ControllerState::decode_payload(payload)?,
        )),
        MESSAGE_CONTROLLER_STATUS => {
            if payload.len() != 2 || payload[1] > 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WiPad controller status is malformed",
                ));
            }
            Ok(ApplicationMessage::ControllerStatus {
                connected: payload[1] == 1,
            })
        }
        MESSAGE_PAIRING_DECISION => {
            if payload.len() != 2 || payload[1] > 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WiPad pairing decision is malformed",
                ));
            }

            Ok(ApplicationMessage::PairingDecision {
                approved: payload[1] == 1,
            })
        }
        MESSAGE_SESSION_READY => {
            if payload.len() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WiPad SESSION_READY must not have a payload",
                ));
            }

            Ok(ApplicationMessage::SessionReady)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WiPad application message type is unknown",
        )),
    }
}

pub fn read_application_message<R: Read>(reader: &mut R) -> io::Result<ApplicationMessage> {
    decode_application_message(&read_frame(reader)?)
}

pub fn write_application_message<W: Write>(
    writer: &mut W,
    message: &ApplicationMessage,
) -> io::Result<()> {
    write_frame(writer, &encode_application_message(message)?)
}
