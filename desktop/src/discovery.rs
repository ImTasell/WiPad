use mdns_sd::{RecvTimeoutError, ServiceDaemon, ServiceEvent, ServiceInfo};
use rustls::{ServerConnection, StreamOwned};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};

use crate::controller::InputSequenceTracker;
use crate::pairing::{PairingRequest, perform_pairing};
use crate::protocol::{
    ApplicationMessage, FrameReader, PeerRole, decode_application_message,
    write_application_message,
};
use crate::security::{
    TlsIdentity, TrustDecision, TrustStore, certificate_fingerprint, server_config,
};
use crate::virtual_gamepad::VirtualGamepad;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DESKTOP_SERVICE: &str = "_wipad-desktop._tcp.local.";
const PHONE_SERVICE: &str = "_wipad-phone._tcp.local.";
const PROTOCOL_MAJOR: &str = "1";
const SESSION_PORT: u16 = 24872;

pub struct Discovery {
    daemon: ServiceDaemon,
    stop: Arc<AtomicBool>,
}

impl Discovery {
    pub fn start(
        on_status: impl Fn(String) + Send + Sync + 'static,
        approve_peer: impl Fn(&PairingRequest) -> bool + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let daemon = ServiceDaemon::new().map_err(|error| error.to_string())?;
        let local_ip = select_local_ipv4().map_err(|error| error.to_string())?;
        let installation_id =
            load_or_create_installation_id().map_err(|error| error.to_string())?;
        let identity_path = TlsIdentity::default_path().map_err(|error| error.to_string())?;
        let identity = Arc::new(
            TlsIdentity::load_or_create(identity_path).map_err(|error| error.to_string())?,
        );
        let trust_store_path = TrustStore::default_path().map_err(|error| error.to_string())?;
        let trust_store = Arc::new(TrustStore::new(trust_store_path));

        let properties = [
            ("pv", PROTOCOL_MAJOR),
            ("id", installation_id.as_str()),
            ("role", "desktop"),
            ("name", "WiPad Desktop"),
        ];
        let instance_name = format!("WiPadDesktop-{}", &installation_id[..8]);
        let service = ServiceInfo::new(
            DESKTOP_SERVICE,
            instance_name.as_str(),
            "wipad-desktop.local.",
            local_ip.to_string(),
            SESSION_PORT,
            &properties[..],
        )
        .map_err(|error| error.to_string())?;
        daemon
            .register(service)
            .map_err(|error| error.to_string())?;

        let listener = TcpListener::bind(("0.0.0.0", SESSION_PORT))
            .map_err(|error| format!("could not reserve discovery endpoint: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;

        let endpoint_stop = Arc::clone(&stop);
        let endpoint_status = Arc::new(on_status);
        let approval_callback = Arc::new(approve_peer);
        let endpoint_identity = Arc::clone(&identity);
        let endpoint_trust_store = Arc::clone(&trust_store);
        let worker_status = Arc::clone(&endpoint_status);

        thread::Builder::new()
            .name("wipad-tls-endpoint".into())
            .spawn(move || {
                while !endpoint_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, remote_address)) => {
                            let result = serve_phone(
                                stream,
                                &endpoint_identity,
                                &endpoint_trust_store,
                                &*worker_status,
                                &*approval_callback,
                                &endpoint_stop,
                            );
                            if let Err(error) = result {
                                worker_status(format!(
                                    "Phone session ended · {remote_address} · {error}"
                                ));
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(40));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(100)),
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        let receiver = daemon
            .browse(PHONE_SERVICE)
            .map_err(|error| error.to_string())?;
        let callback = Arc::clone(&endpoint_status);
        let browse_stop = Arc::clone(&stop);

        thread::Builder::new()
            .name("wipad-mdns-browser".into())
            .spawn(move || {
                let mut discovered_phones = HashSet::new();
                callback("Broadcasting · scanning for phone".to_owned());
                while !browse_stop.load(Ordering::Relaxed) {
                    match receiver.recv_timeout(Duration::from_millis(500)) {
                        Ok(ServiceEvent::ServiceResolved(service)) => {
                            if !is_compatible_phone(
                                service.get_property_val_str("pv"),
                                service.get_property_val_str("role"),
                            ) {
                                continue;
                            }
                            if !discovered_phones.insert(service.get_fullname().to_owned()) {
                                continue;
                            }
                            {
                                let name = service
                                    .get_property_val_str("name")
                                    .unwrap_or("WiPad Phone");
                                let address = service
                                    .get_addresses_v4()
                                    .iter()
                                    .next()
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| "address unavailable".to_owned());
                                callback(format!("Phone found · {name} · {address}"));
                            }
                        }
                        Ok(ServiceEvent::ServiceRemoved(service_type, full_name))
                            if service_type == PHONE_SERVICE =>
                        {
                            discovered_phones.remove(&full_name);
                            callback("Broadcasting · scanning for phone".to_owned());
                        }
                        Ok(ServiceEvent::SearchStopped(_)) => {
                            callback("Discovery stopped".to_owned());
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {
                            callback("Discovery unavailable · browser disconnected".to_owned());
                            break;
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        Ok(Self { daemon, stop })
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.daemon.shutdown();
    }
}

fn serve_phone<FStatus, FApprove>(
    socket: TcpStream,
    identity: &TlsIdentity,
    trust_store: &TrustStore,
    on_status: &FStatus,
    approve_peer: &FApprove,
    stop: &AtomicBool,
) -> io::Result<()>
where
    FStatus: Fn(String),
    FApprove: Fn(&PairingRequest) -> bool,
{
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(120)))?;
    socket.set_write_timeout(Some(Duration::from_secs(10)))?;

    // Reload the pin for every connection. A first pairing writes the pin only
    // after both users approve, so the next TLS handshake is pinned automatically.
    let expected_phone_fingerprint = trust_store.load()?;
    let config = Arc::new(
        server_config(identity, expected_phone_fingerprint)
            .map_err(|error| io::Error::other(error.to_string()))?,
    );
    let connection =
        ServerConnection::new(config).map_err(|error| io::Error::other(error.to_string()))?;
    let mut tls = StreamOwned::new(connection, socket);

    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock)?;
    }

    let peer_certificate = tls
        .conn
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::PermissionDenied, "phone identity is missing")
        })?
        .as_ref()
        .to_vec();
    let peer_fingerprint = certificate_fingerprint(&peer_certificate);

