//! SSH client on top of `russh`, shaped for a terminal session.
//!
//! Design points (proven by the S2 spike):
//! - the channel is split: a reader loop ([`Connected::pump`]) never blocks on writes, so the
//!   remote window keeps being adjusted during a large paste with echo;
//! - the typed host-key outcome (rejected / changed / certificate) leaves the russh handler
//!   through shared state and is mapped to a stable error code after the handshake fails;
//! - a changed host key never prompts. An unknown one emits a `HostKeyPrompt` and waits for
//!   the user's decision;
//! - the algorithms offered to the server put the ones already stored for the host first, so
//!   a known host presents the key we know (OpenSSH does the same).
//!
//! Secrets are only read from `SecretString` at the moment of use and never formatted into an
//! error or a log line.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portolano_core_model::prompt::{
    CredentialAnswer, CredentialPrompt, PromptHooks, PromptKind, PromptReply, PromptSlot,
    SavedCredential, SavedSecret,
};
use portolano_core_model::{AppError, ErrorCode};
use russh::client::{self, AuthResult, Handle, Msg};
use russh::keys::{decode_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, Preferred};
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};

use crate::flow::Source;
use crate::known_hosts::{fingerprint, HostKeyStatus, KnownHosts, StoredKey};
use crate::session::FrameSink;
use crate::TermEvent;

/// Keepalive used when the connection does not set one.
const DEFAULT_KEEPALIVE_SECS: u64 = 30;
const KEEPALIVE_MAX: usize = 3;

#[derive(Clone)]
pub enum SshAuth {
    Password(SecretString),
    /// Private key text (OpenSSH, also encrypted, or PEM) and its optional passphrase.
    Key {
        private_key: SecretString,
        passphrase: Option<SecretString>,
    },
    /// No credential is known: the user is asked (WP11) for the user name (`SshHop::username`
    /// is only the pre-filled hint) and a password or a key file, per attempt.
    Ask(PromptHooks),
}

/// The user may try this many credentials per hop before the connection fails.
const MAX_PROMPT_ATTEMPTS: u32 = 5;

impl std::fmt::Debug for SshAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password(_) => f.write_str("SshAuth::Password(<redacted>)"),
            Self::Key { .. } => f.write_str("SshAuth::Key(<redacted>)"),
            Self::Ask(_) => f.write_str("SshAuth::Ask"),
        }
    }
}

/// One machine of the chain: where to connect and who to be.
#[derive(Debug, Clone)]
pub struct SshHop {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
}

#[derive(Debug, Clone)]
pub struct SshSpec {
    /// Jump hosts, first to last. Each next hop is reached through a `direct-tcpip` channel of
    /// the previous one. Empty for a direct connection.
    pub jumps: Vec<SshHop>,
    pub target: SshHop,
    /// `None`: 30 s. `Some(0)`: no keepalive.
    pub keepalive_secs: Option<u32>,
    pub known_hosts_path: PathBuf,
    /// TCP connect timeout and handshake timeout (the wait for the user's host-key decision
    /// does not count).
    pub connect_timeout: Duration,
    /// Value sent as `LANG` before the shell is opened (the server may ignore it).
    pub lang: String,
}

impl SshSpec {
    pub fn new(target: SshHop, known_hosts_path: impl Into<PathBuf>) -> Self {
        Self {
            jumps: Vec::new(),
            target,
            keepalive_secs: None,
            known_hosts_path: known_hosts_path.into(),
            connect_timeout: Duration::from_secs(10),
            lang: "en_US.UTF-8".to_string(),
        }
    }
}

/// Where the host-key prompt goes and where the decision comes from.
#[derive(Clone)]
pub(crate) struct Gate {
    pub known_hosts: KnownHosts,
    pub sink: FrameSink,
    /// The sender of the pending prompt, taken by `Session::host_key_decision`.
    pub decision: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    /// True while a prompt is pending (the handshake timeout does not run then).
    pub prompting: Arc<AtomicBool>,
    /// Where credential prompts wait for the user's answer.
    pub credential: Arc<PromptSlot>,
}

