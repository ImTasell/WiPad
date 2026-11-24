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
        io::Error::new(io::ErrorKind::InvalidInput, "WiPad frame length is not representable")
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
}
