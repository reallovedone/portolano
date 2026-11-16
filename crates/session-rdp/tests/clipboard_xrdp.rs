//! Shared clipboard against the test xrdp container (127.0.0.1:13389). Skips, with a message,
//! when the server or `dev/keys/test.env` is missing. The local clipboard is a fake: nothing
//! here reads or writes the real system clipboard.
//!
//! The X side of xrdp is driven through `docker exec` (xterm and xdotool are in the image).
//! Those tests skip when Docker or the container is not available.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portolano_core_model::fb::ClipboardEvent;
use portolano_session_rdp::cert::CertStore;
use portolano_session_rdp::clipboard::{
    ClipboardConfig, LocalClipboard, LocalClipboardError, LocalContent,
};
use portolano_session_rdp::{FbEvent, RdpConfig, RdpCredentials, RdpSession};
use secrecy::SecretString;
use tokio::sync::mpsc;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 13389;
const CONTAINER: &str = "portolano-dev-xrdp";

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn test_password() -> Option<String> {
    let dir = std::env::var_os("PORTOLANO_TEST_KEYS_DIR")
        .map_or_else(|| repo_root().join("dev/keys"), PathBuf::from);
    if std::env::var_os("PORTOLANO_TEST_SERVERS_DOWN").is_some() {
        return None;
    }
    let text = std::fs::read_to_string(dir.join("test.env")).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("TEST_PASSWORD="))
        .map(|v| v.trim().trim_matches('"').to_owned())
}

async fn prerequisites() -> Option<String> {
    let password = test_password()?;
    let reachable = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect((HOST, PORT)),
    )
    .await
    .is_ok_and(|r| r.is_ok());
    if !reachable {
        eprintln!("SKIP: no RDP server on {HOST}:{PORT}");
        return None;
    }
    Some(password)
}

#[derive(Default)]
struct FakeState {
    text: Option<String>,
    token: u64,
    written: Vec<String>,
}

#[derive(Default)]
struct Fake(Mutex<FakeState>);

impl Fake {
    fn set_text(&self, text: &str) {
        let mut s = self.0.lock().expect("fake");
        s.text = Some(text.to_owned());
        s.token += 1;
    }
    fn written(&self) -> Vec<String> {
        self.0.lock().expect("fake").written.clone()
    }
}

impl LocalClipboard for Fake {
    fn change_token(&self) -> Option<u64> {
        Some(self.0.lock().expect("fake").token)
    }
    fn read(&self) -> Result<LocalContent, LocalClipboardError> {
        Ok(LocalContent {
            text: self.0.lock().expect("fake").text.clone(),
            ..LocalContent::default()
        })
    }
    fn write_text(&self, text: &str) -> Result<(), LocalClipboardError> {
        let mut s = self.0.lock().expect("fake");
        s.written.push(text.to_owned());
        s.text = Some(text.to_owned());
        s.token += 1;
        Ok(())
    }
}

struct Run {
    session: RdpSession,
    events: mpsc::UnboundedReceiver<FbEvent>,
}

async fn connect(password: &str, clipboard: Option<Arc<Fake>>) -> Run {
    let store = CertStore::in_data_dir(&repo_root().join(".tmp/wp14/data"));
    let mut c = RdpConfig::new(HOST, PORT, 1024, 768, store);
    c.credentials = Some(RdpCredentials {
        username: "tester".into(),
        password: SecretString::from(password.to_owned()),
    });
    c.allow_without_nla = true;
    c.clipboard = clipboard.map(|local| ClipboardConfig { local });
    let (tx, events) = mpsc::unbounded_channel();
    let sink: portolano_session_rdp::EventSink = Arc::new(move |e| tx.send(e).is_ok());
    let mut run = Run {
        session: RdpSession::start(c, sink),
        events,
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut accepted = false;
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("timed out waiting for the desktop");
        match tokio::time::timeout(left, run.events.recv()).await {
            Ok(Some(FbEvent::CertPrompt { .. })) if !accepted => {
                accepted = true;
                run.session.cert_decision(true).expect("decision");
            }
            Ok(Some(FbEvent::Resize { .. })) => return run,
            Ok(Some(FbEvent::Error(e))) => panic!("connection failed: {e}"),
            Ok(Some(FbEvent::Closed { reason })) => panic!("closed: {reason}"),
            Ok(Some(_)) => {}
            _ => panic!("the session ended early"),
        }
    }
}

