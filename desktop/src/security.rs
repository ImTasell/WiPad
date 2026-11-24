use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error as TlsError, ServerConfig,
    SignatureScheme, crypto::CryptoProvider,
};
use rustls::{crypto, version};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;
use x509_parser::parse_x509_certificate;
use zeroize::Zeroizing;

const IDENTITY_MAGIC: &[u8; 5] = b"WIPI\x02";
const TRUST_MAGIC: &[u8; 5] = b"WIPR\x01";
const KEY_KIND_EC_PUBLIC_KEY: &str = "1.2.840.10045.2.1";
const CURVE_P256: &str = "1.2.840.10045.3.1.7";
const MAX_CERTIFICATE_LENGTH: usize = 8 * 1024;
const MAX_PRIVATE_KEY_LENGTH: usize = 4 * 1024;
const PAIRING_DOMAIN: &[u8] = b"WiPad TLS peer comparison v1\0";

/// Stable local P-256 TLS identity.
///
/// The desktop private key is stored as PKCS#8 and protected by owner-only
/// filesystem permissions. The Android peer uses Android Keystore instead.
pub struct TlsIdentity {
    certificate_der: Vec<u8>,
    private_key_pkcs8: Zeroizing<Vec<u8>>,
}

impl TlsIdentity {
    pub fn generate() -> io::Result<Self> {
        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
            .map_err(|error| io::Error::other(error.to_string()))?;

        let mut params = CertificateParams::new(vec!["wipad.local".to_owned()])
            .map_err(|error| io::Error::other(error.to_string()))?;
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(3650);

        let certificate = params
            .self_signed(&key_pair)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let certificate_der = certificate.der().to_vec();
        let private_key_pkcs8 = Zeroizing::new(key_pair.serialize_der());

        validate_peer_certificate(&certificate_der, None)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;

        Ok(Self {
            certificate_der,
            private_key_pkcs8,
        })
    }

    /// Loads a persisted identity or creates it atomically when absent.
    ///
    /// File format: magic/version, certificate length (u32 BE), private-key
    /// length (u32 BE), certificate DER, PKCS#8 private-key DER.
    pub fn load_or_create(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();

        match fs::symlink_metadata(path) {
            Ok(_) => return Self::load(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
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
                let certificate_length =
                    u32::try_from(identity.certificate_der.len()).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "certificate too large")
                    })?;
                let key_length = u32::try_from(identity.private_key_pkcs8.len()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "private key too large")
                })?;

                let result = file
                    .write_all(IDENTITY_MAGIC)
                    .and_then(|()| file.write_all(&certificate_length.to_be_bytes()))
                    .and_then(|()| file.write_all(&key_length.to_be_bytes()))
                    .and_then(|()| file.write_all(&identity.certificate_der))
                    .and_then(|()| file.write_all(&identity.private_key_pkcs8))
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

        if metadata.len() as usize
            > IDENTITY_MAGIC.len() + 8 + MAX_CERTIFICATE_LENGTH + MAX_PRIVATE_KEY_LENGTH
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad identity file exceeds the size limit",
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
        let mut bytes = Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
        file.read_to_end(&mut bytes)?;

        let header_length = IDENTITY_MAGIC.len() + 8;
        if bytes.len() < header_length || &bytes[..IDENTITY_MAGIC.len()] != IDENTITY_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad identity file has an unsupported format",
            ));
        }

        let certificate_length = u32::from_be_bytes(
            bytes[IDENTITY_MAGIC.len()..IDENTITY_MAGIC.len() + 4]
                .try_into()
                .expect("four-byte certificate length"),
        ) as usize;
        let key_length = u32::from_be_bytes(
            bytes[IDENTITY_MAGIC.len() + 4..header_length]
                .try_into()
                .expect("four-byte private-key length"),
        ) as usize;

        if certificate_length == 0
            || certificate_length > MAX_CERTIFICATE_LENGTH
            || key_length == 0
            || key_length > MAX_PRIVATE_KEY_LENGTH
            || bytes.len() != header_length + certificate_length + key_length
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad identity file has invalid component lengths",
            ));
        }

        let certificate_start = header_length;
        let key_start = certificate_start + certificate_length;
        let certificate_der = bytes[certificate_start..key_start].to_vec();
        let private_key_pkcs8 = Zeroizing::new(bytes[key_start..].to_vec());

        validate_peer_certificate(&certificate_der, None)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;

        Ok(Self {
            certificate_der,
            private_key_pkcs8,
        })
    }

    pub fn default_path() -> io::Result<PathBuf> {
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory"))?;

        Ok(config_root.join("wipad").join("identity.tls"))
    }

    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate_der
    }

    fn private_key_der(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.private_key_pkcs8.to_vec()))
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        certificate_fingerprint(&self.certificate_der)
    }
}

