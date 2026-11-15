//! Integration tests against the VeNCrypt test server (`dev/servers.sh up`, service `vnc-tls`,
//! 127.0.0.1:15901: TLS with a throwaway self-signed certificate, security types X509Vnc and
//! X509Plain; user "tester", password = `VNC_PASSWORD` of `dev/keys/test.env`).
//!
//! Every test skips with a message when the server is not reachable and has a timeout on every
//! wait. Data directories live under `.tmp/wp10b/`. Nothing here looks at the screen.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portolano_core_model::prompt::{CredentialAnswer, PromptHooks, PromptKind};
use portolano_core_model::ErrorCode;
use portolano_session_vnc::cert::{CertCheck, CertStore, PlainStore};
use portolano_session_vnc::{EventSink, FbEvent, VncAsk, VncConfig, VncSession};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const HOST: &str = "127.0.0.1";
const TLS_PORT: u16 = 15901;
const PLAIN_PORT: u16 = 15900;
const USER: &str = "tester";
const WAIT: Duration = Duration::from_secs(20);

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn password() -> Option<String> {
    let text = std::fs::read_to_string(repo().join("dev/keys/test.env")).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("VNC_PASSWORD="))
        .map(|v| v.trim().trim_matches('"').to_owned())
}

async fn reachable(port: u16) -> bool {
    tokio::time::timeout(Duration::from_secs(2), TcpStream::connect((HOST, port)))
        .await
        .is_ok_and(|r| r.is_ok())
}

/// The password, or `None` (with a message) when the TLS server cannot be used.
async fn tls_server() -> Option<String> {
    let Some(password) = password() else {
        eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
        return None;
    };
    if !reachable(TLS_PORT).await {
        eprintln!("SKIP: no VeNCrypt server on {HOST}:{TLS_PORT} (dev/servers.sh up)");
        return None;
    }
    Some(password)
}

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

/// A fresh, empty data directory of a test.
fn data_dir(name: &str) -> PathBuf {
    let dir = repo().join(".tmp/wp10b/tls").join(name);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "cleaning {dir:?}");
    }
    std::fs::create_dir_all(&dir).expect("create the data dir");
    dir
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

    fn position(&self, f: impl Fn(&FbEvent) -> bool) -> Option<usize> {
        self.all().iter().position(f)
    }

    fn count(&self, f: impl Fn(&FbEvent) -> bool) -> usize {
        self.all().iter().filter(|e| f(e)).count()
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
                "timed out waiting for {what}; events: {:?}",
                summary(&all)
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

fn summary(events: &[FbEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            FbEvent::Rect { .. } => "Rect".to_owned(),
            FbEvent::Cursor { .. } => "Cursor".to_owned(),
            FbEvent::Error(err) => format!("Error({:?}: {})", err.code, err.message),
            FbEvent::CredentialPrompt(p) => {
                format!("CredentialPrompt({:?}, {:?})", p.kind, p.error)
            }
            other => format!("{other:?}"),
        })
        .collect()
}

fn is_rect(e: &FbEvent) -> bool {
    matches!(e, FbEvent::Rect { .. })
}
fn is_cert(e: &FbEvent) -> bool {
    matches!(e, FbEvent::CertPrompt { .. })
}
fn is_cred(e: &FbEvent) -> bool {
    matches!(e, FbEvent::CredentialPrompt(_))
}
fn is_resize(e: &FbEvent) -> bool {
    matches!(e, FbEvent::Resize { .. })
}
fn is_security(e: &FbEvent) -> bool {
    matches!(e, FbEvent::Security { .. })
}
fn is_error(e: &FbEvent) -> bool {
    matches!(e, FbEvent::Error(_))
}
fn is_closed(e: &FbEvent) -> bool {
    matches!(e, FbEvent::Closed { .. })
}

fn cert_prompt(events: &Events) -> (String, bool) {
    events
        .all()
        .into_iter()
        .find_map(|e| match e {
            FbEvent::CertPrompt {
                fingerprint,
                changed,
                ..
            } => Some((fingerprint, changed)),
            _ => None,
        })
        .expect("a certificate prompt")
}

