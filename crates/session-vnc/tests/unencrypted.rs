//! Regression tests for the VNC security downgrade, against an in-process RFB server (no
//! Docker):
//!
//! - before a VNC password is answered over an unencrypted connection the user decides, and
//!   nothing derived from the password has left until then;
//! - a host with a stored TLS certificate is refused when it stops offering encryption;
//! - security type None needs no password and no question.
//!
//! Every network wait has a timeout.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portolano_core_model::ErrorCode;
use portolano_session_vnc::cert::{CertStore, PlainStore};
use portolano_session_vnc::{EventSink, FbEvent, VncConfig, VncSession};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const HOST: &str = "127.0.0.1";
const WAIT: Duration = Duration::from_secs(20);
const PASSWORD: &str = "a-password-for-tests-only";

fn data_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.tmp/secfix/vnc-unencrypted")
        .join(name);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "cleaning {dir:?}");
    }
    std::fs::create_dir_all(&dir).expect("create the data dir");
    dir
}

#[derive(Clone, Copy)]
enum Offer {
    /// Security types the server lists (RFB 3.8).
    Types(&'static [u8]),
    /// VeNCrypt with only an anonymous-TLS subtype (unusable for this client).
    VeNCryptAnonymousOnly,
}

/// What the server observed from the client.
#[derive(Default, Debug)]
struct Observed {
    chosen: Option<u8>,
    /// Bytes read after the security type, before the server stopped listening.
    response: Vec<u8>,
}

async fn server(listener: TcpListener, offer: Offer, observed: Arc<Mutex<Observed>>) {
    let Ok((mut tcp, _)) = listener.accept().await else {
        return;
    };
    let result: std::io::Result<()> = async {
        tcp.write_all(b"RFB 003.008\n").await?;
        let mut banner = [0u8; 12];
        tcp.read_exact(&mut banner).await?;
        let types: &[u8] = match offer {
            Offer::Types(t) => t,
            Offer::VeNCryptAnonymousOnly => &[19],
        };
        let mut list = vec![u8::try_from(types.len()).unwrap_or(0)];
        list.extend_from_slice(types);
        tcp.write_all(&list).await?;
        let chosen = tcp.read_u8().await?;
        if let Ok(mut o) = observed.lock() {
            o.chosen = Some(chosen);
        }
        match chosen {
            2 => {
                tcp.write_all(&[7u8; 16]).await?;
                // Whatever the client sends next is recorded (the DES response).
                let mut buf = [0u8; 16];
                let mut got = 0;
                while got < buf.len() {
                    match tokio::time::timeout(WAIT, tcp.read(&mut buf[got..])).await {
                        Ok(Ok(0)) | Err(_) => break,
                        Ok(Ok(n)) => {
                            if let Ok(mut o) = observed.lock() {
                                o.response.extend_from_slice(&buf[got..got + n]);
                            }
                            got += n;
                        }
                        Ok(Err(e)) => return Err(e),
                    }
                }
                tcp.write_all(&0u32.to_be_bytes()).await?;
            }
            1 => tcp.write_all(&0u32.to_be_bytes()).await?,
            19 => {
                // VeNCrypt 0.2, accepted, one subtype: TLSNone (anonymous).
                tcp.write_all(&[0, 2]).await?;
                let mut version = [0u8; 2];
                tcp.read_exact(&mut version).await?;
                tcp.write_all(&[0, 1]).await?;
                tcp.write_all(&257u32.to_be_bytes()).await?;
                return Ok(());
            }
            _ => return Ok(()),
        }
        // ClientInit, then ServerInit (16x16, any pixel format, name "test").
        let mut init = [0u8; 1];
        tcp.read_exact(&mut init).await?;
        let mut server_init = Vec::new();
        server_init.extend_from_slice(&16u16.to_be_bytes());
        server_init.extend_from_slice(&16u16.to_be_bytes());
        server_init.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
        server_init.extend_from_slice(&4u32.to_be_bytes());
        server_init.extend_from_slice(b"test");
        tcp.write_all(&server_init).await?;
        // Keep the connection open until the client goes away.
        let mut sink = [0u8; 256];
        while let Ok(Ok(n)) = tokio::time::timeout(WAIT, tcp.read(&mut sink)).await {
            if n == 0 {
                break;
            }
        }
        Ok(())
    }
    .await;
    // The server side ends when the client closes; there is nothing to report.
    drop(result);
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<FbEvent>>>);

impl Events {
    fn sink(&self) -> EventSink {
        let events = self.0.clone();
        Arc::new(move |event| {
            if let Ok(mut v) = events.lock() {
                v.push(event);
            }
            true
        })
    }
    fn all(&self) -> Vec<FbEvent> {
        self.0.lock().map(|v| v.clone()).unwrap_or_default()
    }
    async fn wait(&self, what: &str, f: impl Fn(&[FbEvent]) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            let all = self.all();
            if f(&all) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {all:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

struct Fixture {
    port: u16,
    observed: Arc<Mutex<Observed>>,
    server: tokio::task::JoinHandle<()>,
}

async fn fixture(offer: Offer) -> Fixture {
    let listener = TcpListener::bind((HOST, 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let observed = Arc::new(Mutex::new(Observed::default()));
    let server = tokio::spawn(server(listener, offer, observed.clone()));
    Fixture {
        port,
        observed,
        server,
    }
}

impl Fixture {
    fn observed(&self) -> (Option<u8>, usize) {
        let o = self.observed.lock().expect("lock");
        (o.chosen, o.response.len())
    }
}

fn config(dir: &std::path::Path, port: u16, password: Option<&str>) -> VncConfig {
    let mut c = VncConfig::new(HOST, port);
    c.password = password.map(|p| SecretString::from(p.to_owned()));
    c.cert_store = Some(CertStore::in_data_dir(dir));
    c.plain_store = Some(PlainStore::in_data_dir(dir));
    c
}

async fn finish(session: VncSession, fixture: Fixture) {
    session.close();
    tokio::time::timeout(WAIT, session.closed())
        .await
        .expect("the session ends after close");
    fixture.server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_derived_from_the_password_leaves_before_the_user_agrees() {
    let dir = data_dir("order");
    let fx = fixture(Offer::Types(&[2])).await;
    let events = Events::default();
    let session = VncSession::start(config(&dir, fx.port, Some(PASSWORD)), events.sink());
    events
        .wait("the prompt", |e| {
            e.iter().any(|e| matches!(e, FbEvent::UnencryptedPrompt))
        })
        .await;
    // The prompt is open: the server has seen the security type and nothing else.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        fx.observed(),
        (Some(2), 0),
        "a response was sent before the decision"
    );
    assert_eq!(events.all().len(), 1, "{:?}", events.all());

    session.unencrypted_decision(true).expect("agree");
    events
        .wait("the connection", |e| {
            e.iter().any(|e| matches!(e, FbEvent::Resize { .. }))
        })
        .await;
    assert_eq!(fx.observed(), (Some(2), 16));
    let all = events.all();
    assert!(all.iter().any(|e| matches!(
        e,
        FbEvent::Security {
            encrypted: false,
            ..
        }
    )));
    assert!(PlainStore::in_data_dir(&dir)
        .allowed(HOST, fx.port)
        .expect("allowed"));
    finish(session, fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declining_closes_cleanly_and_no_response_is_ever_sent() {
    let dir = data_dir("decline");
    let fx = fixture(Offer::Types(&[2])).await;
    let events = Events::default();
    let session = VncSession::start(config(&dir, fx.port, Some(PASSWORD)), events.sink());
    events
        .wait("the prompt", |e| {
            e.iter().any(|e| matches!(e, FbEvent::UnencryptedPrompt))
        })
        .await;
    session.unencrypted_decision(false).expect("decline");
    events
        .wait("the close", |e| {
            e.iter().any(|e| matches!(e, FbEvent::Closed { .. }))
        })
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fx.observed().1, 0);
    assert!(!events.all().iter().any(|e| matches!(e, FbEvent::Error(_))));
    assert!(!PlainStore::in_data_dir(&dir)
        .allowed(HOST, fx.port)
        .expect("allowed"));
    finish(session, fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_earlier_agreement_for_the_host_is_honoured_without_a_prompt() {
    let dir = data_dir("remembered");
    let fx = fixture(Offer::Types(&[2])).await;
    PlainStore::in_data_dir(&dir)
        .allow(HOST, fx.port)
        .expect("agree");
    let events = Events::default();
    let session = VncSession::start(config(&dir, fx.port, Some(PASSWORD)), events.sink());
    events
        .wait("the connection", |e| {
            e.iter().any(|e| matches!(e, FbEvent::Resize { .. }))
        })
        .await;
    assert!(!events
        .all()
        .iter()
        .any(|e| matches!(e, FbEvent::UnencryptedPrompt)));
    finish(session, fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pinned_host_that_stops_offering_tls_is_refused_before_anything_is_sent() {
    let dir = data_dir("pinned");
    let fx = fixture(Offer::Types(&[2])).await;
    CertStore::in_data_dir(&dir)
        .remember(
            HOST,
            fx.port,
            "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF",
        )
        .expect("pin");
    // An earlier agreement does not matter: the pin wins.
    PlainStore::in_data_dir(&dir)
        .allow(HOST, fx.port)
        .expect("agree");
    let events = Events::default();
    let session = VncSession::start(config(&dir, fx.port, Some(PASSWORD)), events.sink());
    events
        .wait("the error", |e| {
            e.iter().any(|e| matches!(e, FbEvent::Error(_)))
        })
        .await;
    let all = events.all();
    let FbEvent::Error(err) = &all[0] else {
        panic!("not an error: {all:?}");
    };
    assert_eq!(err.code, ErrorCode::SecurityDowngrade);
    assert!(err.message.contains("downgrade"), "{}", err.message);
    assert_eq!(all.len(), 1, "no prompt, no desktop: {all:?}");
    assert_eq!(fx.observed().1, 0);
    assert!(
        CertStore::in_data_dir(&dir)
            .has_pin(HOST, fx.port)
            .expect("pin"),
        "the pin stays until the user removes it"
    );
    finish(session, fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pinned_host_whose_vencrypt_has_no_usable_subtype_is_refused_too() {
    let dir = data_dir("pinned-vencrypt");
    let fx = fixture(Offer::VeNCryptAnonymousOnly).await;
    CertStore::in_data_dir(&dir)
        .remember(
            HOST,
            fx.port,
            "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF",
        )
        .expect("pin");
    let events = Events::default();
    let session = VncSession::start(config(&dir, fx.port, Some(PASSWORD)), events.sink());
    events
        .wait("the error", |e| {
            e.iter().any(|e| matches!(e, FbEvent::Error(_)))
        })
        .await;
    let FbEvent::Error(err) = events.all().remove(0) else {
        panic!("not an error");
    };
    assert_eq!(err.code, ErrorCode::SecurityDowngrade);
    finish(session, fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_without_authentication_needs_no_prompt_but_reports_it_is_not_encrypted() {
    let dir = data_dir("none");
    let fx = fixture(Offer::Types(&[1])).await;
    let events = Events::default();
    let session = VncSession::start(config(&dir, fx.port, None), events.sink());
    events
        .wait("the connection", |e| {
            e.iter().any(|e| matches!(e, FbEvent::Resize { .. }))
        })
        .await;
    let all = events.all();
    assert!(!all.iter().any(|e| matches!(e, FbEvent::UnencryptedPrompt)));
    assert!(all.iter().any(|e| matches!(
        e,
        FbEvent::Security {
            encrypted: false,
            ..
        }
    )));
    finish(session, fx).await;
}