    match trust_store.decision(&peer_fingerprint)? {
        TrustDecision::Trusted => {
            on_status("Trusted phone authenticated · checking approval".to_owned())
        }
        TrustDecision::NeedsApproval => {
            on_status("New phone authenticated · pairing approval required".to_owned())
        }
        TrustDecision::IdentityChanged => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "phone identity changed; revoke the old trust pin before pairing again",
            ));
        }
    }

    let pairing = perform_pairing(
        &mut tls,
        PeerRole::Desktop,
        "WiPad Desktop",
        identity.certificate_der(),
        &peer_certificate,
        trust_store,
        |request| {
            on_status(format!(
                "Pairing approval required · {} · {}",
                request.peer_display_name, request.comparison_string
            ));
            approve_peer(request)
        },
    )?;

    on_status(format!(
        "Secure session ready · {} · {}",
        pairing.peer_display_name, pairing.comparison_string
    ));

    // Only create the virtual device after both peers have completed pairing.
    tls.sock
        .set_read_timeout(Some(Duration::from_millis(250)))?;
    receive_controller_stream(&mut tls, stop, &on_status)?;
    on_status("Phone disconnected · scanning for phone".to_owned());
    Ok(())
}

fn receive_controller_stream<S: Read + Write>(
    stream: &mut S,
    stop: &AtomicBool,
    on_status: &impl Fn(String),
) -> io::Result<()> {
    let mut decoder = FrameReader::new();
    let mut sequence = InputSequenceTracker::default();
    let mut gamepad = VirtualGamepad::create()?;
    gamepad.release_all()?;

    let mut last_input = Instant::now();
    let mut controls_released = true;
    let mut controller_connected: Option<bool> = None;
    let mut heartbeat_nonce = 1_u64;
    let mut awaiting_pong: Option<(u64, Instant)> = None;
    let mut last_ping = Instant::now() - Duration::from_secs(2);

    loop {
        if stop.load(Ordering::Relaxed) {
            gamepad.release_all()?;
            return Ok(());
        }

        if let Some((_, sent_at)) = awaiting_pong {
            if sent_at.elapsed() >= Duration::from_secs(7) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "phone heartbeat timed out",
                ));
            }
        } else if last_ping.elapsed() >= Duration::from_secs(2) {
            write_application_message(
                stream,
                &ApplicationMessage::Ping {
                    nonce: heartbeat_nonce,
                },
            )?;
            stream.flush()?;
            awaiting_pong = Some((heartbeat_nonce, Instant::now()));
            heartbeat_nonce = heartbeat_nonce.wrapping_add(1).max(1);
            last_ping = Instant::now();
        }

        match decoder.read_next(stream) {
            Ok(Some(frame)) => match decode_application_message(&frame)? {
                ApplicationMessage::InputState(state) => {
                    if sequence.accept(&state)? {
                        last_input = Instant::now();
                        if controller_connected != Some(false) {
                            gamepad.apply_state(&state)?;
                            controls_released = false;
                        }
                    }
                }
                ApplicationMessage::Pong { nonce } => {
                    match awaiting_pong {
                        Some((expected, _)) if expected == nonce => awaiting_pong = None,
                        _ => return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "received an unexpected WiPad heartbeat response",
                        )),
                    }
                }
                ApplicationMessage::ControllerStatus { connected } => {
                    if controller_connected != Some(connected) {
                        controller_connected = Some(connected);
                        if !connected {
                            gamepad.release_all()?;
                            controls_released = true;
                            on_status(
                                "Phone connected · waiting for Bluetooth controller".to_owned(),
                            );
                        } else {
                            on_status(
                                "Bluetooth controller connected · receiving input".to_owned(),
                            );
                        }
                    }
                    last_input = Instant::now();
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "received a message that is invalid during controller streaming",
                    ));
                }
            },
            Ok(None) => {
                gamepad.release_all()?;
                return Ok(());
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                // A silent or stalled phone must never leave a button held forever.
                let input_idle = last_input.elapsed();
                if !controls_released && input_idle >= Duration::from_secs(2) {
                    gamepad.release_all()?;
                    controls_released = true;
                }

                // A trusted peer that stops sending snapshots is no longer a
                // usable session. Close it so the phone's reconnect loop can
                // establish a fresh authenticated connection.
                if input_idle >= Duration::from_secs(15) {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "controller stream was idle for 15 seconds",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_compatible_phone(protocol_versions: Option<&str>, role: Option<&str>) -> bool {
    role == Some("phone")
        && protocol_versions.is_some_and(|versions| {
            versions
                .split(',')
                .any(|version| version.trim() == PROTOCOL_MAJOR)
        })
}

fn select_local_ipv4() -> io::Result<std::net::Ipv4Addr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.connect(("224.0.0.251", 5353))?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(address) if !address.is_unspecified() => Ok(address),
        _ => Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no active IPv4 network interface",
        )),
    }
}

