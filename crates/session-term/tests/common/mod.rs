//! Shared helpers of the integration tests: a frame-decoding sink, polling waits, the test
//! server's credentials (read from `dev/keys` at run time) and scratch directories.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portolano_core_model::prompt::CredentialPrompt;
use portolano_core_model::AppError;
use portolano_session_term::{
    FrameSink, Session, SessionSpec, SshAuth, SshHop, SshSpec, TAG_CREDENTIAL_PROMPT, TAG_DATA,
    TAG_ERROR, TAG_EXIT, TAG_HOST_KEY_PROMPT,
};
use secrecy::SecretString;

/// The test sshd drops unauthenticated connections beyond `MaxStartups` (10 by default, random
/// early drop above): tests that connect run one at a time.
pub async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

pub const SERVER_HOST: &str = "127.0.0.1";
pub const SERVER_PORT: u16 = 12222;
pub const SERVER_USER: &str = "tester";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    HostKeyPrompt {
        fingerprint: String,
        algorithm: String,
        host: String,
        port: u16,
    },
    Exit(Option<i32>),
    Error(AppError),
    CredentialPrompt(CredentialPrompt),
}

/// Everything the session pushed into the sink.
#[derive(Default)]
pub struct Probe {
    data: Mutex<Vec<u8>>,
    events: Mutex<Vec<Event>>,
    acked: Mutex<usize>,
    frames: Mutex<usize>,
    max_frame: Mutex<usize>,
    /// Every frame exactly as it was handed to the UI channel.
    raw: Mutex<Vec<Vec<u8>>>,
}

impl Probe {
    pub fn sink(self: &Arc<Self>) -> FrameSink {
        let probe = self.clone();
        Arc::new(move |frame: Vec<u8>| {
            probe.push(&frame);
            true
        })
    }

    fn push(&self, frame: &[u8]) {
        if let Ok(mut raw) = self.raw.lock() {
            raw.push(frame.to_vec());
        }
        let body = &frame[1..];
        match frame[0] {
            TAG_DATA => {
                if let Ok(mut d) = self.data.lock() {
                    d.extend_from_slice(body);
                }
                if let Ok(mut n) = self.frames.lock() {
                    *n += 1;
                }
                if let Ok(mut m) = self.max_frame.lock() {
                    *m = (*m).max(body.len());
                }
            }
            TAG_HOST_KEY_PROMPT => {
                let j: serde_json::Value = serde_json::from_slice(body).expect("prompt json");
                self.event(Event::HostKeyPrompt {
                    fingerprint: j["fingerprint"].as_str().unwrap_or_default().into(),
                    algorithm: j["algorithm"].as_str().unwrap_or_default().into(),
                    host: j["host"].as_str().unwrap_or_default().into(),
                    port: j["port"]
                        .as_u64()
                        .and_then(|p| u16::try_from(p).ok())
                        .unwrap_or_default(),
                });
            }
            TAG_EXIT => {
                let j: serde_json::Value = serde_json::from_slice(body).expect("exit json");
                self.event(Event::Exit(j["code"].as_i64().map(|c| c as i32)));
            }
            TAG_ERROR => {
                let e: AppError = serde_json::from_slice(body).expect("error json");
                self.event(Event::Error(e));
            }
            TAG_CREDENTIAL_PROMPT => {
                let p: CredentialPrompt = serde_json::from_slice(body).expect("prompt json");
                self.event(Event::CredentialPrompt(p));
            }
            other => panic!("unknown frame tag {other}"),
        }
    }

    fn event(&self, e: Event) {
        if let Ok(mut v) = self.events.lock() {
            v.push(e);
        }
    }

    pub fn data(&self) -> Vec<u8> {
        self.data.lock().map(|d| d.clone()).unwrap_or_default()
    }

    pub fn data_len(&self) -> usize {
        self.data.lock().map(|d| d.len()).unwrap_or(0)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.data()).into_owned()
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    pub fn frame_count(&self) -> usize {
        self.frames.lock().map(|n| *n).unwrap_or(0)
    }

    pub fn max_frame(&self) -> usize {
        self.max_frame.lock().map(|n| *n).unwrap_or(0)
    }

    pub fn exit(&self) -> Option<Option<i32>> {
        self.events().iter().find_map(|e| match e {
            Event::Exit(c) => Some(*c),
            _ => None,
        })
    }

