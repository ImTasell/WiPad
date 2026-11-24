use mdns_sd::{RecvTimeoutError, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::{IpAddr, TcpListener, UdpSocket};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
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
    pub fn start(on_status: impl Fn(String) + Send + Sync + 'static) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let daemon = ServiceDaemon::new().map_err(|error| error.to_string())?;
        let local_ip = select_local_ipv4().map_err(|error| error.to_string())?;
        let installation_id =
            load_or_create_installation_id().map_err(|error| error.to_string())?;

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

        let listener_stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("wipad-endpoint".into())
            .spawn(move || {
                while !listener_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => drop(stream),
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
        let callback = Arc::new(on_status);
        let browse_stop = Arc::clone(&stop);

        thread::Builder::new()
            .name("wipad-mdns-browser".into())
            .spawn(move || {
                callback("Broadcasting · scanning for phone".to_owned());
                while !browse_stop.load(Ordering::Relaxed) {
                    match receiver.recv_timeout(Duration::from_millis(500)) {
                        Ok(ServiceEvent::ServiceResolved(service)) => {
                            if is_compatible_phone(
                                service.get_property_val_str("pv"),
                                service.get_property_val_str("role"),
                            ) {
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
                        Ok(ServiceEvent::ServiceRemoved(service_type, _))
                            if service_type == PHONE_SERVICE =>
                        {
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

fn is_compatible_phone(protocol_versions: Option<&str>, role: Option<&str>) -> bool {
    role == Some("phone")
        && protocol_versions.is_some_and(|versions| {
            versions
                .split(',')
                .any(|version| version.trim() == PROTOCOL_MAJOR)
        })
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
