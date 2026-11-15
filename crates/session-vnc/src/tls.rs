//! The TLS layer of VeNCrypt (X509 subtypes), on `rustls`.
//!
//! VNC servers almost always present self-signed certificates, so the web PKI is of no use.
//! Like the RDP session, this accepts any certificate at the TLS layer **only to inspect it**:
//! the caller pins the SHA-256 fingerprint of the leaf certificate (see `cert.rs`) and must
//! decide on it before sending anything secret. The handshake signatures are still verified
//! against the certificate's public key, so the peer proves it holds the key it presented.
//!
//! The anonymous Diffie-Hellman suites of the TLSNone/TLSVnc/TLSPlain subtypes do not exist in
//! rustls and are not supported.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{self, CryptoProvider};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use tokio_rustls::TlsConnector;

use crate::cert::fingerprint_sha256;
use crate::error::VncError;

pub type TlsStream = tokio_rustls::client::TlsStream<TcpStream>;

/// The connection of a session: plain TCP, or TCP wrapped in TLS after a VeNCrypt upgrade.
pub enum Conn {
    Plain(TcpStream),
    Tls(Box<TlsStream>),
}

impl Conn {
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Self::Tls(_))
    }
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Accepts every certificate (the caller pins it) but checks the handshake signatures.
#[derive(Debug)]
struct InspectOnly {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for InspectOnly {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn client_config() -> Result<ClientConfig, VncError> {
    let provider = Arc::new(crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| VncError::Io(format!("cannot configure TLS: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InspectOnly { provider }))
        .with_no_client_auth();
    Ok(config)
}

/// A TLS session on the connection and the fingerprint of the certificate the server sent.
pub struct Upgraded {
    pub stream: TlsStream,
    pub fingerprint: String,
}

/// Runs the TLS handshake on `tcp` (right after the VeNCrypt subtype was acknowledged).
pub async fn upgrade(tcp: TcpStream, host: &str) -> Result<Upgraded, VncError> {
    // The name is only a hint for SNI: nothing is validated against it.
    let name = ServerName::try_from(host.to_owned())
        .or_else(|_| ServerName::try_from("vnc-server.invalid".to_owned()))
        .map_err(|e| VncError::Io(format!("invalid server name: {e}")))?;
    let connector = TlsConnector::from(Arc::new(client_config()?));
    let stream = connector
        .connect(name, tcp)
        .await
        .map_err(|e| VncError::Io(format!("the TLS handshake failed: {e}")))?;
    let fingerprint = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(|der| fingerprint_sha256(der.as_ref()))
        .ok_or_else(|| VncError::Io("the server sent no certificate".into()))?;
    Ok(Upgraded {
        stream,
        fingerprint,
    })
}