fn tls_config(dir: &std::path::Path, host_port: u16, password: Option<&str>) -> VncConfig {
    let mut c = VncConfig::new(HOST, host_port);
    c.username = Some(USER.to_owned());
    c.password = password.map(secret);
    c.cert_store = Some(CertStore::in_data_dir(dir));
    c
}

fn ask_config(dir: &std::path::Path) -> VncConfig {
    let mut c = VncConfig::new(HOST, TLS_PORT);
    c.cert_store = Some(CertStore::in_data_dir(dir));
    c.ask = Some(VncAsk {
        hooks: PromptHooks::no_vault(),
    });
    c
}

fn start(config: VncConfig) -> (VncSession, Events) {
    let events = Events::default();
    let session = VncSession::start(config, events.sink());
    (session, events)
}

async fn finish(session: VncSession) {
    session.close();
    tokio::time::timeout(WAIT, session.closed())
        .await
        .expect("the session ends after close");
}

fn answer(user: &str, password: &str) -> CredentialAnswer {
    CredentialAnswer {
        username: user.to_owned(),
        password: Some(secret(password)),
        key_path: None,
        passphrase: None,
        domain: None,
    }
}

/// A local relay that counts the bytes the client sends to the server, so a test can show
/// that nothing (TLS records included) leaves between the certificate prompt and the decision.
struct Relay {
    port: u16,
    sent: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Relay {
    async fn start(target: u16) -> Self {
        let listener = TcpListener::bind((HOST, 0)).await.expect("bind the relay");
        let port = listener.local_addr().expect("relay address").port();
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                let Ok(server) = TcpStream::connect((HOST, target)).await else {
                    return;
                };
                let (mut cr, mut cw) = client.into_split();
                let (mut sr, mut sw) = server.into_split();
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    while let Ok(n) = cr.read(&mut buf).await {
                        if n == 0 || sw.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                        counter.fetch_add(n, Ordering::SeqCst);
                    }
                });
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    while let Ok(n) = sr.read(&mut buf).await {
                        if n == 0 || cw.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        Self { port, sent, task }
    }