#[derive(Debug, Clone)]
enum Outcome {
    Rejected,
    Changed {
        stored: Vec<StoredKey>,
        algorithm: String,
        presented: String,
    },
    Certificate,
    Store(String),
}

struct ClientHandler {
    gate: Gate,
    host: String,
    port: u16,
    outcome: Arc<Mutex<Option<Outcome>>>,
}

impl ClientHandler {
    fn record(&self, o: Outcome) {
        match self.outcome.lock() {
            Ok(mut g) => *g = Some(o),
            Err(e) => *e.into_inner() = Some(o),
        }
    }

    async fn prompt(&self, key: &russh::keys::PublicKey) -> bool {
        let (tx, rx) = oneshot::channel();
        match self.gate.decision.lock() {
            Ok(mut slot) => *slot = Some(tx),
            Err(e) => *e.into_inner() = Some(tx),
        }
        self.gate.prompting.store(true, Ordering::SeqCst);
        let frame = TermEvent::HostKeyPrompt {
            fingerprint: fingerprint(key),
            algorithm: key.algorithm().to_string(),
            host: self.host.clone(),
            port: self.port,
        }
        .encode();
        let accepted = if (self.gate.sink)(frame) {
            // A dropped sender (session closed) counts as a rejection.
            rx.await.unwrap_or(false)
        } else {
            false
        };
        self.gate.prompting.store(false, Ordering::SeqCst);
        accepted
    }
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            PublicKeyOrCertificate::Certificate(_) => {
                self.record(Outcome::Certificate);
                return Ok(false);
            }
        };
        let status = match self.gate.known_hosts.check(&self.host, self.port, key) {
            Ok(s) => s,
            Err(e) => {
                self.record(Outcome::Store(e.to_string()));
                return Ok(false);
            }
        };
        match status {
            HostKeyStatus::Matches => Ok(true),
            HostKeyStatus::Changed { stored } => {
                self.record(Outcome::Changed {
                    stored,
                    algorithm: key.algorithm().to_string(),
                    presented: fingerprint(key),
                });
                Ok(false)
            }
            HostKeyStatus::Unknown => {
                if !self.prompt(key).await {
                    self.record(Outcome::Rejected);
                    return Ok(false);
                }
                match self.gate.known_hosts.add(&self.host, self.port, key) {
                    Ok(()) => Ok(true),
                    Err(e) => {
                        self.record(Outcome::Store(e.to_string()));
                        Ok(false)
                    }
                }
            }
        }
    }
}

fn config(spec: &SshSpec, stored_algorithms: &[String]) -> Arc<client::Config> {
    let mut keys = Preferred::DEFAULT.key.to_vec();
    // Stable sort: algorithms already stored for the host first, the rest keep their order.
    // The three RSA signature variants all belong to the stored `ssh-rsa` key type.
    let is_stored = |a: &russh::keys::Algorithm| {
        stored_algorithms
            .iter()
            .any(|s| *s == a.to_string() || (s == "ssh-rsa" && a.clone().is_rsa()))
    };
    keys.sort_by_key(|a| !is_stored(a));
    let keepalive = match spec.keepalive_secs {
        None => Some(Duration::from_secs(DEFAULT_KEEPALIVE_SECS)),
        Some(0) => None,
        Some(n) => Some(Duration::from_secs(u64::from(n))),
    };
    Arc::new(client::Config {
        keepalive_interval: keepalive,
        keepalive_max: KEEPALIVE_MAX,
        // Liveness is the keepalive's job: an idle shell must not be dropped.
        inactivity_timeout: None,
        nodelay: true,
        preferred: Preferred {
            key: Cow::Owned(keys),
            ..Preferred::DEFAULT
        },
        ..Default::default()
    })
}