    pub fn error(&self) -> Option<AppError> {
        self.events().iter().find_map(|e| match e {
            Event::Error(err) => Some(err.clone()),
            _ => None,
        })
    }

    pub fn credential_prompts(&self) -> Vec<CredentialPrompt> {
        self.events()
            .iter()
            .filter_map(|e| match e {
                Event::CredentialPrompt(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    /// Whether `needle` appears anywhere in the bytes sent to the UI.
    pub fn raw_contains(&self, needle: &str) -> bool {
        let needle = needle.as_bytes();
        self.raw
            .lock()
            .map(|raw| {
                raw.iter()
                    .any(|f| f.windows(needle.len()).any(|w| w == needle))
            })
            .unwrap_or(false)
    }

    pub fn prompts(&self) -> Vec<(String, String)> {
        self.events()
            .iter()
            .filter_map(|e| match e {
                Event::HostKeyPrompt {
                    fingerprint,
                    algorithm,
                    ..
                } => Some((fingerprint.clone(), algorithm.clone())),
                _ => None,
            })
            .collect()
    }

    /// The host and port each host-key prompt named, in order.
    pub fn prompt_hosts(&self) -> Vec<(String, u16)> {
        self.events()
            .iter()
            .filter_map(|e| match e {
                Event::HostKeyPrompt { host, port, .. } => Some((host.clone(), *port)),
                _ => None,
            })
            .collect()
    }

    /// Acknowledges the data delivered since the previous call.
    pub fn ack_new(&self, session: &Session) {
        let total = self.data_len();
        if let Ok(mut acked) = self.acked.lock() {
            if total > *acked {
                session.ack((total - *acked) as u64);
                *acked = total;
            }
        }
    }
}

/// Acknowledges delivered data in the background until dropped.
pub struct Acker(tokio::task::JoinHandle<()>);

impl Acker {
    pub fn start(session: &Arc<Session>, probe: &Arc<Probe>) -> Self {
        let (s, p) = (session.clone(), probe.clone());
        Self(tokio::spawn(async move {
            loop {
                p.ack_new(&s);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }))
    }
}

impl Drop for Acker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn wait_until(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub async fn wait_text(probe: &Probe, needle: &str, timeout: Duration) {
    let what = format!("{needle:?} in the output");
    wait_until(&what, timeout, || probe.text().contains(needle)).await;
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh scratch directory under `<repo>/.tmp/wp5/`, removed on drop.
pub fn scratch() -> tempfile::TempDir {
    let base = repo_root().join(".tmp/wp5");
    std::fs::create_dir_all(&base).expect("create scratch base");
    tempfile::tempdir_in(base).expect("scratch dir")
}

/// Folder with the generated test credentials (`dev/servers.sh up` creates it). The
/// environment variable lets a run simulate CI, where the folder does not exist.
pub fn keys_dir() -> PathBuf {
    std::env::var_os("PORTOLANO_TEST_KEYS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("dev/keys"))
}

/// `true` when the test sshd answers and its generated credentials exist; otherwise prints
/// why the test is skipped. Tests that need the server or the credentials call this first
/// (through `require_server!`), so reading the credentials later cannot fail.
pub fn server_up() -> bool {
    if std::env::var_os("PORTOLANO_TEST_SERVERS_DOWN").is_some() {
        eprintln!("SKIPPED: PORTOLANO_TEST_SERVERS_DOWN is set (simulating unreachable servers)");
        return false;
    }
    for name in ["test.env", "id_ed25519"] {
        if !keys_dir().join(name).is_file() {
            eprintln!(
                "SKIPPED: {} is missing; generate the test credentials with dev/servers.sh up",
                keys_dir().join(name).display()
            );
            return false;
        }
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], SERVER_PORT));
    match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("SKIPPED: the test sshd on {SERVER_HOST}:{SERVER_PORT} is not reachable ({e}); start it with dev/servers.sh up");
            false
        }
    }
}

/// Skips the current test (returns) when the sshd is not up.
#[macro_export]
macro_rules! require_server {
    () => {
        if !$crate::common::server_up() {
            return;
        }
    };
}

pub fn test_password() -> SecretString {
    let text = std::fs::read_to_string(keys_dir().join("test.env"))
        .expect("test.env (guarded by require_server!)");
    let value = text
        .lines()
        .find_map(|l| l.strip_prefix("TEST_PASSWORD="))
        .expect("TEST_PASSWORD in test.env");
    SecretString::from(value.trim().trim_matches('"').to_string())
}

pub fn test_key() -> SecretString {
    SecretString::from(
        std::fs::read_to_string(keys_dir().join("id_ed25519"))
            .expect("id_ed25519 (guarded by require_server!)"),
    )
}

pub fn hop(auth: SshAuth) -> SshHop {
    SshHop {
        host: SERVER_HOST.into(),
        port: SERVER_PORT,
        username: SERVER_USER.into(),
        auth,
    }
}

pub fn password_hop() -> SshHop {
    hop(SshAuth::Password(test_password()))
}

pub fn key_hop() -> SshHop {
    hop(SshAuth::Key {
        private_key: test_key(),
        passphrase: None,
    })
}

pub fn ssh_spec(target: SshHop, known_hosts: &Path) -> SshSpec {
    SshSpec::new(target, known_hosts)
}

pub fn open_ssh(spec: SshSpec, cols: u16, rows: u16) -> (Arc<Session>, Arc<Probe>) {
    let probe = Arc::new(Probe::default());
    let session = Session::open(SessionSpec::Ssh(Box::new(spec)), cols, rows, probe.sink())
        .expect("open session");
    (Arc::new(session), probe)
}

/// Waits for the first host-key prompt and answers it.
pub async fn answer_prompt(session: &Session, probe: &Probe, accept: bool) -> (String, String) {
    wait_until("a host key prompt", Duration::from_secs(10), || {
        !probe.prompts().is_empty()
    })
    .await;
    let prompt = probe.prompts().remove(0);
    session.host_key_decision(accept).expect("decision");
    prompt
}

/// Connects with the password, accepting the host key, and waits for the shell.
pub async fn connected_shell(known_hosts: &Path) -> (Arc<Session>, Arc<Probe>) {
    let (session, probe) = open_ssh(ssh_spec(password_hop(), known_hosts), 80, 24);
    if probe.prompts().is_empty() {
        wait_until("prompt or data", Duration::from_secs(10), || {
            !probe.prompts().is_empty() || probe.data_len() > 0 || probe.error().is_some()
        })
        .await;
    }
    if !probe.prompts().is_empty() {
        session.host_key_decision(true).expect("accept");
    }
    ready(&session, &probe).await;
    (session, probe)
}

/// Sends a command and waits until the shell answers (proves it is interactive).
pub async fn ready(session: &Session, probe: &Probe) {
    wait_until("the shell", Duration::from_secs(10), || {
        probe.data_len() > 0 || probe.error().is_some()
    })
    .await;
    assert!(
        probe.error().is_none(),
        "unexpected error: {:?}",
        probe.error()
    );
    session
        .write(b"echo READY-$((20+22))\n".to_vec())
        .await
        .expect("write");
    wait_text(probe, "READY-42", Duration::from_secs(10)).await;
}

// ---- credential prompts (WP11) ----

use portolano_core_model::prompt::{CredentialAnswer, PromptHooks};

/// A target hop that asks for its credential; `hint` pre-fills the user name.
pub fn ask_hop(hint: &str, hooks: PromptHooks) -> SshHop {
    SshHop {
        host: SERVER_HOST.into(),
        port: SERVER_PORT,
        username: hint.into(),
        auth: SshAuth::Ask(hooks),
    }
}

pub fn answer(username: &str, password: Option<&SecretString>) -> CredentialAnswer {
    CredentialAnswer {
        username: username.into(),
        password: password.cloned(),
        key_path: None,
        passphrase: None,
        domain: None,
    }
}

/// Waits until `count` credential prompts arrived and returns the last one.
pub async fn wait_credential_prompt(probe: &Probe, count: usize) -> CredentialPrompt {
    wait_until("a credential prompt", Duration::from_secs(20), || {
        probe.credential_prompts().len() >= count || probe.error().is_some()
    })
    .await;
    probe
        .credential_prompts()
        .get(count - 1)
        .cloned()
        .unwrap_or_else(|| panic!("no prompt number {count}; error: {:?}", probe.error()))
}
