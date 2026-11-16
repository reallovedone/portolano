//! Integration tests against the TigerVNC test server (`dev/servers.sh up`, 127.0.0.1:15900,
//! VNC authentication). Every test skips with a message when the server is not reachable.
//!
//! The server has one shared desktop (an openbox with an xterm), so tests that type or click
//! run one at a time. Images written for inspection are the BACKEND image of the session,
//! under `.tmp/wp7/`; nothing here looks at the screen.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portolano_core_model::prompt::{CredentialAnswer, PromptHooks, PromptKind};
use portolano_core_model::ErrorCode;
use portolano_session_vnc::cert::PlainStore;
use portolano_session_vnc::{EventSink, FbEvent, Snapshot, VncAsk, VncConfig, VncSession};
use secrecy::SecretString;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 15900;

/// Tests that act on the shared desktop run one at a time.
static DESKTOP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Where the generated test credentials are. `PORTOLANO_TEST_KEYS_DIR` moves the folder (CI
/// simulation); `PORTOLANO_TEST_SERVERS_DOWN` makes every integration test skip as if the
/// servers and their credentials did not exist.
fn test_env_path() -> PathBuf {
    if std::env::var_os("PORTOLANO_TEST_SERVERS_DOWN").is_some() {
        return repo().join(".tmp/wp12/servers-down/test.env");
    }
    std::env::var_os("PORTOLANO_TEST_KEYS_DIR")
        .map_or_else(|| repo().join("dev/keys"), PathBuf::from)
        .join("test.env")
}

fn password() -> Option<String> {
    let text = std::fs::read_to_string(test_env_path()).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("VNC_PASSWORD="))
        .map(|v| v.trim().trim_matches('"').to_owned())
}

/// The password, or `None` (with a message) when the server cannot be used.
async fn server_password() -> Option<String> {
    let Some(password) = password() else {
        eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
        return None;
    };
    let reachable = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect((HOST, PORT)),
    )
    .await
    .is_ok_and(|r| r.is_ok());
    if !reachable {
        eprintln!("SKIP: no VNC server on {HOST}:{PORT} (dev/servers.sh up)");
        return None;
    }
    Some(password)
}

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
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

    fn count(&self, f: impl Fn(&FbEvent) -> bool) -> usize {
        self.all().iter().filter(|e| f(e)).count()
    }

    fn take(&self) -> Vec<FbEvent> {
        self.0
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default()
    }

    async fn wait(&self, what: &str, limit: Duration, f: impl Fn(&[FbEvent]) -> bool) {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if f(&self.all()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "timed out waiting for {what}; events: {:?}",
            summary(&self.all())
        );
    }
}

fn summary(events: &[FbEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            FbEvent::Resize { w, h } => format!("resize {w}x{h}"),
            FbEvent::Rect { x, y, w, h, .. } => format!("rect {x},{y} {w}x{h}"),
            FbEvent::Cursor { w, h, .. } => format!("cursor {w}x{h}"),
            FbEvent::CertPrompt { .. } => "cert".into(),
            FbEvent::Closed { reason } => format!("closed {reason}"),
            FbEvent::Error(e) => format!("error {}: {}", e.code.as_str(), e.message),
            FbEvent::CredentialPrompt(p) => format!("prompt {:?} err={:?}", p.kind, p.error),
            FbEvent::Security { encrypted } => format!("security encrypted={encrypted}"),
            FbEvent::UnencryptedPrompt => "unencrypted prompt".into(),
        })
        .collect()
}

fn is_rect(e: &FbEvent) -> bool {
    matches!(e, FbEvent::Rect { .. })
}

fn is_error(e: &FbEvent) -> bool {
    matches!(e, FbEvent::Error(_))
}

fn is_prompt(e: &FbEvent) -> bool {
    matches!(e, FbEvent::CredentialPrompt(_))
}

fn error_code(events: &[FbEvent]) -> Option<ErrorCode> {
    events.iter().find_map(|e| match e {
        FbEvent::Error(err) => Some(err.code),
        _ => None,
    })
}

/// The user agreed beforehand to use VNC Authentication without encryption on the test server
/// (the plain-VNC prompt is tested in `unencrypted.rs` and `tls.rs`).
fn agreed() -> PlainStore {
    let dir = repo().join(".tmp/secfix/vnc-server-tests");
    std::fs::create_dir_all(&dir).expect("create the data dir");
    let store = PlainStore::in_data_dir(&dir);
    store.allow(HOST, PORT).expect("record the agreement");
    store
}

fn config(password: &str) -> VncConfig {
    let mut c = VncConfig::new(HOST, PORT);
    c.password = Some(secret(password));
    c.plain_store = Some(agreed());
    c
}

