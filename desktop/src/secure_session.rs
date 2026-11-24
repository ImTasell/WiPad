use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};
use std::fmt;

use crate::protocol::MAX_FRAME_LENGTH;

pub const NOISE_PROTOCOL: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const NOISE_TAG_LENGTH: usize = 16;
const MAX_PLAINTEXT_LENGTH: usize = MAX_FRAME_LENGTH - NOISE_TAG_LENGTH;

#[derive(Debug)]
pub enum SecureSessionError {
    Noise(snow::Error),
    InvalidIdentityKey,
    HandshakeIncomplete,
    MissingPeerIdentity,
    ApprovalRequired,
    IdentityChanged,
    MessageTooLarge,
}

impl fmt::Display for SecureSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Noise(error) => write!(f, "Noise protocol error: {error}"),
            Self::InvalidIdentityKey => write!(f, "invalid X25519 identity key"),
            Self::HandshakeIncomplete => write!(f, "Noise handshake is not complete"),
            Self::MissingPeerIdentity => write!(f, "Noise handshake did not authenticate a peer key"),
            Self::ApprovalRequired => write!(f, "first-contact peer approval is required"),
            Self::IdentityChanged => write!(f, "peer identity changed and requires approval"),
            Self::MessageTooLarge => write!(f, "secure message exceeds the WiPad frame limit"),
        }
    }
}

impl std::error::Error for SecureSessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Noise(error) => Some(error),
            _ => None,
        }
    }
}

impl From<snow::Error> for SecureSessionError {
    fn from(error: snow::Error) -> Self {
        Self::Noise(error)
    }
}

/// Persistent X25519 identity material for one WiPad installation.
///
/// Store this locally with owner-only file permissions. The private key is never
/// included in mDNS advertisements or protocol messages.
pub struct IdentityKeyPair {
    private_key: [u8; 32],
    public_key: [u8; 32],
}

impl IdentityKeyPair {
    pub fn generate() -> Result<Self, SecureSessionError> {
        let params: NoiseParams = NOISE_PROTOCOL.parse().map_err(SecureSessionError::Noise)?;
        let keypair = Builder::new(params).generate_keypair()?;
        let private_key = keypair
            .private
            .as_slice()
            .try_into()
            .map_err(|_| SecureSessionError::InvalidIdentityKey)?;
        let public_key = keypair
            .public
            .as_slice()
            .try_into()
            .map_err(|_| SecureSessionError::InvalidIdentityKey)?;

        Ok(Self {
            private_key,
            public_key,
        })
    }

    pub fn from_bytes(private_key: [u8; 32], public_key: [u8; 32]) -> Self {
        Self {
            private_key,
            public_key,
        }
    }

    pub fn private_key(&self) -> &[u8; 32] {
        &self.private_key
    }

    pub fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }
}

impl Drop for IdentityKeyPair {
    fn drop(&mut self) {
        self.private_key.fill(0);
    }
}

/// A single in-progress Noise XX handshake.
pub struct NoiseHandshake {
    state: HandshakeState,
}

impl NoiseHandshake {
    pub fn initiator(identity: &IdentityKeyPair) -> Result<Self, SecureSessionError> {
        let params: NoiseParams = NOISE_PROTOCOL.parse().map_err(SecureSessionError::Noise)?;
        let state = Builder::new(params)
            .local_private_key(identity.private_key())?
            .build_initiator()?;
        Ok(Self { state })
    }

    pub fn responder(identity: &IdentityKeyPair) -> Result<Self, SecureSessionError> {
        let params: NoiseParams = NOISE_PROTOCOL.parse().map_err(SecureSessionError::Noise)?;
        let state = Builder::new(params)
            .local_private_key(identity.private_key())?
            .build_responder()?;
        Ok(Self { state })
    }

    pub fn is_my_turn(&self) -> bool {
        self.state.is_my_turn()
    }

    pub fn is_finished(&self) -> bool {
        self.state.is_handshake_finished()
    }

    pub fn write_message(&mut self, payload: &[u8]) -> Result<Vec<u8>, SecureSessionError> {
        if payload.len() > MAX_PLAINTEXT_LENGTH {
            return Err(SecureSessionError::MessageTooLarge);
        }

        let mut message = vec![0_u8; MAX_FRAME_LENGTH];
        let written = self.state.write_message(payload, &mut message)?;
        message.truncate(written);
        Ok(message)
    }