fn unreachable_error(msg: String) -> AppError {
    AppError::new(ErrorCode::HostUnreachable, msg)
}

fn map_handshake_error(
    err: &russh::Error,
    outcome: &Mutex<Option<Outcome>>,
    hop: &SshHop,
) -> AppError {
    let recorded = outcome.lock().ok().and_then(|g| g.clone());
    let at = format!("{}:{}", hop.host, hop.port);
    match recorded {
        Some(Outcome::Rejected) => AppError::new(
            ErrorCode::AuthFailed,
            format!("The host key of {at} was not accepted."),
        ),
        Some(Outcome::Changed {
            stored,
            algorithm,
            presented,
        }) => {
            let stored: Vec<String> = stored
                .iter()
                .map(|s| format!("{} {}", s.algorithm, s.fingerprint))
                .collect();
            AppError::new(
                ErrorCode::HostKeyMismatch,
                format!(
                    "The host key of {at} has CHANGED (possible man-in-the-middle attack). \
                     Stored: {}. Presented: {algorithm} {presented}.",
                    stored.join(", ")
                ),
            )
        }
        Some(Outcome::Certificate) => AppError::new(
            ErrorCode::Unsupported,
            format!("{at} presented a host certificate, which is not supported."),
        ),
        Some(Outcome::Store(msg)) => AppError::new(
            ErrorCode::Io,
            format!("The known_hosts file could not be used: {msg}"),
        ),
        None => match err {
            russh::Error::IO(_) | russh::Error::ConnectionTimeout | russh::Error::HUP => {
                unreachable_error(format!(
                    "The connection to {at} was lost during the handshake."
                ))
            }
            other => AppError::new(
                ErrorCode::Io,
                format!("SSH handshake with {at} failed: {other}"),
            ),
        },
    }
}

/// Runs the handshake; the timeout restarts while a host-key prompt is pending.
async fn handshake<S>(
    stream: S,
    hop: &SshHop,
    spec: &SshSpec,
    gate: &Gate,
) -> Result<Handle<ClientHandler>, AppError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let stored = gate
        .known_hosts
        .algorithms(&hop.host, hop.port)
        .map_err(AppError::from)?;
    let outcome = Arc::new(Mutex::new(None));
    let handler = ClientHandler {
        gate: gate.clone(),
        host: hop.host.clone(),
        port: hop.port,
        outcome: outcome.clone(),
    };
    let mut fut = Box::pin(client::connect_stream(
        config(spec, &stored),
        stream,
        handler,
    ));
    loop {
        tokio::select! {
            result = &mut fut => {
                return result.map_err(|e| map_handshake_error(&e, &outcome, hop));
            }
            () = tokio::time::sleep(spec.connect_timeout) => {
                if !gate.prompting.load(Ordering::SeqCst) {
                    return Err(unreachable_error(format!(
                        "The SSH handshake with {}:{} timed out.", hop.host, hop.port
                    )));
                }
            }
        }
    }
}

fn key_error(e: &russh::keys::Error) -> AppError {
    use russh::keys::Error as K;
    match e {
        K::KeyIsEncrypted => AppError::new(
            ErrorCode::AuthFailed,
            "The private key is encrypted: a passphrase is needed.",
        ),
        K::UnsupportedKeyType { .. } | K::UnknownAlgorithm(_) => AppError::new(
            ErrorCode::Unsupported,
            "The type of the private key is not supported.",
        ),
        _ => AppError::new(
            ErrorCode::AuthFailed,
            "The private key could not be read: wrong passphrase, or an unsupported or damaged key.",
        ),
    }
}

