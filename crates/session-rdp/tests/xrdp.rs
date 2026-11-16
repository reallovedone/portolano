//! Headless integration tests against the test xrdp container (127.0.0.1:13389, TLS without
//! NLA). They skip, with a message, when the server or `dev/keys/test.env` is missing.
//!
//! The server keeps one desktop per user and a new login takes the session over, so the tests
//! that log in run one at a time (`SERIAL`). Nothing is drawn anywhere: the backend image is
//! read through `RdpSession::snapshot` and PNG copies of it are written to `.tmp/wp6/`.
//!
//! NLA/CredSSP cannot be exercised here (xrdp does not offer it), so every connection that
//! sends a credential sets `allow_without_nla` (the explicit per-connection choice). The
//! default (NLA only) is covered by `tls_verification.rs` and by
//! `without_the_option_a_server_without_nla_is_refused_and_no_password_is_sent` below.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use portolano_core_model::prompt::{CredentialAnswer, PromptHooks, PromptKind};
use portolano_core_model::ErrorCode;
use portolano_session_rdp::cert::{CertCheck, CertStore};
use portolano_session_rdp::{
    FbEvent, RdpAsk, RdpConfig, RdpCredentials, RdpSession, Snapshot, FLUSH_INTERVAL,
};
use secrecy::SecretString;
use tokio::sync::mpsc;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 13389;
const USER: &str = "tester";
const WIDTH: u16 = 1280;
const HEIGHT: u16 = 720;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Where the generated test credentials are. `PORTOLANO_TEST_KEYS_DIR` moves the folder (CI
/// simulation); `PORTOLANO_TEST_SERVERS_DOWN` makes every integration test skip as if the
/// servers and their credentials did not exist.
fn test_env_path() -> PathBuf {
    if std::env::var_os("PORTOLANO_TEST_SERVERS_DOWN").is_some() {
        return repo_root().join(".tmp/wp12/servers-down/test.env");
    }
    std::env::var_os("PORTOLANO_TEST_KEYS_DIR")
        .map_or_else(|| repo_root().join("dev/keys"), PathBuf::from)
        .join("test.env")
}

fn out_dir(name: &str) -> PathBuf {
    let dir = repo_root().join(".tmp/wp6").join(name);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// The test password, read at run time from `dev/keys/test.env`.
fn test_password() -> Option<String> {
    let text = fs::read_to_string(test_env_path()).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("TEST_PASSWORD="))
        .map(|v| v.trim().trim_matches('"').to_owned())
}

/// `Some(password)` when the server can be tested, otherwise prints why not.
async fn prerequisites() -> Option<String> {
    let Some(password) = test_password() else {
        eprintln!("SKIP: dev/keys/test.env has no TEST_PASSWORD");
        return None;
    };
    let reachable = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect((HOST, PORT)),
    )
    .await
    .is_ok_and(|r| r.is_ok());
    if !reachable {
        eprintln!("SKIP: no RDP server on {HOST}:{PORT} (start it with `bash dev/servers.sh up`)");
        return None;
    }
    Some(password)
}

struct Harness {
    session: RdpSession,
    events: mpsc::UnboundedReceiver<(Instant, FbEvent)>,
}

fn config(store: &CertStore, password: Option<&str>) -> RdpConfig {
    let mut c = RdpConfig::new(HOST, PORT, WIDTH, HEIGHT, store.clone());
    c.credentials = password.map(|p| RdpCredentials {
        username: USER.into(),
        password: SecretString::from(p.to_owned()),
    });
    // xrdp offers TLS-level security only.
    c.allow_without_nla = true;
    c
}

fn start(config: RdpConfig) -> Harness {
    let (tx, events) = mpsc::unbounded_channel();
    let sink: portolano_session_rdp::EventSink =
        Arc::new(move |event| tx.send((Instant::now(), event)).is_ok());
    Harness {
        session: RdpSession::start(config, sink),
        events,
    }
}

impl Harness {
    async fn next(&mut self, limit: Duration) -> Option<(Instant, FbEvent)> {
        tokio::time::timeout(limit, self.events.recv())
            .await
            .ok()
            .flatten()
    }

