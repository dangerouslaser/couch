//! SSDP search for Samsung TVs. The TV's remote-control receiver answers
//! `urn:samsung.com:device:RemoteControlReceiver:1`, the search target Home
//! Assistant relies on; the answer's LOCATION header carries the TV's address.
//! Discovery is a hint only: a found address is confirmed through the REST
//! endpoint before it is shown, and manual entry remains available.
use crate::{Error, Result};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    time::{Duration, Instant},
};
pub const SEARCH_TARGET: &str = "urn:samsung.com:device:RemoteControlReceiver:1";
const GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
pub fn search(wait: Duration) -> Result<Vec<IpAddr>> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|_| Error::Transport)?;
    socket
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|_| Error::Transport)?;
    let request = format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {SEARCH_TARGET}\r\n\r\n"
    );
    // Two probes: multicast replies are unreliable on Wi-Fi and the TV only
    // answers once per request it hears.
    for _ in 0..2 {
        socket
            .send_to(request.as_bytes(), GROUP)
            .map_err(|_| Error::Transport)?;
    }
    let deadline = Instant::now() + wait;
    let mut found = BTreeSet::new();
    let mut buffer = [0u8; 2048];
    while Instant::now() < deadline && found.len() < 32 {
        match socket.recv_from(&mut buffer) {
            Ok((n, from)) => {
                if let Some(address) = parse(&buffer[..n], from) {
                    found.insert(address);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Err(Error::Transport),
        }
    }
    Ok(found.into_iter().collect())
}
/// Only unicast replies to our search count; a NOTIFY from anything else on
/// the LAN is ignored. The LOCATION host wins over the sender when both are
/// present and agree in family, since some firmware answers from a secondary
/// interface.
pub(crate) fn parse(reply: &[u8], from: SocketAddr) -> Option<IpAddr> {
    let text = std::str::from_utf8(reply).ok()?;
    let mut lines = text.split("\r\n");
    if !lines.next()?.starts_with("HTTP/1.1 200") {
        return None;
    }
    let mut matched = false;
    let mut location = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "st" | "usn" if value.contains(SEARCH_TARGET) => matched = true,
            "location" => location = url::Url::parse(value).ok(),
            _ => {}
        }
    }
    if !matched {
        return None;
    }
    let host = location
        .as_ref()
        .and_then(|u| u.host_str().map(|h| h.trim_matches(['[', ']']).to_string()))
        .and_then(|h| h.parse::<IpAddr>().ok())
        .unwrap_or(from.ip());
    (!host.is_unspecified() && !host.is_multicast() && !host.is_loopback()).then_some(host)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssdp_replies_are_filtered_to_samsung_remote_receivers() {
        let from: SocketAddr = "192.168.1.50:1900".parse().unwrap();
        let reply = b"HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nLOCATION: http://192.168.1.50:7676/smp_2_\r\nST: urn:samsung.com:device:RemoteControlReceiver:1\r\nUSN: uuid:abc::urn:samsung.com:device:RemoteControlReceiver:1\r\n\r\n";
        assert_eq!(parse(reply, from), Some("192.168.1.50".parse().unwrap()));
        let other = b"HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.9:1400/xml/device_description.xml\r\nST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n\r\n";
        assert_eq!(parse(other, from), None);
        let notify =
            b"NOTIFY * HTTP/1.1\r\nST: urn:samsung.com:device:RemoteControlReceiver:1\r\n\r\n";
        assert_eq!(parse(notify, from), None);
        let no_location =
            b"HTTP/1.1 200 OK\r\nST: urn:samsung.com:device:RemoteControlReceiver:1\r\n\r\n";
        assert_eq!(parse(no_location, from), Some(from.ip()));
        let bad_location = b"HTTP/1.1 200 OK\r\nLOCATION: http://0.0.0.0:7676/\r\nST: urn:samsung.com:device:RemoteControlReceiver:1\r\n\r\n";
        assert_eq!(parse(bad_location, from), None);
    }
}