async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    hop: &SshHop,
    username: &str,
    auth: &SshAuth,
) -> Result<(), AppError> {
    let lost = |e: russh::Error| {
        AppError::new(
            ErrorCode::Io,
            format!(
                "The connection to {}:{} failed during authentication: {e}",
                hop.host, hop.port
            ),
        )
    };
    let result = match auth {
        SshAuth::Ask(_) => {
            return Err(AppError::internal(
                "a credential prompt reached the authentication step",
            ))
        }
        SshAuth::Password(password) => handle
            .authenticate_password(username.to_owned(), password.expose_secret())
            .await
            .map_err(lost)?,
        SshAuth::Key {
            private_key,
            passphrase,
        } => {
            let key = decode_secret_key(
                private_key.expose_secret(),
                passphrase.as_ref().map(|p| p.expose_secret()),
            )
            .map_err(|e| key_error(&e))?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(lost)?
                .flatten();
            handle
                .authenticate_publickey(
                    username.to_owned(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(lost)?
        }
    };
    match result {
        AuthResult::Success => Ok(()),
        AuthResult::Failure {
            remaining_methods, ..
        } => {
            let methods: Vec<String> = remaining_methods.iter().map(|m| format!("{m:?}")).collect();
            Err(AppError::new(
                ErrorCode::AuthFailed,
                format!(
                    "{}:{} rejected the credentials of user {} (the server accepts: {}).",
                    hop.host,
                    hop.port,
                    username,
                    if methods.is_empty() {
                        "nothing else".to_string()
                    } else {
                        methods.join(", ")
                    }
                ),
            ))
        }
    }
}

/// An authenticated connection with an open PTY shell channel.
pub(crate) struct Connected {
    /// Every hop, first to last. Dropping a handle would tear the tunnel down: keep them all.
    handles: Vec<Handle<ClientHandler>>,
    read: ChannelReadHalf,
    write: Arc<ChannelWriteHalf<Msg>>,
}

/// Cloneable control side of the shell channel.
#[derive(Clone)]
pub(crate) struct SshControl {
    write: Arc<ChannelWriteHalf<Msg>>,
}

impl SshControl {
    pub(crate) async fn write(&self, data: Vec<u8>) -> Result<(), AppError> {
        self.write
            .data_bytes(data)
            .await
            .map_err(|_| AppError::new(ErrorCode::Io, "The SSH session is closed."))
    }

    pub(crate) async fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError> {
        self.write
            .window_change(u32::from(cols), u32::from(rows), 0, 0)
            .await
            .map_err(|_| AppError::new(ErrorCode::Io, "The SSH session is closed."))
    }
}

/// Opens the transport of one hop (TCP for the first, a `direct-tcpip` channel of `previous`
/// for the next ones) and runs the SSH handshake, host key check included.
async fn open_hop(
    previous: Option<&Handle<ClientHandler>>,
    hop: &SshHop,
    spec: &SshSpec,
    gate: &Gate,
) -> Result<Handle<ClientHandler>, AppError> {
    match previous {
        None => {
            let tcp = tokio::time::timeout(
                spec.connect_timeout,
                TcpStream::connect((hop.host.as_str(), hop.port)),
            )
            .await
            .map_err(|_| {
                unreachable_error(format!(
                    "Connecting to {}:{} timed out.",
                    hop.host, hop.port
                ))
            })?
            .map_err(|e| {
                unreachable_error(format!("Cannot connect to {}:{}: {e}", hop.host, hop.port))
            })?;
            if let Err(e) = tcp.set_nodelay(true) {
                tracing::debug!("set_nodelay: {e}");
            }
            handshake(tcp, hop, spec, gate).await
        }
        Some(previous) => {
            let channel = previous
                .channel_open_direct_tcpip(hop.host.clone(), u32::from(hop.port), "127.0.0.1", 0)
                .await
                .map_err(|e| {
                    unreachable_error(format!(
                        "The jump host cannot reach {}:{}: {e}",
                        hop.host, hop.port
                    ))
                })?;
            handshake(channel.into_stream(), hop, spec, gate).await
        }
    }
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "No credential was provided.")
}

/// Emits a credential prompt and waits for the user's reply.
async fn ask(
    gate: &Gate,
    hooks: &PromptHooks,
    hop: &SshHop,
    username_hint: &str,
    error: Option<String>,
) -> Result<(CredentialAnswer, bool), AppError> {
    let reply = gate.credential.arm();
    let hint = username_hint.trim();
    let frame = TermEvent::CredentialPrompt(CredentialPrompt {
        kind: PromptKind::SshKey,
        username_hint: (!hint.is_empty()).then(|| hint.to_owned()),
        domain_hint: None,
        target: format!("{}:{}", hop.host, hop.port),
        error,
        can_save_to_vault: (hooks.can_save)(),
    })
    .encode();
    if !(gate.sink)(frame) {
        return Err(cancelled());
    }
    // A dropped sender (the session was closed) counts as a cancellation.
    match reply.await {
        Ok(PromptReply::Answer {
            answer,
            save_to_vault,
        }) => Ok((answer, save_to_vault)),
        Ok(PromptReply::Cancel) | Err(_) => Err(cancelled()),
    }
}

/// What the typed answer authenticates with. `Err` is a message for the next prompt.
async fn auth_from_answer(answer: &CredentialAnswer) -> Result<SshAuth, String> {
    let passphrase = answer
        .passphrase
        .clone()
        .filter(|p| !p.expose_secret().is_empty());
    if let Some(path) = answer
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let path = expand_home(path);
        let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
            .await
            .map_err(|_| "The key file could not be read.".to_owned())?
            .map_err(|e| format!("The key file could not be read ({}).", e.kind()))?;
        return Ok(SshAuth::Key {
            private_key: SecretString::from(text),
            passphrase,
        });
    }
    match &answer.password {
        Some(p) if !p.expose_secret().is_empty() => Ok(SshAuth::Password(p.clone())),
        _ => Err("Enter a password or choose a private key file.".to_owned()),
    }
}