    pub fn read_message(&mut self, message: &[u8]) -> Result<Vec<u8>, SecureSessionError> {
        if message.is_empty() || message.len() > MAX_FRAME_LENGTH {
            return Err(SecureSessionError::MessageTooLarge);
        }

        let mut payload = vec![0_u8; MAX_FRAME_LENGTH];
        let written = self.state.read_message(message, &mut payload)?;
        payload.truncate(written);
        Ok(payload)
    }

    /// Completes the cryptographic handshake but does not yet enable application traffic.
    ///
    /// The caller must explicitly approve a new peer or verify a previously trusted key
    /// through PendingSession::approve before a SecureSession can be obtained.
    pub fn finish(self) -> Result<PendingSession, SecureSessionError> {
        if !self.state.is_handshake_finished() {
            return Err(SecureSessionError::HandshakeIncomplete);
        }

        let remote_static = self
            .state
            .get_remote_static()
            .ok_or(SecureSessionError::MissingPeerIdentity)?;
        let peer_public_key = remote_static
            .try_into()
            .map_err(|_| SecureSessionError::InvalidIdentityKey)?;
        let handshake_hash = self.state.get_handshake_hash().to_vec();
        let transport = self.state.into_transport_mode()?;

        Ok(PendingSession {
            peer_public_key,
            handshake_hash,
            transport,
        })
    }
}

/// A cryptographically complete connection that has not yet passed the trust decision.
pub struct PendingSession {
    peer_public_key: [u8; 32],
    handshake_hash: Vec<u8>,
    transport: TransportState,
}

impl PendingSession {
    pub fn peer_public_key(&self) -> &[u8; 32] {
        &self.peer_public_key
    }

    pub fn handshake_hash(&self) -> &[u8] {
        &self.handshake_hash
    }

    /// Activates traffic only after a first-contact approval or an exact trusted-key match.
    ///
    /// A mismatched saved key is never silently accepted. The caller can replace it only
    /// when explicit_first_contact_approval represents a deliberate user approval.
    pub fn approve(
        self,
        previously_trusted_peer: Option<&[u8; 32]>,
        explicit_first_contact_approval: bool,
    ) -> Result<SecureSession, SecureSessionError> {
        match previously_trusted_peer {
            Some(trusted) if trusted == &self.peer_public_key => {}
            Some(_) | None if !explicit_first_contact_approval => {
                return Err(if previously_trusted_peer.is_some() {
                    SecureSessionError::IdentityChanged
                } else {
                    SecureSessionError::ApprovalRequired
                });
            }
            Some(_) | None => {}
        }

        Ok(SecureSession {
            peer_public_key: self.peer_public_key,
            handshake_hash: self.handshake_hash,
            transport: self.transport,
        })
    }
}

/// An approved, encrypted, ordered Noise transport session.
pub struct SecureSession {
    peer_public_key: [u8; 32],
    handshake_hash: Vec<u8>,
    transport: TransportState,
}

impl SecureSession {
    pub fn peer_public_key(&self) -> &[u8; 32] {
        &self.peer_public_key
    }