fn start(config: VncConfig) -> (VncSession, Events) {
    let events = Events::default();
    let session = VncSession::start(config, events.sink());
    (session, events)
}

async fn connected(config: VncConfig) -> (VncSession, Events) {
    let (session, events) = start(config);
    events
        .wait("the first Rect", Duration::from_secs(15), |e| {
            e.iter().any(is_rect)
        })
        .await;
    (session, events)
}

/// The server blacklists an address after a few failed passwords (the tests make some on
/// purpose) and refuses it for a while; a successful login clears that. Waits until the
/// server accepts a good login again.
async fn ensure_not_blacklisted(password: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let (session, events) = start(config(password));
        events
            .wait("a login or a refusal", Duration::from_secs(15), |e| {
                e.iter().any(|e| is_rect(e) || is_error(e))
            })
            .await;
        let ok = events.all().iter().any(is_rect);
        session.close();
        session.closed().await;
        if ok {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the server keeps refusing: {:?}",
            summary(&events.all())
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn answer(password: &str) -> CredentialAnswer {
    CredentialAnswer {
        username: String::new(),
        password: Some(secret(password)),
        key_path: None,
        passphrase: None,
        domain: None,
    }
}

fn ask_config() -> VncConfig {
    let mut c = VncConfig::new(HOST, PORT);
    c.ask = Some(VncAsk {
        hooks: PromptHooks::no_vault(),
    });
    c.plain_store = Some(agreed());
    c
}

fn write_png(name: &str, s: &Snapshot) {
    let dir = repo().join(".tmp/wp7");
    std::fs::create_dir_all(&dir).expect("create .tmp/wp7");
    let file = std::fs::File::create(dir.join(name)).expect("create png");
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        u32::from(s.width),
        u32::from(s.height),
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(&s.rgba).expect("png data");
}

async fn snapshot(session: &VncSession) -> Snapshot {
    session.snapshot().await.expect("snapshot")
}

fn changed_pixels(a: &Snapshot, b: &Snapshot) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(p, q)| p != q)
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connects_authenticates_and_the_backend_image_is_the_desktop() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (session, events) = connected(config(&password)).await;
    // The security report (plain VNC: not encrypted) comes first, then the size of the
    // desktop, before any pixel.
    let all = events.all();
    assert_eq!(all.first(), Some(&FbEvent::Security { encrypted: false }));
    assert_eq!(all.get(1), Some(&FbEvent::Resize { w: 1280, h: 800 }));
    // Give the xterm a moment to appear, then look at the backend image.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let image = snapshot(&session).await;
    assert_eq!((image.width, image.height), (1280, 800));
    assert_eq!(image.rgba.len(), 1280 * 800 * 4);
    assert!(image.rgba.chunks_exact(4).all(|p| p[3] == 255));
    let mut colours = std::collections::HashSet::new();
    for p in image.rgba.chunks_exact(4) {
        colours.insert([p[0], p[1], p[2]]);
        if colours.len() > 8 {
            break;
        }
    }
    assert!(colours.len() > 2, "the desktop looks blank");
    write_png("backend-connect.png", &image);
    session.close();
    session.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_is_auth_failed_and_the_password_never_leaks() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let wrong = format!("{}x", &password[..password.len().min(7)]);
    assert_ne!(
        &wrong[..8.min(wrong.len())],
        &password[..8.min(password.len())]
    );
    let (_session, events) = start(config(&wrong));
    events
        .wait("an error", Duration::from_secs(15), |e| {
            e.iter().any(is_error)
        })
        .await;
    let all = events.all();
    assert_eq!(error_code(&all), Some(ErrorCode::AuthFailed));
    assert!(!all.iter().any(is_rect));
    let bytes: Vec<u8> = all.iter().flat_map(FbEvent::encode).collect();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains(&wrong) && !text.contains(&password));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_port_is_host_unreachable() {
    // Bind and drop to find a loopback port nobody listens on.
    let port = {
        let listener = std::net::TcpListener::bind((HOST, 0)).expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let mut c = VncConfig::new(HOST, port);
    c.password = Some(secret("whatever"));
    let (_session, events) = start(c);
    events
        .wait("an error", Duration::from_secs(15), |e| {
            e.iter().any(is_error)
        })
        .await;
    assert_eq!(error_code(&events.all()), Some(ErrorCode::HostUnreachable));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_prompt_flow_works_without_a_vault() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (session, events) = start(ask_config());
    events
        .wait("the prompt", Duration::from_secs(15), |e| {
            e.iter().any(is_prompt)
        })
        .await;
    // Nothing but the prompt before the answer, and it asks for a password only.
    let first = events.all();
    assert_eq!(first.len(), 1);
    let FbEvent::CredentialPrompt(prompt) = &first[0] else {
        panic!("expected a prompt");
    };
    assert_eq!(prompt.kind, PromptKind::Password);
    assert_eq!(prompt.username_hint, None);
    assert_eq!(prompt.target, format!("{HOST}:{PORT}"));
    assert_eq!(prompt.error, None);
    assert!(!prompt.can_save_to_vault);

    // A wrong password: the same session asks again, with the error.
    session
        .credential(answer("not-the-one"), false)
        .expect("wrong answer");
    events
        .wait("the second prompt", Duration::from_secs(15), |e| {
            e.iter().filter(|e| is_prompt(e)).count() == 2
        })
        .await;
    let FbEvent::CredentialPrompt(second) = &events.all()[1] else {
        panic!("expected a second prompt");
    };
    assert!(second.error.is_some());
    assert!(!events.all().iter().any(is_error));

    // The right one: the desktop appears.
    session
        .credential(answer(&password), false)
        .expect("answer");
    events
        .wait("the first Rect", Duration::from_secs(15), |e| {
            e.iter().any(is_rect)
        })
        .await;
    let bytes: Vec<u8> = events.all().iter().flat_map(FbEvent::encode).collect();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("not-the-one"));
    assert!(!text.contains(&password));
    session.close();
    session.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_the_prompt_ends_with_cancelled() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (session, events) = start(ask_config());
    events
        .wait("the prompt", Duration::from_secs(15), |e| {
            e.iter().any(is_prompt)
        })
        .await;
    session.credential_cancel().expect("cancel");
    events
        .wait("an error", Duration::from_secs(5), |e| {
            e.iter().any(is_error)
        })
        .await;
    assert_eq!(error_code(&events.all()), Some(ErrorCode::Cancelled));
    session.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_while_the_prompt_is_open_ends_quietly() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (session, events) = start(ask_config());
    events
        .wait("the prompt", Duration::from_secs(15), |e| {
            e.iter().any(is_prompt)
        })
        .await;
    session.close();
    tokio::time::timeout(Duration::from_secs(5), session.closed())
        .await
        .expect("the session ends");
    assert!(!events.all().iter().any(is_error));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_reaches_the_server_and_text_renders_like_server_output() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (session, events) = connected(config(&password)).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let initial = snapshot(&session).await;
    write_png("backend-before-typing.png", &initial);
    // Focus the xterm (it starts at +20+20) and type.
    session.pointer(300, 200, 0, 0).expect("move");
    session.pointer(300, 200, 1, 0).expect("press");
    session.pointer(300, 200, 0, 0).expect("release");
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Start from a clean terminal so earlier tests' output does not matter.
    type_text(
        &session,
        &["KeyC", "KeyL", "KeyE", "KeyA", "KeyR", "Enter"],
        Duration::from_millis(40),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let before = snapshot(&session).await;
    let _ = events.take();
    for (code, shift) in [
        ("KeyE", false),
        ("KeyC", false),
        ("KeyH", false),
        ("KeyO", false),
        ("Space", false),
        ("KeyH", true),
        ("KeyE", false),
        ("KeyL", false),
        ("KeyL", false),
        ("KeyO", false),
        ("Enter", false),
    ] {
        if shift {
            session.key("ShiftLeft", true).expect("shift");
        }
        session.key(code, true).expect("down");
        session.key(code, false).expect("up");
        if shift {
            session.key("ShiftLeft", false).expect("shift up");
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let after = snapshot(&session).await;
    write_png("backend-after-typing.png", &after);
    assert!(
        changed_pixels(&before, &after) > 50,
        "typing did not change the desktop"
    );
    // The typed word and the word the shell printed are rendered by the same font: find the
    // typed `Hello` (cells after the prompt) and the output `Hello` (cells 0..5 of the next
    // line) as identical, non-blank, newly drawn blocks. That proves the keysyms (including
    // Shift for the capital H) produced the right characters.
    let block = |img: &Snapshot, x: usize, y: usize| -> Vec<u8> {
        let mut v = Vec::new();
        for row in y..y + 13 {
            let from = (row * usize::from(img.width) + x) * 4;
            v.extend_from_slice(&img.rgba[from..from + 30 * 4]);
        }
        v
    };
    let mut found = false;
    for (ox, oy, col) in
        (20..25).flat_map(|ox| (36..200).flat_map(move |oy| (24..34).map(move |col| (ox, oy, col))))
    {
        {
            let typed = block(&after, ox + col * 6 + 2, oy);
            let printed = block(&after, ox + 2, oy + 13);
            let ink = typed.chunks_exact(4).filter(|p| p[0] < 128).count();
            if typed == printed && ink > 20 && typed != block(&before, ox + col * 6 + 2, oy) {
                found = true;
            }
        }
    }
    assert!(found, "the typed text does not match the printed text");
    // The server sent the changes as Rect events (flushed in bounded numbers).
    assert!(events.count(is_rect) > 0);
    session.close();
    session.closed().await;
}

async fn type_text(session: &VncSession, keys: &[&str], pause: Duration) {
    for code in keys {
        session.key(code, true).expect("down");
        session.key(code, false).expect("up");
        tokio::time::sleep(pause).await;
    }
}

async fn focus_xterm(session: &VncSession) {
    session.pointer(300, 200, 0, 0).expect("move");
    session.pointer(300, 200, 1, 0).expect("press");
    session.pointer(300, 200, 0, 0).expect("release");
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hidden_session_sends_nothing_and_showing_it_sends_one_full_rect_first() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (observer, events) = connected(config(&password)).await;
    let (typist, _typist_events) = connected(config(&password)).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    focus_xterm(&typist).await;
    let before = snapshot(&observer).await;

    observer.set_visible(false).expect("hide");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = events.take();
    type_text(
        &typist,
        &["KeyL", "KeyS", "Space", "Minus", "KeyL", "Enter"],
        Duration::from_millis(60),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(
        events.count(is_rect),
        0,
        "rects were sent while hidden: {:?}",
        summary(&events.all())
    );

    observer.set_visible(true).expect("show");
    events
        .wait("the redraw", Duration::from_secs(5), |e| {
            e.iter().any(is_rect)
        })
        .await;
    let first_rect = events.all().into_iter().find(is_rect);
    let Some(FbEvent::Rect { x, y, w, h, rgba }) = first_rect else {
        panic!("no rect");
    };
    assert_eq!((x, y, w, h), (0, 0, 1280, 800));
    assert_eq!(rgba.len(), 1280 * 800 * 4);
    // The non-incremental request brings the changes made while hidden.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let after = snapshot(&observer).await;
    assert!(
        changed_pixels(&before, &after) > 50,
        "the changes made while hidden did not arrive"
    );
    observer.close();
    typist.close();
    observer.closed().await;
    typist.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_of_server_changes_is_coalesced_into_bounded_rect_messages() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (observer, events) = connected(config(&password)).await;
    let (typist, _typist_events) = connected(config(&password)).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    focus_xterm(&typist).await;
    let _ = events.take();
    let started = Instant::now();
    // 90 characters in about 270 ms: far more server updates than flushes.
    for _ in 0..30 {
        type_text(&typist, &["KeyA", "KeyB", "KeyC"], Duration::from_millis(3)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let elapsed = started.elapsed();
    let rects = events.count(is_rect);
    let flushes =
        (elapsed.as_millis() / portolano_session_vnc::FLUSH_INTERVAL.as_millis()) as usize + 2;
    assert!(rects > 0);
    // At most one flush per interval, and each flush at most 24 rectangles.
    assert!(
        rects <= flushes * 24,
        "{rects} rects in {elapsed:?} exceeds the flush bound"
    );
    // Clean the line up for the next test.
    type_text(&typist, &["Enter"], Duration::from_millis(10)).await;
    observer.close();
    typist.close();
    observer.closed().await;
    typist.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn view_only_ignores_all_input() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let mut c = config(&password);
    c.view_only = true;
    let (session, _events) = connected(c).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let before = snapshot(&session).await;
    focus_xterm(&session).await;
    type_text(
        &session,
        &["KeyX", "KeyY", "KeyZ", "Enter"],
        Duration::from_millis(50),
    )
    .await;
    session.pointer(100, 100, 1, 120).expect("pointer");
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let after = snapshot(&session).await;
    assert_eq!(
        changed_pixels(&before, &after),
        0,
        "view-only input changed the desktop"
    );
    session.close();
    session.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_is_clean_and_later_calls_fail() {
    let Some(password) = server_password().await else {
        return;
    };
    let _desktop = DESKTOP.lock().await;
    ensure_not_blacklisted(&password).await;
    let (session, events) = connected(config(&password)).await;
    session.key("ShiftLeft", true).expect("shift down");
    session.pointer(5, 5, 1, 0).expect("press");
    session.close();
    tokio::time::timeout(Duration::from_secs(3), session.closed())
        .await
        .expect("the session ends promptly");
    // Closing from our side is not reported as an error or a close.
    assert!(!events
        .all()
        .iter()
        .any(|e| matches!(e, FbEvent::Error(_) | FbEvent::Closed { .. })));
    assert!(session.pointer(1, 1, 0, 0).is_err());
    assert!(session.snapshot().await.is_none());
}