fn load_or_create_installation_id() -> io::Result<String> {
    let config_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory"))?;
    let directory = config_root.join("wipad");
    fs::create_dir_all(&directory)?;

    let path = directory.join("installation-id");
    match fs::read_to_string(&path) {
        Ok(id) if Uuid::parse_str(id.trim()).is_ok() => return Ok(id.trim().to_owned()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let id = Uuid::new_v4().to_string();
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(id.as_bytes())?;
            Ok(id)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = fs::read_to_string(path)?;
            Uuid::parse_str(existing.trim())
                .map(|_| existing.trim().to_owned())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid installation ID"))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::is_compatible_phone;

    #[test]
    fn accepts_a_phone_with_a_supported_major_version() {
        assert!(is_compatible_phone(Some("1"), Some("phone")));
    }

    #[test]
    fn accepts_a_supported_version_in_a_version_list() {
        assert!(is_compatible_phone(Some("2, 1, 3"), Some("phone")));
    }

    #[test]
    fn rejects_an_unsupported_version() {
        assert!(!is_compatible_phone(Some("2"), Some("phone")));
    }

    #[test]
    fn rejects_a_different_role() {
        assert!(!is_compatible_phone(Some("1"), Some("desktop")));
    }

    #[test]
    fn rejects_missing_metadata() {
        assert!(!is_compatible_phone(None, Some("phone")));
        assert!(!is_compatible_phone(Some("1"), None));
    }
}