/// A leading `~/` means the user's home folder (the user typed the path by hand).
fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// Authenticates one hop with credentials typed by the user: prompts, tries, and prompts again
/// with the reason when the server refuses them. A server that closes the connection after a
/// failure is reconnected transparently. Gives up after [`MAX_PROMPT_ATTEMPTS`] attempts.
async fn authenticate_asking(
    handle: &mut Handle<ClientHandler>,
    previous: Option<&Handle<ClientHandler>>,
    hop: &SshHop,
    hooks: &PromptHooks,
    spec: &SshSpec,
    gate: &Gate,
) -> Result<(), AppError> {
    let mut username = hop.username.clone();
    let mut error: Option<String> = None;
    for _ in 0..MAX_PROMPT_ATTEMPTS {
        let (answer, save) = ask(gate, hooks, hop, &username, error.take()).await?;
        username = answer.username.trim().to_owned();
        if username.is_empty() {
            error = Some("Enter a user name.".to_owned());
            continue;
        }
        let auth = match auth_from_answer(&answer).await {
            Ok(auth) => auth,
            Err(message) => {
                error = Some(message);
                continue;
            }
        };
        match authenticate(handle, hop, &username, &auth).await {
            Ok(()) => {
                if save {
                    offer_to_save(hooks, &username, auth).await;
                }
                return Ok(());
            }
            Err(e) if matches!(e.code, ErrorCode::AuthFailed | ErrorCode::Unsupported) => {
                error = Some(e.message);
            }
            Err(e) if e.code == ErrorCode::Io => {
                // The server dropped the connection (too many failures, login grace time):
                // start over on a fresh one and ask again.
                tracing::debug!("authentication connection lost, reconnecting: {e}");
                *handle = open_hop(previous, hop, spec, gate).await?;
                error = Some("The server closed the connection. Try again.".to_owned());
            }
            Err(e) => return Err(e),
        }
    }
    Err(AppError::new(
        ErrorCode::AuthFailed,
        format!(
            "Too many failed attempts to authenticate to {}:{}.",
            hop.host, hop.port
        ),
    ))
}

