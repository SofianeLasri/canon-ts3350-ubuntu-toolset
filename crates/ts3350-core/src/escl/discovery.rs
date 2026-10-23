//! eSCL scanner discovery over mDNS (`_uscan._tcp`).

use std::net::{IpAddr, TcpStream};
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::i18n::i18n;

const SERVICE: &str = "_uscan._tcp.local.";
/// Local port exposed by `ipp-usb` for USB-connected printers.
const IPP_USB: &str = "http://localhost:60000/eSCL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredScanner {
    /// Human-readable name ("Canon TS3300 series").
    pub name: String,
    /// Base URL of the eSCL service.
    pub url: String,
}

/// How long to wait for other answers after the first scanner is found.
const GRACE: Duration = Duration::from_millis(1200);

/// Browses the network (for at most `timeout`) and returns the scanners found.
///
/// The printer answers within 0.2 s to several seconds: stop shortly after the
/// first answer rather than always waiting for `timeout`.
pub fn discover(timeout: Duration) -> Vec<DiscoveredScanner> {
    let mut found = Vec::new();
    if let Ok(daemon) = ServiceDaemon::new() {
        if let Ok(receiver) = daemon.browse(SERVICE) {
            let mut deadline = Instant::now() + timeout;
            while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                let Ok(event) = receiver.recv_timeout(remaining) else { break };
                if let ServiceEvent::ServiceResolved(service) = event {
                    let Some(ip) = service.get_addresses_v4().into_iter().next() else { continue };
                    let root = service.get_property_val_str("rs").unwrap_or("eSCL").trim_matches('/');
                    let name = service
                        .get_property_val_str("ty")
                        .map(str::to_owned)
                        .unwrap_or_else(|| instance_name(&service.fullname));
                    let url = format!("http://{}:{}/{}", IpAddr::V4(ip), service.port, root);
                    let scanner = DiscoveredScanner { name, url };
                    if !found.contains(&scanner) {
                        if found.is_empty() {
                            deadline = deadline.min(Instant::now() + GRACE);
                        }
                        found.push(scanner);
                    }
                }
            }
            let _ = daemon.stop_browse(SERVICE);
        }
        let _ = daemon.shutdown();
    }

    if found.is_empty()
        && TcpStream::connect_timeout(&"127.0.0.1:60000".parse().unwrap(), Duration::from_millis(300)).is_ok()
    {
        found.push(DiscoveredScanner { name: i18n("USB scanner (ipp-usb)").into(), url: IPP_USB.into() });
    }
    found
}

fn instance_name(fullname: &str) -> String {
    fullname.split("._").next().unwrap_or(fullname).replace("\\032", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_strips_service_type() {
        assert_eq!(instance_name("Canon TS3300 series._uscan._tcp.local."), "Canon TS3300 series");
    }
}
