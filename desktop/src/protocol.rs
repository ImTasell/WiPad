use std::io::{self, Read, Write};

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