/// Hands the credential the user chose to keep to the host application, off the async workers,
/// and waits for it: the shell only opens after the credential is saved, so the UI can re-read
/// the document as soon as the session shows signs of life.
async fn offer_to_save(hooks: &PromptHooks, username: &str, auth: SshAuth) {
    let secret = match auth {
        SshAuth::Password(password) => SavedSecret::Password(password),
        SshAuth::Key {
            private_key,
            passphrase,
        } => SavedSecret::SshKey {
            private_key_pem: private_key,
            passphrase,
        },
        SshAuth::Ask(_) => return,
    };
    let saved = hooks.saved.clone();
    let credential = SavedCredential {
        username: username.to_owned(),
        domain: None,
        secret,
    };
    if let Err(e) = tokio::task::spawn_blocking(move || saved(credential)).await {
        tracing::warn!("saving the typed credential failed: {e}");
    }
}

/// Connects through the jump hosts (if any) to the target, authenticates every hop and opens
/// the shell with a PTY of `cols` x `rows`.
pub(crate) async fn connect(
    spec: &SshSpec,
    gate: &Gate,
    cols: u16,
    rows: u16,
) -> Result<Connected, AppError> {
    let hops: Vec<&SshHop> = spec
        .jumps
        .iter()
        .chain(std::iter::once(&spec.target))
        .collect();
    let mut handles: Vec<Handle<ClientHandler>> = Vec::with_capacity(hops.len());
    for hop in &hops {
        let mut handle = open_hop(handles.last(), hop, spec, gate).await?;
        let authenticated = match &hop.auth {
            SshAuth::Ask(hooks) => {
                authenticate_asking(&mut handle, handles.last(), hop, hooks, spec, gate).await
            }
            fixed => authenticate(&mut handle, hop, &hop.username, fixed).await,
        };
        if let Err(e) = authenticated {
            // Leave politely: an abandoned half-open connection keeps a server slot busy
            // until its login grace time runs out.
            handles.push(handle);
            disconnect_all(&handles).await;
            return Err(e);
        }
        handles.push(handle);
    }

    let io = |what: &str, e: russh::Error| AppError::new(ErrorCode::Io, format!("{what}: {e}"));
    let last = handles
        .last()
        .ok_or_else(|| AppError::internal("no SSH hop to connect to"))?;
    let channel = last
        .channel_open_session()
        .await
        .map_err(|e| io("Cannot open a session channel", e))?;
    channel
        .request_pty(
            false,
            "xterm-256color",
            u32::from(cols),
            u32::from(rows),
            0,
            0,
            &[],
        )
        .await
        .map_err(|e| io("Cannot request a terminal", e))?;
    // No reply wanted: a server that does not accept the variable just ignores it.
    channel
        .set_env(false, "LANG", spec.lang.clone())
        .await
        .map_err(|e| io("Cannot set the locale", e))?;
    channel
        .request_shell(false)
        .await
        .map_err(|e| io("Cannot start the shell", e))?;
    let (read, write) = channel.split();
    Ok(Connected {
        handles,
        read,
        write: Arc::new(write),
    })
}

impl Connected {
    pub(crate) fn control(&self) -> SshControl {
        SshControl {
            write: self.write.clone(),
        }
    }

