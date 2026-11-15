//! Regression tests for the RDP security layer, against in-process servers (no Docker):
//!
//! - a server that presents the right certificate but signs the handshake with a key it does
//!   not own must be refused before any certificate prompt (the pin cannot be bypassed with a
//!   copy of the public certificate);
//! - with a credential only NLA (CredSSP) is offered, and a server that answers with plain TLS
//!   or says it needs TLS is refused with a message that names the "allow without NLA" option;
//! - with that option on, TLS-level security is offered as well.
//!
//! Every network wait has a timeout.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use portolano_session_rdp::cert::CertStore;
use portolano_session_rdp::tls;
use portolano_session_rdp::{EventSink, FbEvent, RdpConfig, RdpCredentials, RdpSession};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::{CertifiedKey, Signer, SigningKey};
use tokio_rustls::rustls::{Error as TlsError, ServerConfig, SignatureAlgorithm, SignatureScheme};
use tokio_rustls::TlsAcceptor;

const WAIT: Duration = Duration::from_secs(20);
const PROTOCOL_SSL: u32 = 1;
const PROTOCOL_HYBRID: u32 = 2;
const PROTOCOL_HYBRID_EX: u32 = 8;

fn data_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.tmp/secfix/rdp-tls")
        .join(name);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "cleaning {dir:?}");
    }
    std::fs::create_dir_all(&dir).expect("create the data dir");
    dir
}

struct Identity {
    der: Vec<u8>,
    key: Vec<u8>,
}

fn identity() -> Identity {
    let generated =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("certificate");
    Identity {
        der: generated.cert.der().to_vec(),
        key: generated.key_pair.serialize_der(),
    }
}

/// A key that claims to be the certificate's but signs garbage.
#[derive(Debug)]
struct ForgedKey;

#[derive(Debug)]
struct ForgedSigner;

impl Signer for ForgedSigner {
    fn sign(&self, _message: &[u8]) -> Result<Vec<u8>, TlsError> {
        Ok(vec![0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01])
    }
    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::ECDSA_NISTP256_SHA256
    }
}

impl SigningKey for ForgedKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered
            .contains(&SignatureScheme::ECDSA_NISTP256_SHA256)
            .then(|| Box::new(ForgedSigner) as Box<dyn Signer>)
    }
    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ECDSA
    }
}

#[derive(Debug)]
struct Fixed(Arc<CertifiedKey>);

impl ResolvesServerCert for Fixed {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// `forged`: the right certificate, a signature from a key the server does not hold.
fn acceptor(id: &Identity, forged: bool) -> TlsAcceptor {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth();
    let cert = CertificateDer::from(id.der.clone());
    let config = if forged {
        let certified = CertifiedKey::new(vec![cert], Arc::new(ForgedKey));
        builder.with_cert_resolver(Arc::new(Fixed(Arc::new(certified))))
    } else {
        builder
            .with_single_cert(
                vec![cert],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key.clone())),
            )
            .expect("server certificate")
    };
    TlsAcceptor::from(Arc::new(config))
}

async fn bind() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    (listener, port)
}

// ---- the TLS layer on its own ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_handshake_signature_is_refused() {
    let id = identity();
    let (listener, port) = bind().await;
    let acceptor = acceptor(&id, true);
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept");
        // The server side fails as well (the client aborts); the outcome is not interesting.
        tokio::time::timeout(WAIT, acceptor.accept(tcp)).await.ok();
    });
    let tcp = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let result = tokio::time::timeout(WAIT, tls::upgrade(tcp, "127.0.0.1"))
        .await
        .expect("no timeout");
    let Err(error) = result else {
        panic!("a forged handshake signature was accepted");
    };
    assert!(
        error.to_string().to_lowercase().contains("signature")
            || error.to_string().to_lowercase().contains("invalid"),
        "unexpected error: {error}"
    );
    tokio::time::timeout(WAIT, server)
        .await
        .expect("server task")
        .expect("join");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_owns_its_key_is_accepted_and_the_certificate_is_returned() {
    let id = identity();
    let (listener, port) = bind().await;
    let acceptor = acceptor(&id, false);
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept");
        let mut stream = tokio::time::timeout(WAIT, acceptor.accept(tcp))
            .await
            .expect("no timeout")
            .expect("handshake");
        stream.write_all(b"ok").await.expect("write");
        stream.flush().await.expect("flush");
        // Keep the stream open until the client has read.
        tokio::time::sleep(Duration::from_millis(300)).await;
    });
    let tcp = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let upgraded = tokio::time::timeout(WAIT, tls::upgrade(tcp, "127.0.0.1"))
        .await
        .expect("no timeout")
        .expect("handshake");
    assert_eq!(upgraded.certificate, id.der);
    assert!(tls::server_public_key(&upgraded.certificate).is_some());
    let mut stream = upgraded.stream;
    let mut buf = [0u8; 2];
    tokio::time::timeout(WAIT, stream.read_exact(&mut buf))
        .await
        .expect("no timeout")
        .expect("read");
    assert_eq!(&buf, b"ok");
    server.await.expect("join");
}

// ---- the whole session against a fake RDP server ----

/// What the fake server does after the X.224 connection request.
#[derive(Clone, Copy)]
enum Reply {
    /// Selects this protocol and goes on with the TLS handshake.
    Select(u32, bool),
    /// Negotiation failure with this code.
    Failure(u32),
}

struct Seen {
    requested: Option<u32>,
    /// Bytes that arrived after the X.224 exchange (a TLS ClientHello starts with 0x16).
    after: Vec<u8>,
}

