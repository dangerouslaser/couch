//! The TV's plain-HTTP information endpoint on port 8001 (`/api/v2/`).
//!
//! It answers without pairing and is the only source of the TV's MAC, model,
//! Frame TV flag and (on 2019+ firmware) its `PowerState`. Just enough
//! HTTP/1.1 for one GET with `Connection: close`; there is no TLS to justify
//! a client stack on the ARM build.
use crate::{Error, Result};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream},
    time::Duration,
};
pub const PORT: u16 = 8001;
const MAX_BODY: usize = 256 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub model: String,
    /// `on` / `standby` on firmware that reports it; older TVs omit it, so an
    /// absent value is not a power state.
    pub power_state: Option<String>,
    pub mac: Option<String>,
    pub token_auth: bool,
    pub frame_tv: bool,
}
pub fn device_info(address: IpAddr, timeout: Duration) -> Result<DeviceInfo> {
    let body = get(address, PORT, "/api/v2/", timeout)?;
    let value: Value = serde_json::from_slice(&body).map_err(|_| Error::Protocol)?;
    parse_device_info(&value)
}
/// The `device` object carries string-typed booleans (`"true"`), which is why
/// nothing here is deserialised with serde directly.
pub fn parse_device_info(value: &Value) -> Result<DeviceInfo> {
    let device = value.get("device").ok_or(Error::Protocol)?;
    let text = |key: &str| {
        device[key]
            .as_str()
            .map(|s| {
                s.chars()
                    .filter(|c| !c.is_control())
                    .take(128)
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
    };
    let flag = |key: &str| device[key].as_str() == Some("true") || device[key] == true;
    let mac = text("wifiMac")
        .map(|m| m.to_ascii_uppercase())
        .filter(|m| crate::magic_packet(m).is_ok());
    Ok(DeviceInfo {
        name: text("name")
            .or_else(|| value["name"].as_str().map(str::to_string))
            .unwrap_or_default(),
        model: text("modelName")
            .or_else(|| text("model"))
            .unwrap_or_default(),
        power_state: text("PowerState"),
        mac,
        token_auth: flag("TokenAuthSupport"),
        frame_tv: flag("FrameTVSupport"),
    })
}
pub(crate) fn get(address: IpAddr, port: u16, path: &str, timeout: Duration) -> Result<Vec<u8>> {
    if address.is_unspecified() || address.is_multicast() || path.is_empty() {
        return Err(Error::Configuration);
    }
    let mut stream = TcpStream::connect_timeout(&SocketAddr::new(address, port), timeout)
        .map_err(|_| Error::Transport)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| Error::Transport)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|_| Error::Transport)?;
    let host = match address {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .map_err(|_| Error::Transport)?;
    let mut raw = Vec::new();
    stream
        .take(MAX_BODY as u64 + 8192)
        .read_to_end(&mut raw)
        .map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) {
                Error::Timeout
            } else {
                Error::Transport
            }
        })?;
    let (status, body) = parse_response(&raw)?;
    if status != 200 {
        return Err(Error::Rejected);
    }
    Ok(body)
}
pub(crate) fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>)> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(Error::Protocol)?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| Error::Protocol)?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.strip_prefix("HTTP/1."))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or(Error::Protocol)?;
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            length = Some(value.parse::<usize>().map_err(|_| Error::Protocol)?);
        } else if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
    }
    let rest = &raw[split + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else if let Some(length) = length {
        rest.get(..length).ok_or(Error::Protocol)?.to_vec()
    } else {
        rest.to_vec()
    };
    if body.len() > MAX_BODY {
        return Err(Error::Protocol);
    }
    Ok((status, body))
}
fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or(Error::Protocol)?;
        let size_text = std::str::from_utf8(&rest[..line_end]).map_err(|_| Error::Protocol)?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| Error::Protocol)?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(body);
        }
        body.extend_from_slice(rest.get(..size).ok_or(Error::Protocol)?);
        if body.len() > MAX_BODY {
            return Err(Error::Protocol);
        }
        rest = rest.get(size + 2..).ok_or(Error::Protocol)?;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn device_info_reads_string_booleans_and_normalises_mac() {
        let value = json!({"device":{"FrameTVSupport":"false","TokenAuthSupport":"true","PowerState":"on","modelName":"QE55Q80TATXXU","model":"20_MUSEM_QTV","name":"[TV] Lounge","wifiMac":"8c:c8:cd:11:22:33","OS":"Tizen"},"name":"[TV] Lounge"});
        let info = parse_device_info(&value).unwrap();
        assert_eq!(info.model, "QE55Q80TATXXU");
        assert_eq!(info.name, "[TV] Lounge");
        assert_eq!(info.power_state.as_deref(), Some("on"));
        assert_eq!(info.mac.as_deref(), Some("8C:C8:CD:11:22:33"));
        assert!(info.token_auth);
        assert!(!info.frame_tv);
        let older =
            json!({"device":{"model":"16_KANTM_UHD","name":"[TV] Kitchen","wifiMac":"none"}});
        let info = parse_device_info(&older).unwrap();
        assert_eq!(info.model, "16_KANTM_UHD");
        assert_eq!(info.power_state, None);
        assert_eq!(info.mac, None);
        assert!(!info.token_auth);
        assert!(parse_device_info(&json!({"unexpected":1})).is_err());
    }
    #[test]
    fn responses_are_parsed_with_length_chunking_and_status() {
        let (status, body) = parse_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
        )
        .unwrap();
        assert_eq!((status, body.as_slice()), (200, b"{}".as_slice()));
        let (_, body) = parse_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n2\r\n\"}\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(body, b"{\"a\"}");
        assert_eq!(
            parse_response(b"HTTP/1.1 404 Not Found\r\n\r\n").unwrap().0,
            404
        );
        assert!(parse_response(b"garbage").is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}").is_err());
    }
    #[test]
    fn device_info_over_loopback_http() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 512];
            let n = stream.read(&mut request).unwrap();
            let text = String::from_utf8_lossy(&request[..n]).to_string();
            assert!(text.starts_with("GET /api/v2/ HTTP/1.1\r\n"));
            let body = json!({"device":{"modelName":"UE43","name":"[TV] Test","wifiMac":"aa:bb:cc:dd:ee:ff","TokenAuthSupport":"true"}}).to_string();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
        });
        let body = get(
            "127.0.0.1".parse().unwrap(),
            port,
            "/api/v2/",
            Duration::from_secs(2),
        )
        .unwrap();
        let info = parse_device_info(&serde_json::from_slice(&body).unwrap()).unwrap();
        assert_eq!(info.mac.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        server.join().unwrap();
    }
}
