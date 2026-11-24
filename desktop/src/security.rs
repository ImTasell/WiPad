use snow::{Builder, HandshakeState, NoiseParams, TransportState};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use crate::protocol::MAX_FRAME_LENGTH;

const NOISE_PROTOCOL: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const IDENTITY_MAGIC: &[u8; 5] = b"WIPK\x01";
const PRIVATE_KEY_LENGTH: usize = 32;
const AEAD_TAG_LENGTH: usize = 16;
const MAX_PLAINTEXT_LENGTH: usize = MAX_FRAME_LENGTH - AEAD_TAG_LENGTH;

fn noise_params() -> NoiseParams {
    NOISE_PROTOCOL
        .parse()
        .expect("WiPad's static Noise protocol name must remain valid")
}

/// Long-lived X25519 identity for this desktop installation.
///
/// Only the private key is persisted. The public key is authenticated and
/// exchanged by the Noise XX handshake, never by the discovery advertisement.
pub struct NoiseIdentity {
    private_key: Zeroizing<Vec<u8>>,
}

impl NoiseIdentity {
    /// Generates a new random Noise static key.
    pub fn generate() -> io::Result<Self> {
        let keypair = Builder::new(noise_params())
            .generate_keypair()
            .map_err(|error| io::Error::other(error.to_string()))?;

        if keypair.private.len() != PRIVATE_KEY_LENGTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Noise provider returned an unexpected private-key length",
            ));
        }

        Ok(Self {
            private_key: Zeroizing::new(keypair.private),
        })
    }

    /// Loads an existing identity or creates it if absent.
    ///
    /// The file format is a five-byte magic/version prefix followed by the
    /// 32-byte private key. Existing malformed files are rejected rather
    /// than silently replacing the installation identity.
    pub fn load_or_create(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();

        if path.exists() {
            return Self::load(path);
        }

        let identity = Self::generate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        match options.open(path) {
            Ok(mut file) => {
                let result = file
                    .write_all(IDENTITY_MAGIC)
                    .and_then(|()| file.write_all(&identity.private_key))
                    .and_then(|()| file.sync_all());

                if let Err(error) = result {
                    drop(file);
                    let _ = fs::remove_file(path);
                    return Err(error);
                }

                Ok(identity)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Self::load(path),
            Err(error) => Err(error),
        }
    }

    fn load(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad identity path must be a regular file, not a symlink",
            ));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o600);
                fs::set_permissions(path, permissions)?;
            }
        }

        let mut file = File::open(path)?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(
            IDENTITY_MAGIC.len() + PRIVATE_KEY_LENGTH,
        ));
        file.read_to_end(&mut bytes)?;

        if bytes.len() != IDENTITY_MAGIC.len() + PRIVATE_KEY_LENGTH
            || &bytes[..IDENTITY_MAGIC.len()] != IDENTITY_MAGIC
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad identity file has an unsupported format or invalid length",
            ));
        }

        Ok(Self {
            private_key: Zeroizing::new(bytes[IDENTITY_MAGIC.len()..].to_vec()),
        })
    }

    /// Returns the stable identity file path used by the desktop application.
    pub fn default_path() -> io::Result<PathBuf> {
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory"))?;

        Ok(config_root.join("wipad").join("identity.noise"))
    }

    fn handshake_builder(&self) -> Result<Builder<'_>, String> {
        Builder::new(noise_params())
            .local_private_key(&self.private_key)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandshakeRole {
    Initiator,
    Responder,
}

/// One side of a Noise XX handshake.
///
/// Call write_message or read_message according to the Noise turn. A caller
/// must not send application data until finish succeeds and the remote
/// identity has been evaluated against the local trust store.
pub struct NoiseHandshake {
    state: HandshakeState,
    failed: bool,
}

impl NoiseHandshake {
    pub fn new(identity: &NoiseIdentity, role: HandshakeRole) -> Result<Self, String> {
        let builder = identity.handshake_builder()?;
        let state = match role {
            HandshakeRole::Initiator => builder.build_initiator(),
            HandshakeRole::Responder => builder.build_responder(),
        }
        .map_err(|error| error.to_string())?;

        Ok(Self {
            state,
            failed: false,
        })
    }

    /// Creates the next handshake record. The payload is intentionally empty.
    pub fn write_message(&mut self) -> Result<Vec<u8>, String> {
        if self.failed || !self.state.is_my_turn() || self.state.is_handshake_finished() {
            return Err("Noise handshake is not ready to write".to_owned());
        }

        let mut message = [0_u8; MAX_FRAME_LENGTH];
        match self.state.write_message(&[], &mut message) {
            Ok(length) if length > 0 && length <= MAX_FRAME_LENGTH => {
                Ok(message[..length].to_vec())
            }
            Ok(_) => {
                self.failed = true;
                Err("Noise produced an invalid handshake record length".to_owned())
            }
            Err(error) => {
                self.failed = true;
                Err(error.to_string())
            }
        }
    }