fn confirm(reply: Reply) -> Vec<u8> {
    let (kind, value) = match reply {
        Reply::Select(protocol, _) => (0x02u8, protocol),
        Reply::Failure(code) => (0x03u8, code),
    };
    let mut pdu = vec![
        0x03, 0x00, 0x00, 0x13, 0x0E, 0xD0, 0x00, 0x00, 0x12, 0x34, 0x00,
    ];
    pdu.extend_from_slice(&[kind, 0x00, 0x08, 0x00]);
    pdu.extend_from_slice(&value.to_le_bytes());
    pdu
}

async fn fake_server(
    listener: TcpListener,
    reply: Reply,
    acceptor: TlsAcceptor,
) -> Result<Seen, String> {
    let (mut tcp, _) = listener.accept().await.map_err(|e| e.to_string())?;
    let mut header = [0u8; 4];
    tcp.read_exact(&mut header)
        .await
        .map_err(|e| e.to_string())?;
    let len = usize::from(u16::from_be_bytes([header[2], header[3]]));
    let mut body = vec![0u8; len.saturating_sub(4)];
    tcp.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    // The negotiation request is the last 8 bytes: type 1, flags, length 8, protocols.
    let requested = body
        .len()
        .checked_sub(8)
        .filter(|at| body[*at] == 0x01)
        .map(|at| u32::from_le_bytes([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]));
    tcp.write_all(&confirm(reply))
        .await
        .map_err(|e| e.to_string())?;
    let mut after = Vec::new();
    if let Reply::Select(_, true) = reply {
        // The TLS handshake: it is expected to fail on the client side.
        tokio::time::timeout(WAIT, acceptor.accept(tcp)).await.ok();
    } else {
        let mut buf = [0u8; 64];
        if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(2), tcp.read(&mut buf)).await {
            after.extend_from_slice(&buf[..n]);
        }
    }
    Ok(Seen { requested, after })
}

fn credentials() -> RdpCredentials {
    RdpCredentials {
        username: "someone".into(),
        password: SecretString::from("a-password-for-tests-only".to_owned()),
    }
}

/// Runs a session against the fake server and returns its events (until it ends) and what the
/// server saw.
async fn run_session(
    name: &str,
    reply: Reply,
    forged: bool,
    allow_without_nla: bool,
) -> (Vec<FbEvent>, Seen) {
    let id = identity();
    let (listener, port) = bind().await;
    let server = tokio::spawn(fake_server(listener, reply, acceptor(&id, forged)));
    let store = CertStore::in_data_dir(&data_dir(name));
    let mut config = RdpConfig::new("127.0.0.1", port, 800, 600, store);
    config.credentials = Some(credentials());
    config.allow_without_nla = allow_without_nla;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink: EventSink = Arc::new(move |event| tx.send(event).is_ok());
    let session = RdpSession::start(config, sink);
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(event)) => {
                let end = matches!(event, FbEvent::Error(_) | FbEvent::Closed { .. });
                events.push(event);
                if end {
                    break;
                }
            }
            _ => break,
        }
    }
    session.close();
    tokio::time::timeout(WAIT, session.closed())
        .await
        .expect("the session ended");
    let seen = tokio::time::timeout(WAIT, server)
        .await
        .expect("server task")
        .expect("join")
        .expect("server");
    (events, seen)
}

fn error_message(events: &[FbEvent]) -> String {
    events
        .iter()
        .find_map(|e| match e {
            FbEvent::Error(err) => Some(err.message.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no error event in {events:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_right_certificate_with_a_forged_signature_never_reaches_the_prompt() {
    let (events, seen) =
        run_session("forged", Reply::Select(PROTOCOL_HYBRID, true), true, false).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, FbEvent::CertPrompt { .. })),
        "a forged handshake must not even reach the certificate decision: {events:?}"
    );
    assert!(error_message(&events).contains("TLS handshake failed"));
    assert!(seen.requested.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_a_credential_only_nla_is_offered() {
    let (events, seen) = run_session(
        "offer-nla",
        Reply::Failure(1), // the server wants TLS: enough to read the request
        false,
        false,
    )
    .await;
    let requested = seen.requested.expect("a negotiation request");
    assert_eq!(requested & PROTOCOL_SSL, 0, "TLS-only must not be offered");
    assert_ne!(requested & PROTOCOL_HYBRID, 0);
    assert_ne!(requested & PROTOCOL_HYBRID_EX, 0);
    let message = error_message(&events);
    assert!(
        message.contains("Allow connections without NLA"),
        "the error must name the option: {message}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_answers_with_tls_only_is_refused_before_the_handshake() {
    let (events, seen) = run_session(
        "downgrade",
        Reply::Select(PROTOCOL_SSL, false),
        false,
        false,
    )
    .await;
    let message = error_message(&events);
    assert!(
        message.contains("Allow connections without NLA"),
        "the error must name the option: {message}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, FbEvent::CertPrompt { .. })),
        "{events:?}"
    );
    assert!(
        !seen.after.contains(&0x16),
        "no TLS ClientHello may be sent after a downgrade"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_option_offers_tls_as_well() {
    let (_, seen) = run_session("allow", Reply::Failure(1), false, true).await;
    let requested = seen.requested.expect("a negotiation request");
    assert_ne!(requested & PROTOCOL_SSL, 0);
    assert_ne!(requested & PROTOCOL_HYBRID, 0);
}
