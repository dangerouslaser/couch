//! TLS 1.3 connection pinned to the certificate provisioned over selected USB.
use anyhow::{ensure, Context, Result};
use rustls::{
    pki_types::CertificateDer, ClientConfig, ClientConnection, RootCertStore, StreamOwned,
};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};

pub struct DeadlineSocket {
    socket: TcpStream,
    // An *idle* timeout, not a whole-transaction budget: reads and writes may run
    // for as long as data keeps flowing. A multi-gigabyte image over slow Wi-Fi
    // takes far longer than any fixed transaction deadline, so bounding the whole
    // transaction by one clock aborted long transfers mid-stream ("stage phase
    // deadline exceeded"). We instead fail only when the connection stalls with no
    // progress for `idle`, which distinguishes a slow link from a dead one.
    idle: Duration,
    last: Instant,
}
impl DeadlineSocket {
    fn remaining(&self) -> io::Result<Duration> {
        self.idle
            .checked_sub(self.last.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "stage connection stalled"))
    }
    fn progressed(&mut self) {
        self.last = Instant::now();
    }
    pub fn set_phase_timeout(&mut self, timeout: Duration) -> Result<()> {
        ensure!(
            !timeout.is_zero() && timeout <= Duration::from_secs(1800),
            "invalid stage idle timeout"
        );
        self.idle = timeout;
        self.last = Instant::now();
        Ok(())
    }
}
impl Read for DeadlineSocket {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.socket.set_read_timeout(Some(self.remaining()?))?;
        let n = self.socket.read(bytes)?;
        // Any received byte is progress; extend the idle window from now.
        if n > 0 {
            self.progressed();
        }
        Ok(n)
    }
}
impl Write for DeadlineSocket {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.socket.set_write_timeout(Some(self.remaining()?))?;
        let n = self.socket.write(bytes)?;
        if n > 0 {
            self.progressed();
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.remaining()?;
        self.socket.flush()
    }
}
pub type Connection = StreamOwned<ClientConnection, DeadlineSocket>;

/// Explicit connection only: caller obtains address, certificate and token from
/// its physically selected USB provisioning exchange. There is no discovery,
/// operating-system trust store, certificate fallback or automatic retry.
pub fn connect(
    address: SocketAddr,
    certificate_der: &[u8],
    token: &[u8; 32],
) -> Result<Connection> {
    ensure!(
        !certificate_der.is_empty() && certificate_der.len() <= 8192,
        "invalid stage certificate size"
    );
    ensure!(
        !address.ip().is_unspecified() && !address.ip().is_multicast() && address.port() != 0,
        "invalid stage address"
    );
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(certificate_der.to_vec()))
        .context("invalid stage certificate")?;
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_root_certificates(roots)
            .with_no_client_auth();
    let connection = ClientConnection::new(Arc::new(config), "couch-probe".try_into()?)?;
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(15))
        .context("stage connection failed")?;
    socket.set_nodelay(true)?;
    let mut stream = StreamOwned::new(
        connection,
        DeadlineSocket {
            socket,
            idle: Duration::from_secs(15),
            last: Instant::now(),
        },
    );
    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .context("stage TLS verification failed")?;
    }
    ensure!(
        stream
            .conn
            .peer_certificates()
            .and_then(|chain| chain.first())
            .map(|cert| cert.as_ref())
            == Some(certificate_der),
        "stage certificate differs from USB provisioned certificate"
    );
    stream.write_all(token)?;
    stream.flush()?;
    let mut ack = [0; 4];
    stream.read_exact(&mut ack)?;
    ensure!(&ack == b"OKAY", "stage session authentication rejected");
    stream.sock.set_phase_timeout(Duration::from_secs(900))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_requires_usb_certificate_and_session_token() {
        use rustls::{pki_types::PrivatePkcs8KeyDer, ServerConfig, ServerConnection};
        let identity = rcgen::generate_simple_self_signed(vec!["couch-probe".into()]).unwrap();
        let cert = identity.cert.der().to_vec();
        let other = rcgen::generate_simple_self_signed(vec!["couch-probe".into()]).unwrap();
        let config = Arc::new(
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(cert.clone())],
                    PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into(),
                )
                .unwrap(),
        );
        for (trusted, right_token) in [(true, true), (false, true), (true, false)] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let config = config.clone();
            let server = std::thread::spawn(move || {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut stream = StreamOwned::new(ServerConnection::new(config).unwrap(), socket);
                let mut token = [0; 32];
                let received = stream.read_exact(&mut token).is_ok();
                if received {
                    let _ = stream.write_all(if token == [7; 32] { b"OKAY" } else { b"NOPE" });
                    let _ = stream.flush();
                }
                received
            });
            let selected = if trusted {
                cert.as_slice()
            } else {
                other.cert.der().as_ref()
            };
            let result = connect(address, selected, &[if right_token { 7 } else { 0 }; 32]);
            assert_eq!(result.is_ok(), trusted && right_token);
            drop(result);
            assert_eq!(
                server.join().unwrap(),
                trusted,
                "token must never precede TLS pin validation"
            );
        }
    }
    #[test]
    fn invalid_certificate_fails_before_connecting() {
        assert!(connect(
            "127.0.0.1:1".parse().unwrap(),
            b"not a certificate",
            &[0; 32]
        )
        .is_err());
    }
    #[test]
    fn stalled_connection_times_out_but_progress_extends_the_idle_window() {
        use std::io::Write as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        // Idle window already elapsed: a read with no data pending times out.
        let mut deadline = DeadlineSocket {
            socket,
            idle: Duration::from_secs(1),
            last: Instant::now() - Duration::from_secs(2),
        };
        assert_eq!(
            deadline.read(&mut [0; 1]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        // A received byte resets the window: after progress a fresh read waits the
        // full idle window again, so a slow-but-steady transfer never aborts.
        deadline.set_phase_timeout(Duration::from_secs(1)).unwrap();
        deadline.last = Instant::now() - Duration::from_millis(900);
        peer.write_all(b"x").unwrap();
        peer.flush().unwrap();
        let mut byte = [0; 1];
        assert_eq!(deadline.read(&mut byte).unwrap(), 1);
        assert!(
            deadline.remaining().unwrap() > Duration::from_millis(500),
            "progress must extend the idle window"
        );
        // The idle window is still bounded and must be positive and <= 30 min.
        assert!(deadline.set_phase_timeout(Duration::ZERO).is_err());
        assert!(deadline
            .set_phase_timeout(Duration::from_secs(1801))
            .is_err());
    }
}