    /// Consumes a handshake record. Non-empty handshake payloads are rejected.
    pub fn read_message(&mut self, message: &[u8]) -> Result<(), String> {
        if self.failed
            || self.state.is_my_turn()
            || self.state.is_handshake_finished()
            || message.is_empty()
            || message.len() > MAX_FRAME_LENGTH
        {
            self.failed = true;
            return Err("Noise handshake record is invalid for the current state".to_owned());
        }

        let mut payload = [0_u8; MAX_FRAME_LENGTH];
        match self.state.read_message(message, &mut payload) {
            Ok(0) => Ok(()),
            Ok(_) => {
                self.failed = true;
                Err("Unexpected payload in Noise handshake".to_owned())
            }
            Err(error) => {
                self.failed = true;
                Err(error.to_string())
            }
        }
    }

    pub fn is_finished(&self) -> bool {
        !self.failed && self.state.is_handshake_finished()
    }

    /// Finalizes the handshake and exposes the authenticated remote static key.
    ///
    /// The caller still has to compare remote_static with its trust store.
    /// For a new peer, both apps must compare authentication_string and
    /// explicitly approve before using the returned transport.
    pub fn finish(self) -> Result<EstablishedSession, String> {
        if self.failed || !self.state.is_handshake_finished() {
            return Err("Cannot finish an incomplete or failed Noise handshake".to_owned());
        }

        let remote_static = self
            .state
            .get_remote_static()
            .ok_or_else(|| "Noise XX did not produce a remote static identity".to_owned())?
            .to_vec();
        let handshake_hash = self.state.get_handshake_hash().to_vec();
        let authentication_string = authentication_string(&handshake_hash)?;

        let transport = self
            .state
            .into_transport_mode()
            .map_err(|error| error.to_string())?;

        Ok(EstablishedSession {
            remote_static,
            authentication_string,
            transport: NoiseTransport {
                inner: transport,
                closed: false,
            },
        })
    }
}

/// Session metadata and encrypted record transport after a completed handshake.
pub struct EstablishedSession {
    pub remote_static: Vec<u8>,
    pub authentication_string: String,
    pub transport: NoiseTransport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustDecision {
    NeedsApproval,
    Trusted,
    IdentityChanged,
}

/// Evaluates a presented Noise static key against the single trusted peer.
pub fn evaluate_trust(trusted: Option<&[u8]>, observed: &[u8]) -> TrustDecision {
    match trusted {
        None => TrustDecision::NeedsApproval,
        Some(key) if key == observed => TrustDecision::Trusted,
        Some(_) => TrustDecision::IdentityChanged,
    }
}

/// Creates a 48-bit comparison string from the symmetric Noise handshake hash.
///
/// The value is for visual comparison on both devices, not a secret or a PIN.
pub fn authentication_string(handshake_hash: &[u8]) -> Result<String, String> {
    if handshake_hash.len() < 6 {
        return Err("Noise handshake hash is too short".to_owned());
    }

    Ok(format!(
        "{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}",
        handshake_hash[0],
        handshake_hash[1],
        handshake_hash[2],
        handshake_hash[3],
        handshake_hash[4],
        handshake_hash[5],
    ))
}

/// Encrypted Noise transport for the reliable, ordered TCP session.
pub struct NoiseTransport {
    inner: TransportState,
    closed: bool,
}

impl NoiseTransport {
    /// Encrypts one bounded application record.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        if self.closed {
            return Err("Noise transport is closed".to_owned());
        }
        if plaintext.is_empty() || plaintext.len() > MAX_PLAINTEXT_LENGTH {
            return Err(format!(
                "WiPad plaintext length must be between 1 and {MAX_PLAINTEXT_LENGTH} bytes"
            ));
        }