    pub fn handshake_hash(&self) -> &[u8] {
        &self.handshake_hash
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SecureSessionError> {
        if plaintext.len() > MAX_PLAINTEXT_LENGTH {
            return Err(SecureSessionError::MessageTooLarge);
        }

        let mut ciphertext = vec![0_u8; MAX_FRAME_LENGTH];
        let written = self.transport.write_message(plaintext, &mut ciphertext)?;
        ciphertext.truncate(written);
        Ok(ciphertext)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, SecureSessionError> {
        if ciphertext.len() <= NOISE_TAG_LENGTH || ciphertext.len() > MAX_FRAME_LENGTH {
            return Err(SecureSessionError::MessageTooLarge);
        }

        let mut plaintext = vec![0_u8; MAX_FRAME_LENGTH];
        let written = self.transport.read_message(ciphertext, &mut plaintext)?;
        plaintext.truncate(written);
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_xx_handshake() -> (PendingSession, PendingSession, IdentityKeyPair, IdentityKeyPair) {
        let phone = IdentityKeyPair::generate().unwrap();
        let desktop = IdentityKeyPair::generate().unwrap();

        let mut phone_handshake = NoiseHandshake::initiator(&phone).unwrap();
        let mut desktop_handshake = NoiseHandshake::responder(&desktop).unwrap();

        let first = phone_handshake.write_message(&[]).unwrap();
        assert!(desktop_handshake.read_message(&first).unwrap().is_empty());

        let second = desktop_handshake.write_message(&[]).unwrap();
        assert!(phone_handshake.read_message(&second).unwrap().is_empty());

        let third = phone_handshake.write_message(&[]).unwrap();
        assert!(desktop_handshake.read_message(&third).unwrap().is_empty());

        assert!(phone_handshake.is_finished());
        assert!(desktop_handshake.is_finished());

        (
            phone_handshake.finish().unwrap(),
            desktop_handshake.finish().unwrap(),
            phone,
            desktop,
        )
    }

    #[test]
    fn xx_handshake_authenticates_both_static_identities() {
        let (phone_pending, desktop_pending, phone, desktop) = complete_xx_handshake();

        assert_eq!(phone_pending.peer_public_key(), desktop.public_key());
        assert_eq!(desktop_pending.peer_public_key(), phone.public_key());
        assert_eq!(
            phone_pending.handshake_hash(),
            desktop_pending.handshake_hash()
        );
    }

    #[test]
    fn first_contact_requires_explicit_approval() {
        let (phone_pending, _, _, _) = complete_xx_handshake();

        assert!(matches!(
            phone_pending.approve(None, false),
            Err(SecureSessionError::ApprovalRequired)
        ));
    }

    #[test]
    fn a_saved_identity_reconnects_without_approval() {
        let (phone_pending, desktop_pending, phone, desktop) = complete_xx_handshake();

        let phone_session = phone_pending.approve(Some(desktop.public_key()), false);
        let desktop_session = desktop_pending.approve(Some(phone.public_key()), false);

        assert!(phone_session.is_ok());
        assert!(desktop_session.is_ok());
    }

    #[test]
    fn a_changed_identity_requires_reapproval() {
        let (phone_pending, _, _, _) = complete_xx_handshake();
        let unexpected_peer = IdentityKeyPair::generate().unwrap();

        assert!(matches!(
            phone_pending.approve(Some(unexpected_peer.public_key()), false),
            Err(SecureSessionError::IdentityChanged)
        ));
    }

    #[test]
    fn explicit_reapproval_can_replace_a_changed_identity() {
        let (phone_pending, _, _, _) = complete_xx_handshake();
        let old_peer = IdentityKeyPair::generate().unwrap();

        let session = phone_pending.approve(Some(old_peer.public_key()), true);
        assert!(session.is_ok());
    }

    #[test]
    fn encrypted_transport_is_bidirectional() {
        let (phone_pending, desktop_pending, phone, desktop) = complete_xx_handshake();
        let mut phone_session = phone_pending
            .approve(Some(desktop.public_key()), false)
            .unwrap();
        let mut desktop_session = desktop_pending
            .approve(Some(phone.public_key()), false)
            .unwrap();

        let encrypted = phone_session.encrypt(b"controller-state").unwrap();
        assert_ne!(encrypted, b"controller-state");
        assert_eq!(
            desktop_session.decrypt(&encrypted).unwrap(),
            b"controller-state"
        );

        let reply = desktop_session.encrypt(b"pong").unwrap();
        assert_eq!(phone_session.decrypt(&reply).unwrap(), b"pong");
    }

    #[test]
    fn transport_rejects_tampered_ciphertext() {
        let (phone_pending, desktop_pending, phone, desktop) = complete_xx_handshake();
        let mut phone_session = phone_pending
            .approve(Some(desktop.public_key()), false)
            .unwrap();
        let mut desktop_session = desktop_pending
            .approve(Some(phone.public_key()), false)
            .unwrap();

        let mut encrypted = phone_session.encrypt(b"input").unwrap();
        encrypted[0] ^= 0x80;

        assert!(desktop_session.decrypt(&encrypted).is_err());
    }

    #[test]
    fn transport_enforces_the_frame_limit() {
        let (phone_pending, desktop_pending, phone, desktop) = complete_xx_handshake();
        let mut phone_session = phone_pending
            .approve(Some(desktop.public_key()), false)
            .unwrap();
        let mut desktop_session = desktop_pending
            .approve(Some(phone.public_key()), false)
            .unwrap();

        assert!(matches!(
            phone_session.encrypt(&vec![0_u8; MAX_PLAINTEXT_LENGTH + 1]),
            Err(SecureSessionError::MessageTooLarge)
        ));
        assert!(matches!(
            desktop_session.decrypt(&vec![0_u8; MAX_FRAME_LENGTH + 1]),
            Err(SecureSessionError::MessageTooLarge)
        ));
    }
}
