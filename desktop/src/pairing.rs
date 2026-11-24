use std::io::{self, Read, Write};

use crate::protocol::{
    ApplicationMessage, PROTOCOL_MAJOR, PROTOCOL_MINOR, PeerRole, read_application_message,
    write_application_message,
};
use crate::security::{
    PairingGate, TrustDecision, TrustStore, certificate_fingerprint, pairing_string,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairingRequest {
    pub peer_display_name: String,
    pub comparison_string: String,
    pub peer_fingerprint: [u8; 32],
    pub reconnecting: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairingResult {
    pub peer_display_name: String,
    pub comparison_string: String,
    pub peer_fingerprint: [u8; 32],
    pub reconnecting: bool,
}

fn expected_peer_role(local_role: PeerRole) -> PeerRole {
    match local_role {
        PeerRole::Phone => PeerRole::Desktop,
        PeerRole::Desktop => PeerRole::Phone,
    }
}

fn flush_message<W: Write>(stream: &mut W, message: &ApplicationMessage) -> io::Result<()> {
    write_application_message(stream, message)?;
    stream.flush()
}

/// Completes the two-sided WiPad approval exchange over an already-authenticated TLS stream.
///
/// The TLS handshake must have verified the peer's certificate signature and, when a
/// stored pin exists, its fingerprint. This function never treats discovery metadata
/// as identity. A new certificate is stored only after both peers approve and
/// SESSION_READY has been received from the other side.
pub fn perform_pairing<S, F>(
    stream: &mut S,
    local_role: PeerRole,
    local_display_name: &str,
    local_certificate_der: &[u8],
    peer_certificate_der: &[u8],
    trust_store: &TrustStore,
    approve_local: F,
) -> io::Result<PairingResult>
where
    S: Read + Write,
    F: FnOnce(&PairingRequest) -> bool,
{
    let peer_fingerprint = certificate_fingerprint(peer_certificate_der);
    let reconnecting = match trust_store.decision(&peer_fingerprint)? {
        TrustDecision::Trusted => true,
        TrustDecision::NeedsApproval => false,
        TrustDecision::IdentityChanged => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "peer identity changed; revoke the old trust pin before pairing again",
            ));
        }
    };

    let hello = ApplicationMessage::PairingHello {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
        role: local_role,
        display_name: local_display_name.to_owned(),
    };
    flush_message(stream, &hello)?;

    let peer_hello = match read_application_message(stream)? {
        ApplicationMessage::PairingHello {
            major,
            minor,
            role,
            display_name,
        } if major == PROTOCOL_MAJOR
            && minor >= PROTOCOL_MINOR
            && role == expected_peer_role(local_role) =>
        {
            display_name
        },
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "peer sent an invalid WiPad pairing hello",
            ));
        }
    };

    let (phone_certificate, desktop_certificate) = match local_role {
        PeerRole::Phone => (local_certificate_der, peer_certificate_der),
        PeerRole::Desktop => (peer_certificate_der, local_certificate_der),
    };
    let comparison_string = pairing_string(phone_certificate, desktop_certificate)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let request = PairingRequest {
        peer_display_name: peer_hello.clone(),
        comparison_string: comparison_string.clone(),
        peer_fingerprint,
        reconnecting,
    };

    let mut gate = PairingGate::new(reconnecting);
    let local_approved = reconnecting || approve_local(&request);
    flush_message(
        stream,
        &ApplicationMessage::PairingDecision {
            approved: local_approved,
        },
    )?;

    if !local_approved {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local pairing approval was rejected",
        ));
    }
    if !reconnecting {
        gate.approve_local();
    }

    match read_application_message(stream)? {
        ApplicationMessage::PairingDecision { approved } => {
            gate.receive_peer_decision(approved);
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "peer did not send a WiPad pairing decision",
            ));
        }
    }

    if !gate.can_store_peer_pin() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer rejected pairing",
        ));
    }

    if !reconnecting {
        trust_store.store(&peer_fingerprint)?;
    }

    flush_message(stream, &ApplicationMessage::SessionReady)?;
    match read_application_message(stream)? {
        ApplicationMessage::SessionReady => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "peer did not confirm SESSION_READY",
            ));
        }
    }

    Ok(PairingResult {
        peer_display_name: peer_hello,
        comparison_string,
        peer_fingerprint,
        reconnecting,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::TlsIdentity;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::time::Duration;
    use uuid::Uuid;

    fn temp_trust_store(label: &str) -> (PathBuf, TrustStore) {
        let directory =
            std::env::temp_dir().join(format!("wipad-pairing-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let store = TrustStore::new(directory.join("trusted-peer.sha256"));
        (directory, store)
    }

    fn socket_pair() -> (UnixStream, UnixStream) {
        let (first, second) = UnixStream::pair().unwrap();
        for stream in [&first, &second] {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
        }
        (first, second)
    }

    #[test]
    fn first_contact_requires_and_persists_both_approvals() {
        let phone = TlsIdentity::generate().unwrap();
        let desktop = TlsIdentity::generate().unwrap();
        let phone_certificate = phone.certificate_der().to_vec();
        let desktop_certificate = desktop.certificate_der().to_vec();
        let expected = pairing_string(&phone_certificate, &desktop_certificate).unwrap();

        let (mut desktop_stream, mut phone_stream) = socket_pair();
        let (desktop_directory, desktop_store) = temp_trust_store("desktop-first");
        let (phone_directory, phone_store) = temp_trust_store("phone-first");
        let phone_thread_store = TrustStore::new(phone_directory.join("trusted-peer.sha256"));
        let expected_for_phone = expected.clone();

        let phone_thread = std::thread::spawn(move || {
            perform_pairing(
                &mut phone_stream,
                PeerRole::Phone,
                "WiPad Phone",
                &phone_certificate,
                &desktop_certificate,
                &phone_thread_store,
                |request| {
                    assert_eq!(request.comparison_string, expected_for_phone);
                    assert_eq!(request.peer_display_name, "WiPad Desktop");
                    assert!(!request.reconnecting);
                    true
                },
            )
            .unwrap()
        });

        let desktop_result = perform_pairing(
            &mut desktop_stream,
            PeerRole::Desktop,
            "WiPad Desktop",
            desktop.certificate_der(),
            phone.certificate_der(),
            &desktop_store,
            |request| {
                assert_eq!(request.comparison_string, expected);
                assert_eq!(request.peer_display_name, "WiPad Phone");
                assert!(!request.reconnecting);
                true
            },
        )
        .unwrap();
        let phone_result = phone_thread.join().unwrap();

        assert_eq!(
            desktop_result.comparison_string,
            phone_result.comparison_string
        );
        assert!(!desktop_result.reconnecting);
        assert!(!phone_result.reconnecting);
        assert_eq!(
            desktop_store.load().unwrap(),
            Some(certificate_fingerprint(phone.certificate_der()))
        );
        assert_eq!(
            phone_store.load().unwrap(),
            Some(certificate_fingerprint(desktop.certificate_der()))
        );

        let _ = desktop_stream.shutdown(Shutdown::Both);
        std::fs::remove_dir_all(desktop_directory).unwrap();
        std::fs::remove_dir_all(phone_directory).unwrap();
    }

    #[test]
    fn matching_saved_pins_reconnect_without_local_prompts() {
        let phone = TlsIdentity::generate().unwrap();
        let desktop = TlsIdentity::generate().unwrap();
        let phone_certificate = phone.certificate_der().to_vec();
        let desktop_certificate = desktop.certificate_der().to_vec();

        let (mut desktop_stream, mut phone_stream) = socket_pair();
        let (desktop_directory, desktop_store) = temp_trust_store("desktop-reconnect");
        let (phone_directory, phone_store) = temp_trust_store("phone-reconnect");
        desktop_store
            .store(&certificate_fingerprint(phone.certificate_der()))
            .unwrap();
        phone_store
            .store(&certificate_fingerprint(desktop.certificate_der()))
            .unwrap();

        let phone_thread = std::thread::spawn(move || {
            perform_pairing(
                &mut phone_stream,
                PeerRole::Phone,
                "WiPad Phone",
                &phone_certificate,
                &desktop_certificate,
                &phone_store,
                |_| panic!("trusted reconnect must not request local approval"),
            )
            .unwrap()
        });

        let desktop_result = perform_pairing(
            &mut desktop_stream,
            PeerRole::Desktop,
            "WiPad Desktop",
            desktop.certificate_der(),
            phone.certificate_der(),
            &desktop_store,
            |_| panic!("trusted reconnect must not request local approval"),
        )
        .unwrap();
        let phone_result = phone_thread.join().unwrap();

        assert!(desktop_result.reconnecting);
        assert!(phone_result.reconnecting);

        let _ = desktop_stream.shutdown(Shutdown::Both);
        std::fs::remove_dir_all(desktop_directory).unwrap();
        std::fs::remove_dir_all(phone_directory).unwrap();
    }

    #[test]
    fn one_side_rejection_prevents_both_sides_from_saving_a_pin() {
        let phone = TlsIdentity::generate().unwrap();
        let desktop = TlsIdentity::generate().unwrap();
        let phone_certificate = phone.certificate_der().to_vec();
        let desktop_certificate = desktop.certificate_der().to_vec();

        let (mut desktop_stream, mut phone_stream) = socket_pair();
        let (desktop_directory, desktop_store) = temp_trust_store("desktop-reject");
        let (phone_directory, phone_store) = temp_trust_store("phone-reject");
        let phone_thread_store = TrustStore::new(phone_directory.join("trusted-peer.sha256"));

        let phone_thread = std::thread::spawn(move || {
            perform_pairing(
                &mut phone_stream,
                PeerRole::Phone,
                "WiPad Phone",
                &phone_certificate,
                &desktop_certificate,
                &phone_thread_store,
                |_| true,
            )
        });

        let desktop_result = perform_pairing(
            &mut desktop_stream,
            PeerRole::Desktop,
            "WiPad Desktop",
            desktop.certificate_der(),
            phone.certificate_der(),
            &desktop_store,
            |_| false,
        );
        let phone_result = phone_thread.join().unwrap();

        assert!(desktop_result.is_err());
        assert!(phone_result.is_err());
        assert_eq!(desktop_store.load().unwrap(), None);
        assert_eq!(phone_store.load().unwrap(), None);

        let _ = desktop_stream.shutdown(Shutdown::Both);
        std::fs::remove_dir_all(desktop_directory).unwrap();
        std::fs::remove_dir_all(phone_directory).unwrap();
    }
}