/// Computes a SHA-256 fingerprint over the exact DER certificate bytes.
pub fn certificate_fingerprint(certificate_der: &[u8]) -> [u8; 32] {
    Sha256::digest(certificate_der).into()
}

/// Returns the same comparison string on both devices when passed the same
/// phone certificate and desktop certificate, in that role order.
pub fn pairing_string(
    phone_certificate_der: &[u8],
    desktop_certificate_der: &[u8],
) -> Result<String, String> {
    if phone_certificate_der.is_empty()
        || desktop_certificate_der.is_empty()
        || phone_certificate_der.len() > MAX_CERTIFICATE_LENGTH
        || desktop_certificate_der.len() > MAX_CERTIFICATE_LENGTH
    {
        return Err("Pairing certificate length is invalid".to_owned());
    }

    let phone_fingerprint = certificate_fingerprint(phone_certificate_der);
    let desktop_fingerprint = certificate_fingerprint(desktop_certificate_der);
    let mut hash = Sha256::new();
    hash.update(PAIRING_DOMAIN);
    hash.update(phone_fingerprint);
    hash.update(desktop_fingerprint);
    let digest = hash.finalize();

    Ok(format!(
        "{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5]
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustDecision {
    NeedsApproval,
    Trusted,
    IdentityChanged,
}

pub fn evaluate_trust(
    trusted_fingerprint: Option<&[u8; 32]>,
    observed_fingerprint: &[u8; 32],
) -> TrustDecision {
    match trusted_fingerprint {
        None => TrustDecision::NeedsApproval,
        Some(expected) if expected == observed_fingerprint => TrustDecision::Trusted,
        Some(_) => TrustDecision::IdentityChanged,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingState {
    AwaitingBothApprovals,
    AwaitingLocalApproval,
    AwaitingPeerApproval,
    Ready,
    Rejected,
}

/// Two-sided first-contact approval gate.
///
/// A local trust-pin match counts as the local side's approval. The peer must
/// still send its own approval decision over the authenticated TLS stream.
/// Persist a new peer pin only after this gate reaches Ready.
#[derive(Clone, Debug)]
pub struct PairingGate {
    local_approved: bool,
    peer_approved: bool,
    rejected: bool,
}

impl PairingGate {
    pub fn new(local_pin_matches: bool) -> Self {
        Self {
            local_approved: local_pin_matches,
            peer_approved: false,
            rejected: false,
        }
    }

    pub fn approve_local(&mut self) {
        if !self.rejected {
            self.local_approved = true;
        }
    }

    pub fn receive_peer_decision(&mut self, approved: bool) {
        if self.rejected {
            return;
        }

        if approved {
            self.peer_approved = true;
        } else {
            self.rejected = true;
        }
    }

    pub fn state(&self) -> PairingState {
        if self.rejected {
            PairingState::Rejected
        } else {
            match (self.local_approved, self.peer_approved) {
                (true, true) => PairingState::Ready,
                (false, true) => PairingState::AwaitingLocalApproval,
                (true, false) => PairingState::AwaitingPeerApproval,
                (false, false) => PairingState::AwaitingBothApprovals,
            }
        }
    }

    pub fn can_store_peer_pin(&self) -> bool {
        self.state() == PairingState::Ready
    }

    pub fn can_start_input(&self) -> bool {
        self.can_store_peer_pin()
    }
}

/// Single-peer certificate pin store.
///
/// The store operation is the explicit approval boundary. Discovery and TLS
/// verification must never write this file automatically.
pub struct TrustStore {
    path: PathBuf,
}

/// Revokes the desktop's remembered phone identity.
///
/// A future connection is treated as first contact and must be explicitly approved.
pub fn revoke_trusted_peer() -> io::Result<()> {
    TrustStore::new(TrustStore::default_path()?).revoke()
}

impl TrustStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_path() -> io::Result<PathBuf> {
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory"))?;

        Ok(config_root.join("wipad").join("trusted-peer.sha256"))
    }

    pub fn load(&self) -> io::Result<Option<[u8; 32]>> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };

        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 37 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad trust record is not a valid regular file",
            ));
        }

        let mut bytes = [0_u8; 37];
        File::open(&self.path)?.read_exact(&mut bytes)?;
        if &bytes[..5] != TRUST_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WiPad trust record has an unsupported format",
            ));
        }

        let mut fingerprint = [0_u8; 32];
        fingerprint.copy_from_slice(&bytes[5..]);
        Ok(Some(fingerprint))
    }

    pub fn store(&self, fingerprint: &[u8; 32]) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file_name = self
            .path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid trust path"))?
            .to_string_lossy();
        let temporary_path = self
            .path
            .with_file_name(format!("{file_name}.tmp-{}", Uuid::new_v4()));

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(&temporary_path)?;
        let result = file
            .write_all(TRUST_MAGIC)
            .and_then(|()| file.write_all(fingerprint))
            .and_then(|()| file.sync_all());
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }
        drop(file);

        if let Err(error) = fs::rename(&temporary_path, &self.path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }

        if let Some(parent) = self.path.parent()
            && let Ok(directory) = File::open(parent)
        {
            directory.sync_all()?;
        }

        Ok(())
    }

    pub fn decision(&self, observed: &[u8; 32]) -> io::Result<TrustDecision> {
        Ok(evaluate_trust(self.load()?.as_ref(), observed))
    }

    pub fn revoke(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn tls_provider() -> Arc<CryptoProvider> {
    Arc::new(crypto::ring::default_provider())
}

fn validate_peer_certificate(
    certificate_der: &[u8],
    expected_fingerprint: Option<&[u8; 32]>,
) -> Result<[u8; 32], TlsError> {
    if certificate_der.is_empty() || certificate_der.len() > MAX_CERTIFICATE_LENGTH {
        return Err(TlsError::General(
            "peer certificate length is invalid".to_owned(),
        ));
    }

    let (remaining, certificate) = parse_x509_certificate(certificate_der)
        .map_err(|_| TlsError::General("peer certificate is malformed".to_owned()))?;
    if !remaining.is_empty() || !certificate.tbs_certificate.validity().is_valid() {
        return Err(TlsError::General(
            "peer certificate encoding or validity is invalid".to_owned(),
        ));
    }

    let public_key_algorithm = certificate
        .tbs_certificate
        .subject_pki
        .algorithm
        .algorithm
        .to_id_string();
    let curve = certificate
        .tbs_certificate
        .subject_pki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|parameters| parameters.as_oid().ok())
        .map(|oid| oid.to_id_string());

    if public_key_algorithm != KEY_KIND_EC_PUBLIC_KEY || curve.as_deref() != Some(CURVE_P256) {
        return Err(TlsError::General(
            "peer certificate must use an ECDSA P-256 public key".to_owned(),
        ));
    }

    let fingerprint: [u8; 32] = Sha256::digest(certificate_der).into();
    if expected_fingerprint.is_some_and(|expected| expected != &fingerprint) {
        return Err(TlsError::General(
            "peer certificate does not match the trusted identity".to_owned(),
        ));
    }

    Ok(fingerprint)
}