        let mut ciphertext = [0_u8; MAX_FRAME_LENGTH];
        match self.inner.write_message(plaintext, &mut ciphertext) {
            Ok(length) if length > 0 && length <= MAX_FRAME_LENGTH => {
                Ok(ciphertext[..length].to_vec())
            }
            Ok(_) => {
                self.closed = true;
                Err("Noise produced an invalid ciphertext length".to_owned())
            }
            Err(error) => {
                self.closed = true;
                Err(error.to_string())
            }
        }
    }

    /// Authenticates and decrypts one application record.
    ///
    /// Any malformed record or authentication failure poisons this transport;
    /// callers must close the underlying connection and discard the session.
    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        if self.closed {
            return Err("Noise transport is closed".to_owned());
        }
        if ciphertext.len() <= AEAD_TAG_LENGTH || ciphertext.len() > MAX_FRAME_LENGTH {
            self.closed = true;
            return Err("Encrypted WiPad record has an invalid length".to_owned());
        }

        let mut plaintext = [0_u8; MAX_FRAME_LENGTH];
        match self.inner.read_message(ciphertext, &mut plaintext) {
            Ok(length) if length > 0 && length <= MAX_PLAINTEXT_LENGTH => {
                Ok(plaintext[..length].to_vec())
            }
            Ok(_) => {
                self.closed = true;
                Err("Decrypted WiPad record has an invalid length".to_owned())
            }
            Err(error) => {
                self.closed = true;
                Err(error.to_string())
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;


    fn perform_xx_handshake() -> (EstablishedSession, EstablishedSession) {
        let phone_identity = NoiseIdentity::generate().unwrap();
        let desktop_identity = NoiseIdentity::generate().unwrap();

        let mut phone = NoiseHandshake::new(&phone_identity, HandshakeRole::Initiator).unwrap();
        let mut desktop = NoiseHandshake::new(&desktop_identity, HandshakeRole::Responder).unwrap();

        let message_one = phone.write_message().unwrap();
        desktop.read_message(&message_one).unwrap();

        let message_two = desktop.write_message().unwrap();
        phone.read_message(&message_two).unwrap();

        let message_three = phone.write_message().unwrap();
        desktop.read_message(&message_three).unwrap();

        assert!(phone.is_finished());
        assert!(desktop.is_finished());

        (phone.finish().unwrap(), desktop.finish().unwrap())
    }

    #[test]
    fn xx_handshake_authenticates_both_static_keys_and_derives_same_hash() {
        let (phone, desktop) = perform_xx_handshake();

        assert_eq!(
            phone.remote_static,
            desktop.transport.inner.get_remote_static().unwrap()
        );
        assert_eq!(
            desktop.remote_static,
            phone.transport.inner.get_remote_static().unwrap()
        );
        assert_eq!(phone.authentication_string, desktop.authentication_string);
        assert_eq!(phone.authentication_string.len(), 14);
    }

    #[test]
    fn trust_evaluation_distinguishes_new_trusted_and_changed_keys() {
        let (phone, desktop) = perform_xx_handshake();

        assert_eq!(
            evaluate_trust(None, &phone.remote_static),
            TrustDecision::NeedsApproval
        );
        assert_eq!(
            evaluate_trust(Some(&phone.remote_static), &phone.remote_static),
            TrustDecision::Trusted
        );
        assert_eq!(
            evaluate_trust(Some(&phone.remote_static), &desktop.remote_static),
            TrustDecision::IdentityChanged
        );
    }

    #[test]
    fn transport_encrypts_authenticates_and_rejects_tampering() {
        let (mut phone, mut desktop) = perform_xx_handshake();

        let ciphertext = phone.transport.seal(b"controller-state").unwrap();
        assert_ne!(&ciphertext, b"controller-state");
        assert_eq!(
            desktop.transport.open(&ciphertext).unwrap(),
            b"controller-state"
        );

        let mut corrupted = phone.transport.seal(b"next-state").unwrap();
        corrupted[0] ^= 0x80;

        assert!(desktop.transport.open(&corrupted).is_err());
        assert!(desktop.transport.is_closed());
        assert!(desktop.transport.open(&corrupted).is_err());
    }

    #[test]
    fn identity_is_persisted_with_restricted_permissions_and_reloaded() {
        let directory = std::env::temp_dir().join(format!("wipad-identity-{}", Uuid::new_v4()));
        let path = directory.join("identity.noise");

        let first = NoiseIdentity::load_or_create(&path).unwrap();
        let original_private = first.private_key.to_vec();
        drop(first);

        let second = NoiseIdentity::load_or_create(&path).unwrap();
        assert_eq!(second.private_key.as_slice(), original_private.as_slice());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_identity_is_not_silently_replaced() {
        let directory =
            std::env::temp_dir().join(format!("wipad-invalid-identity-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("identity.noise");
        fs::write(&path, b"not-a-wipad-identity").unwrap();

        let result = NoiseIdentity::load_or_create(&path);
        assert!(matches!(
            result,
            Err(error) if error.kind() == io::ErrorKind::InvalidData
        ));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn authentication_string_rejects_short_hashes() {
        assert!(authentication_string(&[1, 2, 3, 4, 5]).is_err());
    }
}