    /// Waits for the first event matching `pick`, returning everything seen before it.
    async fn until<T>(
        &mut self,
        limit: Duration,
        mut pick: impl FnMut(&FbEvent) -> Option<T>,
    ) -> Result<(T, Vec<FbEvent>), Vec<FbEvent>> {
        let deadline = Instant::now() + limit;
        let mut seen = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some((_, event)) = self.next(left).await else {
                break;
            };
            if let Some(found) = pick(&event) {
                return Ok((found, seen));
            }
            seen.push(event);
        }
        Err(seen)
    }

    /// Reads until no event arrived for `quiet`, or `limit` passed. Returns all events.
    async fn drain(&mut self, quiet: Duration, limit: Duration) -> Vec<(Instant, FbEvent)> {
        let deadline = Instant::now() + limit;
        let mut all = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.next(quiet.min(left)).await {
                Some(item) => all.push(item),
                None => break,
            }
        }
        all
    }

    /// Connects, accepts any certificate prompt and waits for the first `Resize`.
    async fn connect_and_wait_desktop(&mut self) -> (u16, u16) {
        let mut accepted = false;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("timed out waiting for the desktop");
            let (_, event) = self.next(left).await.expect("session ended early");
            match event {
                FbEvent::CertPrompt { .. } if !accepted => {
                    accepted = true;
                    self.session.cert_decision(true).expect("decision");
                }
                FbEvent::Resize { w, h } => return (w, h),
                FbEvent::Error(e) => panic!("connection failed: {e}"),
                FbEvent::Closed { reason } => panic!("closed: {reason}"),
                _ => {}
            }
        }
    }

    /// Waits until the desktop has painted and then stopped changing for `quiet`.
    async fn settle(&mut self, quiet: Duration) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut rect_events = 0usize;
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the desktop never settled");
            match self.next(quiet.min(left)).await {
                Some((_, FbEvent::Rect { .. })) => rect_events += 1,
                Some((_, FbEvent::Error(e))) => panic!("session error: {e}"),
                Some((_, FbEvent::Closed { reason })) => panic!("closed: {reason}"),
                Some(_) => {}
                None => {
                    let snap = self.session.snapshot().await.expect("snapshot");
                    if rect_events > 0 && !is_blank(&snap) {
                        return snap;
                    }
                }
            }
        }
    }
}

fn is_blank(s: &Snapshot) -> bool {
    distinct_colors(s) < 8
}

fn distinct_colors(s: &Snapshot) -> usize {
    let mut colors = HashSet::new();
    for px in s.rgba.chunks_exact(4).step_by(7) {
        colors.insert((px[0], px[1], px[2]));
        if colors.len() > 4096 {
            break;
        }
    }
    colors.len()
}

fn non_black_ratio(s: &Snapshot) -> f64 {
    let lit = s
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count();
    lit as f64 / (usize::from(s.width) * usize::from(s.height)) as f64
}

/// Number of pixels whose RGB differs by more than a small threshold.
fn changed_pixels(a: &Snapshot, b: &Snapshot) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(p, q)| p[..3].iter().zip(&q[..3]).any(|(x, y)| x.abs_diff(*y) > 24))
        .count()
}