struct PeerCertificateVerifier {
    provider: Arc<CryptoProvider>,
    expected_fingerprint: Option<[u8; 32]>,
}

impl fmt::Debug for PeerCertificateVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerCertificateVerifier")
            .field("has_pin", &self.expected_fingerprint.is_some())
            .finish()
    }
}

impl PeerCertificateVerifier {
    fn new(expected_fingerprint: Option<[u8; 32]>) -> Self {
        Self {
            provider: tls_provider(),
            expected_fingerprint,
        }
    }

    fn validate(&self, certificate: &CertificateDer<'_>) -> Result<(), TlsError> {
        validate_peer_certificate(certificate.as_ref(), self.expected_fingerprint.as_ref())?;
        Ok(())
    }

    fn verify_tls13(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        crypto::verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }
}

impl ServerCertVerifier for PeerCertificateVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if !intermediates.is_empty() {
            return Err(TlsError::General(
                "WiPad expects a single peer identity certificate".to_owned(),
            ));
        }
        self.validate(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _certificate: &CertificateDer<'_>,
        _signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("WiPad only permits TLS 1.3".to_owned()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.verify_tls13(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

impl ClientCertVerifier for PeerCertificateVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        if !intermediates.is_empty() {
            return Err(TlsError::General(
                "WiPad expects a single peer identity certificate".to_owned(),
            ));
        }
        self.validate(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _certificate: &CertificateDer<'_>,
        _signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("WiPad only permits TLS 1.3".to_owned()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.verify_tls13(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

/// Builds the TLS 1.3 client configuration with mandatory client identity.
///
/// None is only for first-contact pairing. The TLS CertificateVerify signature
/// is still checked against the P-256 certificate key. After the user approves
/// the comparison string, supply the stored fingerprint on every reconnect.
pub fn client_config(
    identity: &TlsIdentity,
    expected_server_fingerprint: Option<[u8; 32]>,
) -> Result<ClientConfig, TlsError> {
    let provider = tls_provider();
    ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PeerCertificateVerifier::new(
            expected_server_fingerprint,
        )))
        .with_client_auth_cert(
            vec![CertificateDer::from(identity.certificate_der.clone())],
            identity.private_key_der(),
        )
}

pub fn server_config(
    identity: &TlsIdentity,
    expected_client_fingerprint: Option<[u8; 32]>,
) -> Result<ServerConfig, TlsError> {
    let provider = tls_provider();
    ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&version::TLS13])?
        .with_client_cert_verifier(Arc::new(PeerCertificateVerifier::new(
            expected_client_fingerprint,
        )))
        .with_single_cert(
            vec![CertificateDer::from(identity.certificate_der.clone())],
            identity.private_key_der(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{ClientConnection, ProtocolVersion, ServerConnection};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    fn temporary_directory(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}-{}", prefix, Uuid::new_v4()))
    }

    #[test]
    fn generated_identity_is_a_valid_p256_certificate() {
        let identity = TlsIdentity::generate().unwrap();

        assert!(identity.certificate_der.len() <= MAX_CERTIFICATE_LENGTH);
        assert!(validate_peer_certificate(&identity.certificate_der, None).is_ok());
        assert_eq!(
            identity.fingerprint(),
            certificate_fingerprint(identity.certificate_der())
        );
    }

    #[test]
    fn identity_is_persisted_and_reloaded_without_changing_certificate() {
        let directory = temporary_directory("wipad-tls-identity");
        let path = directory.join("identity.tls");

        let first = TlsIdentity::load_or_create(&path).unwrap();
        let certificate = first.certificate_der().to_vec();
        let fingerprint = first.fingerprint();
        drop(first);

        let second = TlsIdentity::load_or_create(&path).unwrap();
        assert_eq!(second.certificate_der(), certificate);
        assert_eq!(second.fingerprint(), fingerprint);

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
    fn malformed_identity_file_is_rejected_not_replaced() {
        let directory = temporary_directory("wipad-tls-invalid");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("identity.tls");
        fs::write(&path, b"not-a-wipad-tls-identity").unwrap();

        let result = TlsIdentity::load_or_create(&path);
        assert!(matches!(
            result,
            Err(error) if error.kind() == io::ErrorKind::InvalidData
        ));
        assert_eq!(fs::read(&path).unwrap(), b"not-a-wipad-tls-identity");

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn trust_store_requires_explicit_write_and_supports_revocation() {
        let directory = temporary_directory("wipad-tls-trust");
        let store = TrustStore::new(directory.join("trusted-peer.sha256"));
        let identity = TlsIdentity::generate().unwrap();
        let fingerprint = identity.fingerprint();

        assert_eq!(store.load().unwrap(), None);
        assert_eq!(
            store.decision(&fingerprint).unwrap(),
            TrustDecision::NeedsApproval
        );

        store.store(&fingerprint).unwrap();
        assert_eq!(store.load().unwrap(), Some(fingerprint));
        assert_eq!(
            store.decision(&fingerprint).unwrap(),
            TrustDecision::Trusted
        );

        let other = TlsIdentity::generate().unwrap().fingerprint();
        assert_eq!(
            store.decision(&other).unwrap(),
            TrustDecision::IdentityChanged
        );

        store.revoke().unwrap();
        assert_eq!(store.load().unwrap(), None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn comparison_string_is_role_ordered_and_deterministic() {
        let phone = TlsIdentity::generate().unwrap();
        let desktop = TlsIdentity::generate().unwrap();

        let first = pairing_string(phone.certificate_der(), desktop.certificate_der()).unwrap();
        let second = pairing_string(phone.certificate_der(), desktop.certificate_der()).unwrap();
        let reversed = pairing_string(desktop.certificate_der(), phone.certificate_der()).unwrap();

        assert_eq!(first, second);
        assert_ne!(first, reversed);
        assert_eq!(first.len(), 14);
    }

    #[test]
    fn loopback_session_uses_mutual_tls_13_and_pinned_peer_certificates() {
        let phone = TlsIdentity::generate().unwrap();
        let desktop = TlsIdentity::generate().unwrap();

        let phone_pin = desktop.fingerprint();
        let desktop_pin = phone.fingerprint();

        let client = Arc::new(client_config(&phone, Some(phone_pin)).unwrap());
        let server = Arc::new(server_config(&desktop, Some(desktop_pin)).unwrap());

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();

        let server_thread = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();

            let connection = ServerConnection::new(server).unwrap();
            let mut stream = rustls::StreamOwned::new(connection, tcp);

            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).unwrap();
            stream.write_all(b"pong").unwrap();
            stream.flush().unwrap();

            let peer_certificate = stream
                .conn
                .peer_certificates()
                .unwrap()
                .first()
                .unwrap()
                .as_ref()
                .to_vec();
            let protocol = stream.conn.protocol_version();

            (request, peer_certificate, protocol)
        });

        let tcp = TcpStream::connect(address).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();

        let server_name = ServerName::try_from("wipad.local".to_owned()).unwrap();
        let connection = ClientConnection::new(client, server_name).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, tcp);

        stream.write_all(b"ping").unwrap();
        stream.flush().unwrap();

        let mut response = [0_u8; 4];
        stream.read_exact(&mut response).unwrap();

        let server_certificate = stream
            .conn
            .peer_certificates()
            .unwrap()
            .first()
            .unwrap()
            .as_ref()
            .to_vec();
        let client_protocol = stream.conn.protocol_version();

        let (request, client_certificate, server_protocol) = server_thread.join().unwrap();

        assert_eq!(&request, b"ping");
        assert_eq!(&response, b"pong");
        assert_eq!(client_certificate, phone.certificate_der());
        assert_eq!(server_certificate, desktop.certificate_der());
        assert_eq!(client_protocol, Some(ProtocolVersion::TLSv1_3));
        assert_eq!(server_protocol, Some(ProtocolVersion::TLSv1_3));
    }

    #[test]
    fn pinned_verifier_rejects_a_changed_certificate() {
        let trusted = TlsIdentity::generate().unwrap();
        let changed = TlsIdentity::generate().unwrap();
        let verifier = PeerCertificateVerifier::new(Some(trusted.fingerprint()));
        let certificate = CertificateDer::from(changed.certificate_der.clone());

        assert!(verifier.validate(&certificate).is_err());
    }

    #[test]
    fn pairing_gate_requires_both_sides_to_approve() {
        let mut gate = PairingGate::new(false);

        assert_eq!(gate.state(), PairingState::AwaitingBothApprovals);
        assert!(!gate.can_store_peer_pin());
        assert!(!gate.can_start_input());

        gate.approve_local();
        assert_eq!(gate.state(), PairingState::AwaitingPeerApproval);
        assert!(!gate.can_start_input());

        gate.receive_peer_decision(true);
        assert_eq!(gate.state(), PairingState::Ready);
        assert!(gate.can_store_peer_pin());
        assert!(gate.can_start_input());
    }

    #[test]
    fn a_trusted_local_pin_auto_approves_only_the_local_side() {
        let mut gate = PairingGate::new(true);

        assert_eq!(gate.state(), PairingState::AwaitingPeerApproval);
        assert!(!gate.can_start_input());

        gate.receive_peer_decision(true);
        assert_eq!(gate.state(), PairingState::Ready);
        assert!(gate.can_start_input());
    }

    #[test]
    fn peer_rejection_permanently_blocks_the_session() {
        let mut gate = PairingGate::new(true);
        gate.receive_peer_decision(false);
        gate.approve_local();
        gate.receive_peer_decision(true);

        assert_eq!(gate.state(), PairingState::Rejected);
        assert!(!gate.can_store_peer_pin());
        assert!(!gate.can_start_input());
    }

    #[test]
    fn peer_approval_waits_for_local_approval() {
        let mut gate = PairingGate::new(false);
        gate.receive_peer_decision(true);

        assert_eq!(gate.state(), PairingState::AwaitingLocalApproval);
        assert!(!gate.can_start_input());

        gate.approve_local();
        assert_eq!(gate.state(), PairingState::Ready);
    }
}
