//! Hue certificates name the bridge ID, not its IP. Trust on first pairing,
//! then pin the exact certificate. TLS handshake signatures are always checked.
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, ClientConnection, StreamOwned,
};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
};
use ureq::unversioned::{
    resolver::DefaultResolver,
    transport::{
        Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, TcpConnector, Transport,
        TransportAdapter,
    },
};
#[derive(Debug)]
struct Pin {
    certificate: Arc<Mutex<Vec<u8>>>,
}
impl ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut pin = self
            .certificate
            .lock()
            .map_err(|_| rustls::Error::General("Certificate lock failed".into()))?;
        if pin.is_empty() {
            *pin = cert.as_ref().to_vec();
        }
        if pin.as_slice() != cert.as_ref() {
            return Err(rustls::Error::General(
                "Hue bridge certificate changed; pair again".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            m,
            c,
            s,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            s,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
#[derive(Debug)]
struct PinnedConnector(Arc<ClientConfig>);
impl<In: Transport> Connector<In> for PinnedConnector {
    type Out = PinnedTransport;
    fn connect(
        &self,
        d: &ConnectionDetails,
        input: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let input = input.ok_or(ureq::Error::Tls("Missing TCP connection"))?;
        let name = ServerName::try_from(
            d.uri
                .host()
                .ok_or(ureq::Error::Tls("Missing host"))?
                .to_string(),
        )
        .map_err(|_| ureq::Error::Tls("Invalid host"))?;
        let mut conn = ClientConnection::new(self.0.clone(), name)?;
        let mut sock = TransportAdapter::new(input.boxed());
        sock.set_timeout(d.timeout);
        conn.complete_io(&mut sock)?;
        Ok(Some(PinnedTransport {
            stream: StreamOwned::new(conn, sock),
            buffers: LazyBuffers::new(8192, 8192),
        }))
    }
}
struct PinnedTransport {
    stream: StreamOwned<ClientConnection, TransportAdapter>,
    buffers: LazyBuffers,
}
impl Transport for PinnedTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }
    fn transmit_output(&mut self, n: usize, t: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.sock.set_timeout(t);
        self.stream.write_all(&self.buffers.output()[..n])?;
        Ok(())
    }
    fn await_input(&mut self, t: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.sock.set_timeout(t);
        let n = self.stream.read(self.buffers.input_append_buf())?;
        self.buffers.input_appended(n);
        Ok(n > 0)
    }
    fn is_open(&mut self) -> bool {
        self.stream.sock.get_mut().is_open()
    }
    fn is_tls(&self) -> bool {
        true
    }
}
pub fn agent(certificate: Arc<Mutex<Vec<u8>>>) -> ureq::Agent {
    let tls =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pin { certificate }))
            .with_no_client_auth();
    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .max_redirects(0)
        .proxy(None)
        .build();
    ureq::Agent::with_parts(
        cfg,
        TcpConnector::default().chain(PinnedConnector(Arc::new(tls))),
        DefaultResolver::default(),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_certificate_is_rejected() {
        let pin = Pin {
            certificate: Arc::new(Mutex::new(vec![1, 2, 3])),
        };
        let name = ServerName::try_from("bridge").unwrap();
        assert!(pin
            .verify_server_cert(
                &CertificateDer::from(vec![1, 2, 3]),
                &[],
                &name,
                &[],
                UnixTime::since_unix_epoch(std::time::Duration::ZERO)
            )
            .is_ok());
        assert!(pin
            .verify_server_cert(
                &CertificateDer::from(vec![1, 2, 4]),
                &[],
                &name,
                &[],
                UnixTime::since_unix_epoch(std::time::Duration::ZERO)
            )
            .is_err());
    }
}

impl std::fmt::Debug for PinnedTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinnedTransport").finish_non_exhaustive()
    }
}