    /// Reads the channel until it ends or the session is closed. Output goes to `tx`; the end
    /// is reported as `Exit` (clean close) or `Fatal` (connection lost).
    pub(crate) async fn pump(self, tx: mpsc::Sender<Source>, mut closed: watch::Receiver<bool>) {
        let Self {
            handles,
            mut read,
            write,
        } = self;
        let mut exit: Option<Option<i32>> = None;
        let mut clean = false;
        let mut stopped_by_us = false;
        loop {
            let msg = tokio::select! {
                msg = read.wait() => msg,
                _ = closed.wait_for(|c| *c) => {
                    stopped_by_us = true;
                    break;
                }
            };
            match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if tx.send(Source::Data(data.to_vec())).await.is_err() {
                        stopped_by_us = true;
                        break;
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit = Some(i32::try_from(exit_status).ok());
                }
                Some(ChannelMsg::ExitSignal { .. }) => exit = Some(None),
                Some(ChannelMsg::Close) => {
                    clean = true;
                    break;
                }
                Some(_) => {}
                None => break,
            }
        }
        if !stopped_by_us {
            let last = if clean || exit.is_some() {
                Source::Exit(exit.flatten())
            } else {
                Source::Fatal(AppError::new(
                    ErrorCode::HostUnreachable,
                    "The connection to the server was lost.",
                ))
            };
            if tx.send(last).await.is_err() {
                tracing::debug!("session closed before the end of the stream was delivered");
            }
        }
        shutdown(&write, handles).await;
    }
}

/// Sends a disconnect on every hop, last first. Best effort, bounded in time.
async fn disconnect_all(handles: &[Handle<ClientHandler>]) {
    let work = async {
        for handle in handles.iter().rev() {
            if let Err(e) = handle.disconnect(Disconnect::ByApplication, "", "en").await {
                tracing::debug!("disconnect: {e}");
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(2), work)
        .await
        .is_err()
    {
        tracing::debug!("SSH disconnect timed out");
    }
}

/// Best-effort orderly shutdown, bounded in time.
async fn shutdown(write: &ChannelWriteHalf<Msg>, handles: Vec<Handle<ClientHandler>>) {
    let work = async {
        // EOF and Close fail when the remote already closed the channel: that is fine.
        if let Err(e) = write.eof().await {
            tracing::debug!("eof while closing: {e}");
        }
        if let Err(e) = write.close().await {
            tracing::debug!("close while closing: {e}");
        }
        for handle in handles.iter().rev() {
            if let Err(e) = handle.disconnect(Disconnect::ByApplication, "", "en").await {
                tracing::debug!("disconnect: {e}");
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(3), work)
        .await
        .is_err()
    {
        tracing::debug!("orderly SSH shutdown timed out");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(keepalive: Option<u32>) -> SshSpec {
        SshSpec {
            keepalive_secs: keepalive,
            ..SshSpec::new(
                SshHop {
                    host: "h".into(),
                    port: 22,
                    username: "u".into(),
                    auth: SshAuth::Password(SecretString::from("x")),
                },
                "/nonexistent/known_hosts",
            )
        }
    }

    #[test]
    fn keepalive_follows_the_connection_setting() {
        assert_eq!(
            config(&spec(None), &[]).keepalive_interval,
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            config(&spec(Some(45)), &[]).keepalive_interval,
            Some(Duration::from_secs(45))
        );
        assert_eq!(config(&spec(Some(0)), &[]).keepalive_interval, None);
    }

    #[test]
    fn stored_algorithms_are_offered_first() {
        let stored = vec!["ssh-rsa".to_string()];
        let cfg = config(&spec(None), &stored);
        let first = cfg.preferred.key.first().map(ToString::to_string);
        assert_eq!(first.as_deref(), Some("rsa-sha2-512"));
        assert!(cfg.preferred.key.iter().take(3).all(|a| a.clone().is_rsa()));
        // Nothing is dropped.
        assert_eq!(cfg.preferred.key.len(), Preferred::DEFAULT.key.len());
        let plain = config(&spec(None), &[]);
        assert_eq!(
            plain
                .preferred
                .key
                .first()
                .map(ToString::to_string)
                .as_deref(),
            Some("ssh-ed25519")
        );
    }

    #[test]
    fn auth_debug_never_shows_secrets() {
        let auth = SshAuth::Password(SecretString::from("super-secret-value"));
        assert!(!format!("{auth:?}").contains("super-secret"));
        let key = SshAuth::Key {
            private_key: SecretString::from("super-secret-key"),
            passphrase: Some(SecretString::from("super-secret-pass")),
        };
        assert!(!format!("{key:?}").contains("super-secret"));
    }
}