    fn sent(&self) -> usize {
        self.sent.load(Ordering::SeqCst)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_contact_prompts_and_sends_no_credential_before_the_decision() {
    let Some(password) = tls_server().await else {
        return;
    };
    let dir = data_dir("first");
    let relay = Relay::start(TLS_PORT).await;
    let (session, events) = start(tls_config(&dir, relay.port, Some(&password)));
    events
        .wait("the certificate prompt", |e| e.iter().any(is_cert))
        .await;
    let (fingerprint, changed) = cert_prompt(&events);
    assert!(!changed);
    assert_eq!(fingerprint.split(':').count(), 32, "SHA-256: {fingerprint}");

    // The TLS handshake is done and the user has not answered: nothing may be sent, and
    // nothing may reach the UI but the prompt.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = relay.sent();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        relay.sent(),
        before,
        "bytes were sent while the prompt was open"
    );
    assert_eq!(events.all().len(), 1, "{:?}", summary(&events.all()));

    session.cert_decision(true).expect("decision");
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    assert!(
        relay.sent() > before,
        "the credentials went out after the decision"
    );

    // Order: prompt, then the security report, then the desktop.
    let cert = events.position(is_cert).expect("cert");
    let security = events.position(is_security).expect("security");
    let resize = events.position(is_resize).expect("resize");
    assert!(cert < security && security < resize);
    assert!(events
        .all()
        .iter()
        .any(|e| matches!(e, FbEvent::Security { encrypted: true })));
    assert_eq!(events.count(is_cert), 1);

    // The store pins exactly what was shown.
    let store = CertStore::in_data_dir(&dir);
    assert_eq!(
        store.check(HOST, relay.port, &fingerprint).expect("check"),
        CertCheck::Matches
    );

    // No secret in anything the UI received.
    let bytes: Vec<u8> = events.all().iter().flat_map(FbEvent::encode).collect();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains(&password));
    finish(session).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_second_connection_is_silent_and_a_tampered_pin_is_a_change() {
    let Some(password) = tls_server().await else {
        return;
    };
    let dir = data_dir("second");
    let (session, events) = start(tls_config(&dir, TLS_PORT, Some(&password)));
    events
        .wait("the certificate prompt", |e| e.iter().any(is_cert))
        .await;
    session.cert_decision(true).expect("accept");
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    let (fingerprint, _) = cert_prompt(&events);
    finish(session).await;

    // Same store: no prompt at all.
    let (session, events) = start(tls_config(&dir, TLS_PORT, Some(&password)));
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    assert_eq!(events.count(is_cert), 0, "{:?}", summary(&events.all()));
    assert!(events
        .all()
        .iter()
        .any(|e| matches!(e, FbEvent::Security { encrypted: true })));
    finish(session).await;

    // A different pinned fingerprint: the attempt ends with the changed-certificate report
    // (both fingerprints), no question is asked and the pin stays.
    let store = CertStore::in_data_dir(&dir);
    let tampered = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";
    store.remember(HOST, TLS_PORT, tampered).expect("tamper");
    let relay = Relay::start(TLS_PORT).await;
    store
        .remember(HOST, relay.port, tampered)
        .expect("tamper the relay entry");
    let (session, events) = start(tls_config(&dir, relay.port, Some(&password)));
    events
        .wait("the changed report", |e| e.iter().any(is_cert))
        .await;
    let (shown, changed) = cert_prompt(&events);
    assert!(changed);
    assert_eq!(shown, fingerprint);
    let stored_in_event = events.all().into_iter().find_map(|e| match e {
        FbEvent::CertPrompt { stored, .. } => stored,
        _ => None,
    });
    assert_eq!(stored_in_event.as_deref(), Some(tampered));
    // The session is over by itself: nothing else is reported, nothing more is sent, and a
    // decision (a late "accept" click) cannot revive it.
    tokio::time::timeout(WAIT, session.closed())
        .await
        .expect("a changed certificate ends the attempt");
    let sent = relay.sent();
    assert!(session.cert_decision(true).is_err() || relay.sent() == sent);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(relay.sent(), sent, "nothing may be sent after the report");
    assert_eq!(events.all().len(), 1, "{:?}", summary(&events.all()));
    assert_eq!(
        store.check(HOST, relay.port, &fingerprint).expect("check"),
        CertCheck::Changed {
            stored: tampered.to_owned()
        },
        "the pin must not be replaced"
    );
    finish(session).await;

    // Only a deliberate removal of the stored certificate lets the host be trusted again; it
    // then looks like a new one.
    assert!(store.forget(HOST, relay.port).expect("forget"));
    let (session, events) = start(tls_config(&dir, relay.port, Some(&password)));
    events
        .wait("the new-certificate prompt", |e| e.iter().any(is_cert))
        .await;
    let (_, changed) = cert_prompt(&events);
    assert!(!changed);
    session.cert_decision(true).expect("accept");
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    assert_eq!(
        store.check(HOST, relay.port, &fingerprint).expect("check"),
        CertCheck::Matches
    );
    finish(session).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejecting_a_new_certificate_closes_cleanly_and_stores_nothing() {
    let Some(password) = tls_server().await else {
        return;
    };
    let dir = data_dir("reject");
    let relay = Relay::start(TLS_PORT).await;
    let (session, events) = start(tls_config(&dir, relay.port, Some(&password)));
    events
        .wait("the certificate prompt", |e| e.iter().any(is_cert))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = relay.sent();
    session.cert_decision(false).expect("reject");
    events.wait("the close", |e| e.iter().any(is_closed)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        relay.sent(),
        before,
        "nothing may be sent after a rejection"
    );
    let all = events.all();
    assert!(!all
        .iter()
        .any(|e| is_error(e) || is_rect(e) || is_resize(e) || is_security(e)));
    assert!(
        !CertStore::in_data_dir(&dir).path().exists(),
        "nothing may be stored"
    );
    finish(session).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_credentials_ask_again_without_a_second_certificate_prompt() {
    let Some(password) = tls_server().await else {
        return;
    };
    let dir = data_dir("wrong");
    let (session, events) = start(ask_config(&dir));
    events
        .wait("the certificate prompt", |e| e.iter().any(is_cert))
        .await;
    // Credentials come only after the certificate decision.
    assert_eq!(events.count(is_cred), 0);
    session.cert_decision(true).expect("accept");
    events
        .wait("the credential prompt", |e| e.iter().any(is_cred))
        .await;
    let FbEvent::CredentialPrompt(first) = events.all().into_iter().find(is_cred).expect("p")
    else {
        panic!("not a prompt");
    };
    // The server offers Plain, which the client prefers: user name and password.
    assert_eq!(first.kind, PromptKind::UsernamePassword);
    assert_eq!(first.error, None);
    assert_eq!(first.target, format!("{HOST}:{TLS_PORT}"));

    session
        .credential(answer(USER, "definitely-wrong"), false)
        .expect("wrong answer");
    events
        .wait("the second prompt", |e| {
            e.iter().filter(|e| is_cred(e)).count() == 2
        })
        .await;
    let FbEvent::CredentialPrompt(second) =
        events.all().into_iter().filter(is_cred).nth(1).expect("p2")
    else {
        panic!("not a prompt");
    };
    assert_eq!(second.kind, PromptKind::UsernamePassword);
    assert!(second.error.is_some());
    assert_eq!(
        events.count(is_cert),
        1,
        "the certificate was asked about again"
    );
    assert_eq!(events.count(is_error), 0);

    session
        .credential(answer(USER, &password), false)
        .expect("right answer");
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    assert_eq!(events.count(is_cert), 1);
    let bytes: Vec<u8> = events.all().iter().flat_map(FbEvent::encode).collect();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("definitely-wrong"));
    assert!(!text.contains(&password));
    finish(session).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_vault_password_is_final_auth_failed() {
    let Some(_password) = tls_server().await else {
        return;
    };
    let dir = data_dir("vaultwrong");
    let (session, events) = start(tls_config(&dir, TLS_PORT, Some("not-the-password")));
    events
        .wait("the certificate prompt", |e| e.iter().any(is_cert))
        .await;
    session.cert_decision(true).expect("accept");
    events.wait("the error", |e| e.iter().any(is_error)).await;
    let code = events.all().into_iter().find_map(|e| match e {
        FbEvent::Error(err) => Some(err.code),
        _ => None,
    });
    assert_eq!(code, Some(ErrorCode::AuthFailed));
    finish(session).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_vnc_still_works_and_reports_it_is_not_encrypted() {
    let Some(password) = password() else {
        eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
        return;
    };
    if !reachable(PLAIN_PORT).await {
        eprintln!("SKIP: no VNC server on {HOST}:{PLAIN_PORT} (dev/servers.sh up)");
        return;
    }
    let dir = data_dir("plain");
    let mut config = VncConfig::new(HOST, PLAIN_PORT);
    config.password = Some(secret(&password));
    config.cert_store = Some(CertStore::in_data_dir(&dir));
    // The user agreed to this host before (see `plain_vnc_asks_before_the_password_exchange`).
    let plain = PlainStore::in_data_dir(&dir);
    plain.allow(HOST, PLAIN_PORT).expect("agree");
    config.plain_store = Some(plain);
    let (session, events) = start(config);
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    assert_eq!(events.count(is_cert), 0);
    assert!(events
        .all()
        .iter()
        .any(|e| matches!(e, FbEvent::Security { encrypted: false })));
    assert!(!CertStore::in_data_dir(&dir).path().exists());
    finish(session).await;
}

fn is_unencrypted_prompt(e: &FbEvent) -> bool {
    matches!(e, FbEvent::UnencryptedPrompt)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_vnc_asks_before_the_password_exchange_and_remembers_the_answer() {
    let Some(password) = password() else {
        eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
        return;
    };
    if !reachable(PLAIN_PORT).await {
        eprintln!("SKIP: no VNC server on {HOST}:{PLAIN_PORT} (dev/servers.sh up)");
        return;
    }
    let dir = data_dir("plain-asks");
    let relay = Relay::start(PLAIN_PORT).await;
    let config = |relay_port| {
        let mut c = VncConfig::new(HOST, relay_port);
        c.password = Some(secret(&password));
        c.cert_store = Some(CertStore::in_data_dir(&dir));
        c.plain_store = Some(PlainStore::in_data_dir(&dir));
        c
    };

    // Declining: the connection closes and the password response never goes out.
    let (session, events) = start(config(relay.port));
    events
        .wait("the encryption prompt", |e| {
            e.iter().any(is_unencrypted_prompt)
        })
        .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let waiting = relay.sent();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        relay.sent(),
        waiting,
        "bytes were sent while the prompt was open"
    );
    // Version banner (12) and the chosen security type (1); the 16-byte response is not there.
    assert!(
        waiting <= 13,
        "only the negotiation may have been sent, got {waiting}"
    );
    session.unencrypted_decision(false).expect("decline");
    events.wait("the close", |e| e.iter().any(is_closed)).await;
    assert!(!events
        .all()
        .iter()
        .any(|e| is_error(e) || is_rect(e) || is_security(e)));
    assert_eq!(relay.sent(), waiting, "nothing is sent after declining");
    assert!(!PlainStore::in_data_dir(&dir)
        .allowed(HOST, relay.port)
        .expect("allowed"));
    finish(session).await;

    // Agreeing: the session connects, and the answer is remembered for this host.
    let (session, events) = start(config(relay.port));
    events
        .wait("the encryption prompt", |e| {
            e.iter().any(is_unencrypted_prompt)
        })
        .await;
    session.unencrypted_decision(true).expect("agree");
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    assert!(relay.sent() > waiting);
    assert!(events
        .all()
        .iter()
        .any(|e| matches!(e, FbEvent::Security { encrypted: false })));
    finish(session).await;
    assert!(PlainStore::in_data_dir(&dir)
        .allowed(HOST, relay.port)
        .expect("allowed"));

    let (session, events) = start(config(relay.port));
    events
        .wait("the first Rect", |e| e.iter().any(is_rect))
        .await;
    assert_eq!(
        events.count(is_unencrypted_prompt),
        0,
        "asked again for the same host"
    );
    finish(session).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_with_a_stored_certificate_is_never_downgraded_to_vnc_authentication() {
    let Some(password) = password() else {
        eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
        return;
    };
    if !reachable(PLAIN_PORT).await {
        eprintln!("SKIP: no VNC server on {HOST}:{PLAIN_PORT} (dev/servers.sh up)");
        return;
    }
    let dir = data_dir("downgrade");
    // The relay stands for an attacker that strips TLS from a host we know uses it: this
    // host:port has a pin, and the server behind it offers plain VNC Authentication only.
    let relay = Relay::start(PLAIN_PORT).await;
    let store = CertStore::in_data_dir(&dir);
    store
        .remember(
            HOST,
            relay.port,
            "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF",
        )
        .expect("pin");
    let mut config = VncConfig::new(HOST, relay.port);
    config.password = Some(secret(&password));
    config.cert_store = Some(store.clone());
    // Even an earlier agreement does not matter: the pin wins.
    let plain = PlainStore::in_data_dir(&dir);
    plain.allow(HOST, relay.port).expect("agree");
    config.plain_store = Some(plain);
    let (session, events) = start(config);
    events.wait("the error", |e| e.iter().any(is_error)).await;
    let all = events.all();
    let code = all.iter().find_map(|e| match e {
        FbEvent::Error(err) => Some((err.code, err.message.clone())),
        _ => None,
    });
    let (code, message) = code.expect("an error");
    assert_eq!(code, ErrorCode::SecurityDowngrade);
    assert!(message.contains("downgrade"), "{message}");
    assert!(!all
        .iter()
        .any(|e| is_unencrypted_prompt(e) || is_rect(e) || is_security(e)));
    assert!(relay.sent() <= 13, "the password response must not be sent");
    // The pin is still there.
    assert!(store.has_pin(HOST, relay.port).expect("pin"));
    finish(session).await;
}
