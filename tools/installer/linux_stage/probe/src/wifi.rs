//! USB-provisioned, certificate-pinned TLS benchmark. All device state is RAM-only.
use super::{invalid, recovery_hash, request, response, CHUNK};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ServerConfig, ServerConnection, StreamOwned,
};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    net::TcpListener,
    os::unix::fs::OpenOptionsExt,
    sync::Arc,
    thread,
    time::Duration,
};
use subtle::ConstantTimeEq;
static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn active() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::Acquire)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provision {
    ssid_hex: String,
    psk_hex: Option<String>,
    certificate_hex: String,
    private_key_hex: String,
    token_hex: String,
}
fn unhex(value: &str, min: usize, max: usize) -> io::Result<Vec<u8>> {
    if value.len() % 2 != 0
        || value.len() / 2 < min
        || value.len() / 2 > max
        || !value.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err(invalid("invalid bounded hex field"));
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| invalid("invalid hex")))
        .collect()
}
fn configuration(value: &Provision) -> io::Result<String> {
    unhex(&value.ssid_hex, 1, 32)?;
    let security = if let Some(psk) = &value.psk_hex {
        unhex(psk, 32, 32)?;
        format!("    key_mgmt=WPA-PSK\n    proto=RSN\n    psk={psk}\n")
    } else {
        "    key_mgmt=NONE\n".into()
    };
    Ok(format!(
        "ctrl_interface=/tmp/couch-wpa\nupdate_config=0\nnetwork={{\n    ssid={}\n    scan_ssid=1\n{security}}}\n",
        value.ssid_hex
    ))
}
fn private_file(path: &str, data: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)?;
    file.sync_all()
}
fn tls_config(value: &Provision) -> io::Result<ServerConfig> {
    let cert = CertificateDer::from(unhex(&value.certificate_hex, 1, 4096)?);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(unhex(
        &value.private_key_hex,
        1,
        4096,
    )?));
    let provider = rustls::crypto::ring::default_provider();
    ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| invalid("TLS versions unavailable"))?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|_| invalid("TLS certificate/key invalid"))
}
pub fn provision(payload: &[u8]) -> io::Result<()> {
    if payload.len() > 16384 {
        return Err(invalid("provisioning too large"));
    }
    let value: Provision =
        serde_json::from_slice(payload).map_err(|_| invalid("invalid provisioning fields"))?;
    let conf = configuration(&value)?;
    let token = unhex(&value.token_hex, 32, 32)?;
    let config = tls_config(&value)?;
    let listener = TcpListener::bind(("0.0.0.0", 8443))?;
    // Commit only fully validated credentials; config and request are on tmpfs.
    private_file("/tmp/couch-wpa_supplicant.conf.pending", conf.as_bytes())?;
    if active() {
        return Err(invalid("already provisioned"));
    }
    fs::rename(
        "/tmp/couch-wpa_supplicant.conf.pending",
        "/tmp/couch-wpa_supplicant.conf",
    )?;
    // Reuse the credential-free scan supplicant; DHCP starts only after its ACK.
    super::scan::reconfigure().map_err(|_| invalid("supplicant reconfigure failed"))?;
    private_file("/tmp/couch-wifi.request.pending", b"connect\n")?;
    fs::rename("/tmp/couch-wifi.request.pending", "/tmp/couch-wifi.request")?;
    let config = Arc::new(config);
    ACTIVE.store(true, std::sync::atomic::Ordering::Release);
    thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(socket) = socket else { break };
            let timeout = if cfg!(feature = "private-install") {
                900
            } else {
                30
            };
            let _ = socket.set_read_timeout(Some(Duration::from_secs(timeout)));
            let _ = socket.set_write_timeout(Some(Duration::from_secs(30)));
            let Ok(connection) = ServerConnection::new(config.clone()) else {
                break;
            };
            let mut stream = StreamOwned::new(connection, socket);
            let _ = session(&mut stream, &token); // Do not log credentials, tokens or untrusted payloads.
        }
    });
    Ok(())
}
fn session(stream: &mut (impl Read + Write), token: &[u8]) -> io::Result<()> {
    let mut supplied = [0u8; 32];
    stream.read_exact(&mut supplied)?;
    if !bool::from(supplied.as_slice().ct_eq(token)) {
        return Err(invalid("authentication failed"));
    }
    stream.write_all(b"OKAY")?;
    stream.flush()?;
    for _ in 0..16 {
        let mut header = [0u8; 16];
        stream.read_exact(&mut header)?;
        let (op, mut remaining) = request(&header)?;
        match op {
            0 => {
                response(stream, 4)?;
                stream.write_all(b"CBP1")?;
            }
            1 | 2 => {
                if op == 1 {
                    response(stream, remaining)?;
                }
                let mut buffer = [0xa5; CHUNK];
                while remaining > 0 {
                    let count = remaining.min(CHUNK as u64) as usize;
                    if op == 1 {
                        stream.write_all(&buffer[..count])?;
                    } else {
                        stream.read_exact(&mut buffer[..count])?;
                        if buffer[..count].iter().any(|v| *v != 0xa5) {
                            return Err(invalid("RAM pattern mismatch"));
                        }
                    }
                    remaining -= count as u64;
                }
                if op == 2 {
                    response(stream, 0)?;
                }
            }
            3 => {
                let data = recovery_hash()?;
                response(stream, data.len() as u64)?;
                stream.write_all(&data)?;
            }
            #[cfg(feature = "private-install")]
            10 => {
                let result = super::install::session(stream);
                if let Err(error) = &result {
                    eprintln!("Installer stopped: {error}");
                    let _ = super::install::report_error(stream);
                }
                return result;
            }
            _ => return Err(invalid("USB-only operation")),
        }
        stream.flush()?;
    }
    Ok(())
}
fn status_label(status: &str) -> &str {
    match status.trim() {
        "initializing" | "ready" | "connecting" | "connected" | "failed" => status.trim(),
        _ => "waiting",
    }
}
fn error_label(error: &str) -> &str {
    match error.trim() {
        "detect-node"
        | "loader-exit"
        | "transport-node"
        | "wifi-node"
        | "launcher-exit"
        | "transport-timeout"
        | "power-on"
        | "interface-timeout"
        | "interface-up"
        | "dhcp-exit"
        | "supplicant-exit"
        | "supplicant-socket-timeout" => error.trim(),
        _ => "unknown",
    }
}
pub fn status() -> Vec<u8> {
    status_from(std::path::Path::new("/tmp"))
}
fn status_from(root: &std::path::Path) -> Vec<u8> {
    let ip = fs::read_to_string(root.join("couch-wifi.ip")).unwrap_or_default();
    let status =
        fs::read_to_string(root.join("couch-wifi.status")).unwrap_or_else(|_| "waiting".into());
    let ip = ip
        .trim()
        .parse::<std::net::Ipv4Addr>()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let status = status_label(&status);
    let error = fs::read_to_string(root.join("couch-wifi.error")).unwrap_or_default();
    let error = if status == "failed" {
        error_label(&error)
    } else {
        "none"
    };
    serde_json::json!({"ip":ip,"status":status,"port":8443,"error":error,"provisioned":active(),"scan":true,"stage_network_config":cfg!(feature = "private-install")})
        .to_string()
        .into_bytes()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn value() -> Provision {
        Provision {
            ssid_hex: "636f756368".into(),
            psk_hex: Some("ab".repeat(32)),
            certificate_hex: String::new(),
            private_key_hex: String::new(),
            token_hex: String::new(),
        }
    }
    #[test]
    fn dhcp_connected_status_is_preserved_and_unknown_status_is_bounded() {
        assert_eq!(status_label("connected\n"), "connected");
        assert_eq!(status_label("failed"), "failed");
        assert_eq!(status_label("untrusted status"), "waiting");
        assert_eq!(error_label("loader-exit\n"), "loader-exit");
        assert_eq!(error_label("private network text"), "unknown");
    }
    #[test]
    fn every_reason_the_stage_can_write_survives_the_status_wire() {
        // The stage and the host each carry this list. A reason missing here is
        // silently flattened to "unknown", which is how a distinct failure can
        // reach the operator looking exactly like the one it was split from.
        // Read the reasons the stage actually writes, then drive them through
        // error_label rather than asserting on the source text.
        let stage = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../wifi-stage/wifi-init"
        ))
        .expect("stage script");
        let mut written: Vec<&str> = stage
            .split("fail ")
            .skip(1)
            .filter_map(|rest| {
                rest.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                    .next()
            })
            .filter(|token| !token.is_empty() && *token != "reason")
            .collect();
        written.sort_unstable();
        written.dedup();
        assert!(
            written.contains(&"supplicant-socket-timeout") && written.contains(&"supplicant-exit"),
            "stage no longer writes the reasons under test: {written:?}"
        );
        for reason in written {
            assert_eq!(
                error_label(reason),
                reason,
                "{reason} is flattened to unknown on the status wire"
            );
        }
    }
    #[test]
    fn a_stalled_supplicant_reaches_status_without_being_flattened() {
        // End to end through the real status(): stage files in, JSON out.
        let root = std::env::temp_dir().join(format!("couch-probe-status-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("couch-wifi.status"), "failed\n").unwrap();
        fs::write(root.join("couch-wifi.error"), "supplicant-socket-timeout\n").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&status_from(&root)).unwrap();
        assert_eq!(parsed["status"], "failed");
        assert_eq!(parsed["error"], "supplicant-socket-timeout");
        assert_eq!(parsed["ip"], "");
        // An unrecognised reason must still be bounded to unknown.
        fs::write(root.join("couch-wifi.error"), "private network text\n").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&status_from(&root)).unwrap();
        assert_eq!(parsed["error"], "unknown");
        // A reason is only surfaced while the status is failed.
        fs::write(root.join("couch-wifi.status"), "connected\n").unwrap();
        fs::write(root.join("couch-wifi.error"), "supplicant-socket-timeout\n").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&status_from(&root)).unwrap();
        assert_eq!(parsed["error"], "none");
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn credentials_are_hex_bounded_and_injection_is_rejected() {
        let mut p = value();
        assert!(configuration(&p).unwrap().contains("ssid=636f756368"));
        assert!(configuration(&p).unwrap().contains("scan_ssid=1\n"));
        for s in ["", "00\n}", &"ab".repeat(33)] {
            p.ssid_hex = s.into();
            assert!(configuration(&p).is_err());
        }
        p = value();
        p.psk_hex = Some("a".repeat(63));
        assert!(configuration(&p).is_err());
    }
    #[test]
    fn wrong_token_cannot_reach_commands() {
        let mut stream = io::Cursor::new(vec![0u8; 48]);
        assert!(session(&mut stream, &[1u8; 32]).is_err());
        assert_eq!(stream.position(), 32);
    }
    #[test]
    fn tls_loopback_requires_pinned_certificate_and_session_token() {
        use std::process::Command;
        let root = std::env::temp_dir().join(format!("couch-tls-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let openssl = |args: &[&str]| {
            assert!(Command::new("openssl")
                .current_dir(&root)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success());
        };
        openssl(&[
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
            "-keyout",
            "key.pem",
            "-out",
            "cert.pem",
            "-days",
            "1",
            "-subj",
            "/CN=couch-probe",
            "-addext",
            "subjectAltName=DNS:couch-probe",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
        ]);
        openssl(&[
            "x509", "-in", "cert.pem", "-outform", "DER", "-out", "cert.der",
        ]);
        openssl(&[
            "pkcs8", "-topk8", "-nocrypt", "-in", "key.pem", "-outform", "DER", "-out", "key.der",
        ]);
        let cert = fs::read(root.join("cert.der")).unwrap();
        let hex = |bytes: Vec<u8>| bytes.iter().map(|n| format!("{n:02x}")).collect::<String>();
        let mut value = value();
        value.certificate_hex = hex(cert.clone());
        value.private_key_hex = hex(fs::read(root.join("key.der")).unwrap());
        let config = Arc::new(tls_config(&value).unwrap());
        for (trust, correct_token) in [(true, true), (false, true), (true, false)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let config = config.clone();
            let server = thread::spawn(move || {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut stream = StreamOwned::new(ServerConnection::new(config).unwrap(), socket);
                let _ = session(&mut stream, &[7u8; 32]);
            });
            let mut roots = rustls::RootCertStore::empty();
            if trust {
                roots.add(CertificateDer::from(cert.clone())).unwrap();
            }
            let client = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            let connection =
                rustls::ClientConnection::new(Arc::new(client), "couch-probe".try_into().unwrap())
                    .unwrap();
            let socket = std::net::TcpStream::connect(addr).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            let sent = stream.write_all(&[if correct_token { 7 } else { 0 }; 32]);
            let mut ack = [0u8; 4];
            let accepted = sent.is_ok() && stream.read_exact(&mut ack).is_ok();
            assert_eq!(accepted, trust && correct_token);
            if accepted {
                assert_eq!(&ack, b"OKAY");
                let mut command = [0u8; 16];
                command[..4].copy_from_slice(b"CBP1");
                stream.write_all(&command).unwrap();
                let mut reply = [0u8; 20];
                stream.read_exact(&mut reply).unwrap();
                assert_eq!(&reply[16..], b"CBP1");
            }
            drop(stream);
            server.join().unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }
}
