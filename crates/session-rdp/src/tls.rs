//! The TLS layer of an RDP session, on `rustls`.
//!
//! RDP servers almost always present self-signed certificates, so the web PKI is of no use.
//! The certificate is accepted at the TLS layer **only to be inspected**: the caller pins the
//! SHA-256 fingerprint of the leaf certificate (see `cert.rs`) and decides on it before any
//! credential is sent.
//!
//! The handshake signatures are verified against the public key of the certificate the server
//! presented. Without that check the pin would be worthless: a certificate is public, so anyone
//! can present a pinned one without owning its private key. (`ironrdp-tls` offers no verifier
//! that checks signatures while skipping the chain validation, hence this module.)

use std::io;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{self, CryptoProvider};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use tokio_rustls::TlsConnector;

pub type TlsStream<S> = tokio_rustls::client::TlsStream<S>;

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

fn client_config() -> io::Result<ClientConfig> {
    let provider = Arc::new(crypto::aws_lc_rs::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InspectOnly { provider }))
        .with_no_client_auth();
    // CredSSP binds to the public key of this very handshake.
    config.resumption = tokio_rustls::rustls::client::Resumption::disabled();
    Ok(config)
}

/// A TLS session and the DER of the leaf certificate the server proved to own.
pub struct Upgraded<S> {
    pub stream: TlsStream<S>,
    pub certificate: Vec<u8>,
}

/// Runs the TLS handshake on `stream` (after the X.224 negotiation selected TLS or CredSSP).
/// Fails when the server cannot prove it owns the certificate it presents.
pub async fn upgrade<S>(stream: S, host: &str) -> io::Result<Upgraded<S>>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    // The name is only a hint for SNI: nothing is validated against it.
    let name = ServerName::try_from(host.to_owned())
        .or_else(|_| ServerName::try_from("rdp-server.invalid".to_owned()))
        .map_err(io::Error::other)?;
    let mut stream = TlsConnector::from(Arc::new(client_config()?))
        .connect(name, stream)
        .await?;
    stream.flush().await?;
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(|der| der.as_ref().to_vec())
        .ok_or_else(|| io::Error::other("peer certificate is missing"))?;
    Ok(Upgraded {
        stream,
        certificate,
    })
}

/// The public key CredSSP binds to: the subject public key bit string of the certificate.
pub fn server_public_key(certificate: &[u8]) -> Option<Vec<u8>> {
    use x509_cert::der::Decode as _;
    let cert = x509_cert::Certificate::from_der(certificate).ok()?;
    ironrdp_tls::extract_tls_server_public_key(&cert).map(<[u8]>::to_vec)
}