impl Run {
    /// Events for `window`; the session must stay up.
    async fn watch(&mut self, window: Duration) -> Vec<ClipboardEvent> {
        let deadline = Instant::now() + window;
        let mut out = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match tokio::time::timeout(left, self.events.recv()).await {
                Ok(Some(FbEvent::Clipboard(c))) => out.push(c),
                Ok(Some(FbEvent::Error(e))) => panic!("session error: {e}"),
                Ok(Some(FbEvent::Closed { reason })) => panic!("closed: {reason}"),
                Ok(Some(_)) => {}
                Ok(None) => panic!("the session ended"),
                Err(_) => break,
            }
        }
        out
    }
}

fn docker_available() -> bool {
    Command::new("docker")
        .args(["exec", CONTAINER, "true"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs a shell command in the container as the desktop user, on its X display.
fn in_x(script: &str) -> Option<String> {
    let out = Command::new("docker")
        .args([
            "exec",
            "-u",
            "tester",
            "-e",
            "DISPLAY=:10",
            CONTAINER,
            "sh",
            "-c",
            script,
        ])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_option_the_channel_is_not_negotiated_and_the_desktop_works() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let mut run = connect(&password, None).await;
    // Nothing clipboard-related ever happens; the session stays up.
    assert!(run.watch(Duration::from_secs(3)).await.is_empty());
    run.session.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_the_option_the_session_stays_up_and_local_text_is_announced() {
    let Some(password) = prerequisites().await else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let fake = Arc::new(Fake::default());
    let mut run = connect(&password, Some(fake.clone())).await;
    fake.set_text("announced from portolano");
    assert!(run.watch(Duration::from_secs(4)).await.is_empty());
    run.session.close();
}

/// Text both ways with the real X clipboard owner of xrdp (`chansrv`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_goes_both_ways_through_xrdp() {
    let Some(password) = prerequisites().await else {
        return;
    };
    if !docker_available() {
        eprintln!("SKIP: no docker or no container {CONTAINER}");
        return;
    }
    let _serial = SERIAL.lock().await;
    let fake = Arc::new(Fake::default());
    let mut run = connect(&password, Some(fake.clone())).await;
    run.watch(Duration::from_secs(4)).await;

    // Remote to local: xterm puts text on the CLIPBOARD selection (OSC 52).
    let _ = in_x(
        "pkill -u tester xterm; (xterm -xrm 'XTerm*allowWindowOps: true' -xrm 'XTerm*disallowedWindowOps: 20,21,SetXprop' -e sh -c \"printf '\\033]52;c;%s\\007' \\\"\\$(printf 'from the remote' | base64)\\\"; sleep 20\" >/dev/null 2>&1 &) ; sleep 1; echo started",
    );
    run.watch(Duration::from_secs(6)).await;
    // Whatever the desktop held when we connected may have arrived first.
    let written = fake.written();
    assert_eq!(
        written.last().map(String::as_str),
        Some("from the remote"),
        "remote text reached the local clipboard"
    );
    let writes_before = written.len();

    // Local to remote: after the local copy, the X clipboard (owned by chansrv) holds it.
    fake.set_text("from the local side");
    run.watch(Duration::from_secs(4)).await;
    let got = in_x(
        "pkill -u tester xterm; rm -f /tmp/pasted.txt; (xterm -xrm 'XTerm*vt100.translations: #override Ctrl Shift <Key>V: insert-selection(CLIPBOARD)' -e sh -c 'cat > /tmp/pasted.txt' >/dev/null 2>&1 &); sleep 2; xdotool key --clearmodifiers ctrl+shift+v; sleep 2; xdotool key --clearmodifiers ctrl+d; sleep 1; cat /tmp/pasted.txt",
    )
    .unwrap_or_default();
    assert_eq!(got.trim_end(), "from the local side");
    let _ = in_x("pkill -u tester xterm; rm -f /tmp/pasted.txt");
    // No loop: announcing our text did not make the desktop send anything back.
    assert_eq!(fake.written().len(), writes_before);
    run.session.close();
}