fn write_png(path: &std::path::Path, s: &Snapshot) {
    let file = fs::File::create(path).expect("create png");
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

fn fresh_store(name: &str) -> CertStore {
    let dir = out_dir(name);
    let store = CertStore::in_data_dir(&dir);
    // Start from an empty store.
    if let Err(e) = fs::remove_file(store.path()) {
        assert_eq!(
            e.kind(),
            std::io::ErrorKind::NotFound,
            "cannot reset the store"
        );
    }
    store
}

async fn close_cleanly(h: &Harness) {
    h.session.close();
    tokio::time::timeout(Duration::from_secs(5), h.session.closed())
        .await
        .expect("the session task did not end within 5 s of close()");
}

// ---------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_renders_a_plausible_desktop_and_closes_cleanly() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("login");
    let mut h = start(config(&store, Some(&password)));
    let (w, hgt) = h.connect_and_wait_desktop().await;
    assert_eq!(
        (w, hgt),
        (WIDTH, HEIGHT),
        "server granted another desktop size"
    );

    let snap = h.settle(Duration::from_secs(3)).await;
    assert_eq!((snap.width, snap.height), (WIDTH, HEIGHT));
    assert_eq!(
        snap.rgba.len(),
        usize::from(WIDTH) * usize::from(HEIGHT) * 4
    );
    let colors = distinct_colors(&snap);
    let lit = non_black_ratio(&snap);
    eprintln!(
        "desktop: {colors}+ distinct colours, {:.1}% non-black",
        lit * 100.0
    );
    assert!(colors >= 8, "the image looks blank");
    assert!(lit > 0.2, "the image is mostly black");
    write_png(&out_dir("login").join("backend-image.png"), &snap);

    close_cleanly(&h).await;
    // Closing from our side reports nothing further.
    let rest = h
        .drain(Duration::from_millis(200), Duration::from_secs(1))
        .await;
    assert!(!rest
        .iter()
        .any(|(_, e)| matches!(e, FbEvent::Error(_) | FbEvent::Closed { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn certificate_trust_on_first_use() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("tofu");

    // 1. First connection: unknown certificate -> prompt; accepting stores it.
    let mut h = start(config(&store, Some(&password)));
    let (fingerprint, _) = h
        .until(Duration::from_secs(30), |e| match e {
            FbEvent::CertPrompt {
                fingerprint,
                changed,
                ..
            } => {
                assert!(!changed, "first contact must not be flagged as changed");
                Some(fingerprint.clone())
            }
            _ => None,
        })
        .await
        .expect("no certificate prompt");
    assert_eq!(
        fingerprint.split(':').count(),
        32,
        "SHA-256 with 32 hex pairs"
    );
    assert_eq!(
        store.check(HOST, PORT, &fingerprint).expect("check"),
        CertCheck::Unknown
    );
    h.session.cert_decision(true).expect("decision");
    h.until(Duration::from_secs(60), |e| {
        matches!(e, FbEvent::Resize { .. }).then_some(())
    })
    .await
    .expect("no desktop after accepting");
    assert_eq!(
        store.check(HOST, PORT, &fingerprint).expect("check"),
        CertCheck::Matches
    );
    close_cleanly(&h).await;

    // 2. Second connection: stored and matching -> no prompt before the desktop.
    let mut h = start(config(&store, Some(&password)));
    let (_, before) = h
        .until(Duration::from_secs(60), |e| {
            matches!(e, FbEvent::Resize { .. }).then_some(())
        })
        .await
        .expect("no desktop on the second connection");
    assert!(
        !before
            .iter()
            .any(|e| matches!(e, FbEvent::CertPrompt { .. })),
        "a known certificate must not prompt"
    );
    close_cleanly(&h).await;

    // 3. Tampered stored fingerprint -> the attempt ends with the changed-certificate report
    //    (both fingerprints) and nothing to decide; the stored value stays.
    let tampered = "00:".repeat(31) + "00";
    store.remember(HOST, PORT, &tampered).expect("tamper");
    let mut h = start(config(&store, Some(&password)));
    let (seen, _) = h
        .until(Duration::from_secs(30), |e| match e {
            FbEvent::CertPrompt {
                fingerprint: f,
                changed,
                stored,
            } => Some((f.clone(), *changed, stored.clone())),
            _ => None,
        })
        .await
        .expect("no report for a changed certificate");
    assert_eq!(seen.0, fingerprint);
    assert!(
        seen.1,
        "a different stored fingerprint must be reported as changed"
    );
    assert_eq!(seen.2.as_deref(), Some(tampered.as_str()));
    tokio::time::timeout(Duration::from_secs(5), h.session.closed())
        .await
        .expect("a changed certificate ends the attempt by itself");
    let rest = h
        .drain(Duration::from_millis(300), Duration::from_secs(2))
        .await;
    assert!(
        rest.iter().all(|(_, e)| !matches!(
            e,
            FbEvent::Resize { .. } | FbEvent::Error(_) | FbEvent::Closed { .. }
        )),
        "nothing follows the report"
    );
    assert_eq!(
        store.check(HOST, PORT, &fingerprint).expect("check"),
        CertCheck::Changed {
            stored: tampered.clone()
        },
        "the stored fingerprint must stay"
    );

    // 4. Only removing the stored certificate on purpose lets the host be trusted again.
    assert!(store.forget(HOST, PORT).expect("forget"));
    let mut h = start(config(&store, Some(&password)));
    let (changed, _) = h
        .until(Duration::from_secs(30), |e| match e {
            FbEvent::CertPrompt { changed, .. } => Some(*changed),
            _ => None,
        })
        .await
        .expect("no prompt");
    assert!(!changed, "after removing the pin the certificate is new");
    h.session.cert_decision(true).expect("decision");
    h.until(Duration::from_secs(60), |e| {
        matches!(e, FbEvent::Resize { .. }).then_some(())
    })
    .await
    .expect("no desktop after trusting the certificate again");
    assert_eq!(
        store.check(HOST, PORT, &fingerprint).expect("check"),
        CertCheck::Matches
    );
    close_cleanly(&h).await;

    // 5. ignoreCert: no prompt and nothing stored.
    let store2 = fresh_store("tofu-ignore");
    let mut cfg = config(&store2, Some(&password));
    cfg.ignore_cert = true;
    let mut h = start(cfg);
    let (_, before) = h
        .until(Duration::from_secs(60), |e| {
            matches!(e, FbEvent::Resize { .. }).then_some(())
        })
        .await
        .expect("no desktop with ignoreCert");
    assert!(!before
        .iter()
        .any(|e| matches!(e, FbEvent::CertPrompt { .. })));
    assert_eq!(
        store2.check(HOST, PORT, &fingerprint).expect("check"),
        CertCheck::Unknown
    );
    close_cleanly(&h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_password_is_reported() {
    let Some(_) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("wrongpw");
    let mut cfg = config(&store, Some("definitely-not-the-password"));
    cfg.ignore_cert = true;
    let mut h = start(cfg);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut outcome = String::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match h.next(left).await {
            Some((_, FbEvent::Error(e))) => {
                outcome = format!("error {}: {}", e.code.as_str(), e.message);
                assert_eq!(e.code, ErrorCode::AuthFailed, "{outcome}");
                break;
            }
            Some((_, FbEvent::Closed { reason })) => {
                outcome = format!("closed: {reason}");
                break;
            }
            Some((_, FbEvent::Resize { .. })) => {
                // xrdp lets the RDP connection up and shows its own login screen.
                outcome = "connected (xrdp login screen)".into();
                break;
            }
            Some(_) => {}
            None => break,
        }
    }
    eprintln!("wrong password outcome: {outcome}");
    assert!(!outcome.is_empty(), "no outcome within 60 s");
    close_cleanly(&h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_port_is_host_unreachable() {
    let store = fresh_store("unreachable");
    // Port 1 on loopback: nothing listens there.
    let mut cfg = RdpConfig::new(HOST, 1, 800, 600, store);
    cfg.ignore_cert = true;
    let mut h = start(cfg);
    let (code, _) = h
        .until(Duration::from_secs(15), |e| match e {
            FbEvent::Error(err) => Some(err.code),
            _ => None,
        })
        .await
        .expect("no error event");
    assert_eq!(code, ErrorCode::HostUnreachable);
    tokio::time::timeout(Duration::from_secs(2), h.session.closed())
        .await
        .expect("task did not finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_during_certificate_prompt_ends_the_session() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("close-prompt");
    let mut h = start(config(&store, Some(&password)));
    h.until(Duration::from_secs(30), |e| {
        matches!(e, FbEvent::CertPrompt { .. }).then_some(())
    })
    .await
    .expect("no prompt");
    close_cleanly(&h).await;
}

/// Pointer, keyboard, visibility and coalescing share one login: starting the desktop takes
/// tens of seconds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_visibility_and_coalescing() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("interactive");
    let mut h = start(config(&store, Some(&password)));
    h.connect_and_wait_desktop().await;
    let idle = h.settle(Duration::from_secs(4)).await;
    let dir = out_dir("interactive");
    write_png(&dir.join("01-idle.png"), &idle);

    // ---- Input: a right click on the desktop opens a context menu, Escape closes it. ----
    let (cx, cy) = (WIDTH / 2, HEIGHT / 2);
    h.session.pointer(cx, cy, 0, 0).expect("move");
    h.session.pointer(cx, cy, 2, 0).expect("press");
    h.session.pointer(cx, cy, 0, 0).expect("release");
    let _ = h
        .drain(Duration::from_millis(1500), Duration::from_secs(8))
        .await;
    let with_menu = h.session.snapshot().await.expect("snapshot");
    write_png(&dir.join("02-context-menu.png"), &with_menu);
    let menu_pixels = changed_pixels(&idle, &with_menu);
    eprintln!("context menu changed {menu_pixels} pixels");
    assert!(
        menu_pixels > 2_000,
        "pointer input did not reach the server ({menu_pixels} px changed)"
    );

    // ---- Coalescing: hover sweep over the menu produces a steady stream of tiny updates. ----
    let sweep_start = Instant::now();
    for step in 0..150u16 {
        h.session
            .pointer(cx + 20 + (step % 40), cy + 6 + (step % 120), 0, 0)
            .expect("sweep");
        tokio::time::sleep(Duration::from_millis(8)).await;
    }
    let sweep = h
        .drain(Duration::from_millis(800), Duration::from_secs(5))
        .await;
    let rect_times: Vec<Instant> = sweep
        .iter()
        .filter(|(_, e)| matches!(e, FbEvent::Rect { .. }))
        .map(|(t, _)| *t)
        .collect();
    let elapsed = sweep_start.elapsed().as_secs_f64();
    // Group messages that belong to one flush (they are sent back to back).
    let mut flushes: Vec<Instant> = Vec::new();
    for t in &rect_times {
        match flushes.last() {
            Some(last) if t.duration_since(*last) < Duration::from_millis(4) => {}
            _ => flushes.push(*t),
        }
    }
    let min_gap = flushes
        .windows(2)
        .map(|w| w[1].duration_since(w[0]))
        .min()
        .unwrap_or(Duration::MAX);
    eprintln!(
        "coalescing: {} rect messages in {} flushes over {:.1}s (min gap between flushes {:?}, interval {:?})",
        rect_times.len(),
        flushes.len(),
        elapsed,
        min_gap,
        FLUSH_INTERVAL
    );
    assert!(
        !rect_times.is_empty(),
        "the hover sweep produced no updates"
    );
    let rate = flushes.len() as f64 / elapsed;
    assert!(
        rate <= 45.0,
        "flush rate {rate:.1} Hz exceeds the 40 Hz bound"
    );
    assert!(
        min_gap >= FLUSH_INTERVAL - Duration::from_millis(8),
        "two flushes only {min_gap:?} apart"
    );

    // Escape (keyboard) closes the menu again.
    h.session.key("Escape", true).expect("key down");
    h.session.key("Escape", false).expect("key up");
    let _ = h
        .drain(Duration::from_millis(1500), Duration::from_secs(8))
        .await;
    let after_escape = h.session.snapshot().await.expect("snapshot");
    write_png(&dir.join("03-after-escape.png"), &after_escape);
    let remaining = changed_pixels(&idle, &after_escape);
    eprintln!("after Escape {remaining} pixels still differ from the idle desktop");
    assert!(
        remaining < menu_pixels / 2,
        "the Escape key did not reach the server ({remaining} vs {menu_pixels} px)"
    );

    // ---- Visibility ----
    h.session.set_visible(false).expect("hide");
    let _ = h
        .drain(Duration::from_millis(300), Duration::from_secs(2))
        .await;
    // Activity while hidden: the backend keeps working but sends nothing.
    h.session.pointer(cx, cy, 0, 0).expect("move");
    h.session.pointer(cx, cy, 2, 0).expect("press");
    h.session.pointer(cx, cy, 0, 0).expect("release");
    let hidden_events = h
        .drain(Duration::from_secs(3), Duration::from_secs(4))
        .await;
    let hidden_rects = hidden_events
        .iter()
        .filter(|(_, e)| matches!(e, FbEvent::Rect { .. }))
        .count();
    assert_eq!(hidden_rects, 0, "frames were sent while hidden");

    h.session.set_visible(true).expect("show");
    let (full, _) = h
        .until(Duration::from_secs(5), |e| match e {
            FbEvent::Rect { x, y, w, h, rgba } if *x == 0 && *y == 0 => Some((*w, *h, rgba.len())),
            _ => None,
        })
        .await
        .expect("no full redraw after showing");
    assert_eq!(
        (full.0, full.1),
        (WIDTH, HEIGHT),
        "the redraw must cover the whole desktop"
    );
    assert_eq!(full.2, usize::from(WIDTH) * usize::from(HEIGHT) * 4);
    // Close the menu opened while hidden and make sure updates flow again.
    h.session.key("Escape", true).expect("key down");
    h.session.key("Escape", false).expect("key up");
    let _ = h
        .drain(Duration::from_millis(1500), Duration::from_secs(6))
        .await;
    let end = h.session.snapshot().await.expect("snapshot");
    write_png(&dir.join("04-after-visibility.png"), &end);
    assert!(!is_blank(&end));

    close_cleanly(&h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cursor_shapes_are_forwarded() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("cursor");
    let mut cfg = config(&store, Some(&password));
    cfg.ignore_cert = true;
    let mut h = start(cfg);
    h.connect_and_wait_desktop().await;
    // Wiggle the pointer so the server sends its shapes.
    let mut cursors: Vec<(u16, u16, usize)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut step = 0u16;
    while Instant::now() < deadline && cursors.is_empty() {
        h.session
            .pointer(100 + step % 500, 100 + step % 300, 0, 0)
            .expect("move");
        step = step.wrapping_add(37);
        let events = h
            .drain(Duration::from_millis(300), Duration::from_millis(500))
            .await;
        for (_, e) in events {
            if let FbEvent::Cursor { w, h, rgba, .. } = e {
                assert_eq!(rgba.len(), usize::from(w) * usize::from(h) * 4);
                cursors.push((w, h, rgba.len()));
            }
        }
    }
    eprintln!("cursor events seen: {cursors:?}");
    assert!(!cursors.is_empty(), "no cursor event within 40 s");
    close_cleanly(&h).await;
}

/// Display Control: asking for another size makes the server reactivate the session with
/// the new desktop size (or, when xrdp does not support it, nothing happens).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dynamic_resize_or_graceful_fallback() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("resize");
    let mut cfg = config(&store, Some(&password));
    cfg.ignore_cert = true;
    let mut h = start(cfg);
    h.connect_and_wait_desktop().await;
    let _ = h.settle(Duration::from_secs(3)).await;
    h.session.resize(1024, 600).expect("resize");
    let resized = h
        .until(Duration::from_secs(15), |e| match e {
            FbEvent::Resize { w, h } => Some((*w, *h)),
            _ => None,
        })
        .await;
    match resized {
        Ok(((w, hgt), _)) => {
            eprintln!("dynamic resize: server granted {w}x{hgt}");
            assert_eq!((w, hgt), (1024, 600));
            let _ = h
                .drain(Duration::from_secs(2), Duration::from_secs(10))
                .await;
            let snap = h.session.snapshot().await.expect("snapshot");
            assert_eq!((snap.width, snap.height), (1024, 600));
            write_png(&out_dir("resize").join("resized.png"), &snap);
        }
        Err(_) => {
            eprintln!("dynamic resize: not performed by this server (no Resize event)");
            let snap = h.session.snapshot().await.expect("session still alive");
            assert_eq!((snap.width, snap.height), (WIDTH, HEIGHT));
        }
    }
    close_cleanly(&h).await;
}

// ---- credentials on demand (WP11): no vault, no stored credential ----

/// A configuration with no credential that asks the user, `hint` pre-filling the user name.
fn asking_config(store: &CertStore, hint: Option<&str>) -> RdpConfig {
    let mut c = RdpConfig::new(HOST, PORT, WIDTH, HEIGHT, store.clone());
    c.ask = Some(RdpAsk {
        username_hint: hint.map(str::to_owned),
        hooks: PromptHooks::no_vault(),
    });
    c.allow_without_nla = true;
    c
}

fn typed(user: &str, password: &str) -> CredentialAnswer {
    CredentialAnswer {
        username: user.into(),
        password: Some(SecretString::from(password.to_owned())),
        key_path: None,
        passphrase: None,
        domain: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_credential_the_session_prompts_and_then_connects() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("ask-connect");
    let mut h = start(asking_config(&store, Some(USER)));

    // The prompt comes first: nothing was sent to the server yet.
    let (prompt, before) = h
        .until(Duration::from_secs(20), |e| match e {
            FbEvent::CredentialPrompt(p) => Some(p.clone()),
            _ => None,
        })
        .await
        .expect("no credential prompt");
    assert!(before.is_empty(), "events before the prompt: {before:?}");
    assert_eq!(prompt.kind, PromptKind::UsernamePassword);
    assert_eq!(prompt.username_hint.as_deref(), Some(USER));
    assert_eq!(prompt.target, format!("{HOST}:{PORT}"));
    assert_eq!(prompt.error, None);
    assert!(!prompt.can_save_to_vault);

    h.session
        .credential(typed(USER, &password), false)
        .expect("answer");
    // Certificate prompt (fresh store), then the desktop.
    let (w, hgt) = h.connect_and_wait_desktop().await;
    assert_eq!((w, hgt), (WIDTH, HEIGHT));
    let (_, seen) = h
        .until(Duration::from_secs(60), |e| {
            matches!(e, FbEvent::Rect { .. }).then_some(())
        })
        .await
        .expect("no frames arrived");
    // Frames arrived, and the secret is in none of the non-pixel bytes sent to the UI.
    for event in seen
        .iter()
        .filter(|e| !matches!(e, FbEvent::Rect { .. } | FbEvent::Cursor { .. }))
    {
        let bytes = event.encode();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(&password),
            "secret in {event:?}"
        );
    }
    close_cleanly(&h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_the_prompt_ends_with_a_cancelled_error_and_nothing_else() {
    let store = fresh_store("ask-cancel");
    let reachable = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect((HOST, PORT)),
    )
    .await
    .is_ok_and(|r| r.is_ok());
    if !reachable {
        eprintln!("SKIP: no RDP server on {HOST}:{PORT}");
        return;
    }
    let mut h = start(asking_config(&store, None));
    let (prompt, _) = h
        .until(Duration::from_secs(20), |e| match e {
            FbEvent::CredentialPrompt(p) => Some(p.clone()),
            _ => None,
        })
        .await
        .expect("no credential prompt");
    assert_eq!(prompt.username_hint, None);
    h.session.credential_cancel().expect("cancel");
    let (code, _) = h
        .until(Duration::from_secs(10), |e| match e {
            FbEvent::Error(err) => Some(err.code),
            _ => None,
        })
        .await
        .expect("no error event");
    assert_eq!(code, ErrorCode::Cancelled);
    tokio::time::timeout(Duration::from_secs(2), h.session.closed())
        .await
        .expect("task did not finish");
    // No certificate prompt, no connection attempt after the cancellation.
    let rest = h
        .drain(Duration::from_millis(200), Duration::from_secs(1))
        .await;
    assert!(rest.is_empty(), "{rest:?}");
    // A late answer has nothing to answer.
    assert!(h.session.credential_cancel().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_invalid_answer_is_asked_again_before_any_connection() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("ask-invalid");
    let mut h = start(asking_config(&store, None));
    let pick = |e: &FbEvent| match e {
        FbEvent::CredentialPrompt(p) => Some(p.clone()),
        _ => None,
    };
    h.until(Duration::from_secs(20), pick)
        .await
        .expect("prompt");
    h.session
        .credential(typed("", &password), false)
        .expect("answer");
    let (again, _) = h
        .until(Duration::from_secs(10), pick)
        .await
        .expect("second prompt");
    assert_eq!(again.error.as_deref(), Some("Enter a user name."));
    close_cleanly(&h).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_option_a_server_without_nla_is_refused_and_no_password_is_sent() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let store = fresh_store("no-nla");
    let mut cfg = config(&store, Some(&password));
    cfg.allow_without_nla = false;
    let mut h = start(cfg);
    let (message, seen) = h
        .until(Duration::from_secs(30), |e| match e {
            FbEvent::Error(err) => Some(err.message.clone()),
            _ => None,
        })
        .await
        .expect("no error");
    assert!(
        message.contains("Allow connections without NLA"),
        "the error must name the option: {message}"
    );
    assert!(
        !seen.iter().any(|e| matches!(e, FbEvent::CertPrompt { .. })),
        "nothing may be asked: the credential never travels"
    );
    assert!(!message.contains(&password));
}
