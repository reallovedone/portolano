//! One VNC session: connection and authentication, the reader task, and the handle used by
//! the IPC commands.
//!
//! # Design
//!
//! Same contract as the RDP session. The decoded desktop lives in the backend (a shared RGBA
//! image); the UI only ever receives `FbEvent`s through an [`EventSink`]:
//!
//! - a **reader task** owns the read half of the socket: it decodes server messages into the
//!   shared image (the lock is held only while pixels are copied) and tells the session task
//!   what changed through a small bounded channel;
//! - the **session task** collects the changed rectangles in a [`DirtyRegion`], merges them
//!   and flushes at most every [`FLUSH_INTERVAL`]; there is no per-update message and no
//!   queue: whatever changed since the last flush is read from the current image (latest
//!   wins). It also owns the write half: input, update requests, close;
//! - exactly one FramebufferUpdateRequest is outstanding while the tab is visible, so the
//!   server never produces more than we consume. While the tab is hidden no new request is
//!   made (the server goes quiet), nothing is sent to the UI, and when it is shown again one
//!   full-image rectangle is sent plus a non-incremental request;
//! - input arrives through an unbounded command channel; consecutive pointer moves are
//!   collapsed before they are encoded.
//!
//! # Encryption
//!
//! Plain VNC is not encrypted. When the server offers VeNCrypt with an X509 subtype the
//! session upgrades the connection to TLS (see `tls.rs`) before any secret is sent: the
//! server certificate is pinned by its SHA-256 fingerprint (trust on first use, `cert.rs`) and
//! the user decides on a new or changed one through `FbEvent::CertPrompt` and `cert_decision`.
//! Once connected the session reports whether the connection is encrypted with one
//! `FbEvent::Security`, right before the first `Resize`.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use portolano_core_model::fb::FbEvent;
use portolano_core_model::prompt::{
    CredentialAnswer, CredentialPrompt, PromptHooks, PromptKind, PromptReply, PromptSlot,
    SavedCredential, SavedSecret,
};
use secrecy::{ExposeSecret, SecretString};
use tokio::io::{AsyncRead, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::cert::{CertCheck, CertStore, PlainStore};
use crate::dirty::{DirtyRegion, Rect};
use crate::error::VncError;
use crate::input::InputState;
use crate::keymap;
use crate::rfb::{
    self, Auth, Credentials, Image, Method, Notice, SharedImage, VeNCrypt, ZrleStream,
};
use crate::tls::{self, Conn};

/// Receives the events of the session. Returns `false` when nobody listens any more, which
/// ends the session.
pub type EventSink = Arc<dyn Fn(FbEvent) -> bool + Send + Sync>;

/// Minimum time between two flushes of dirty rectangles (40 Hz at most).
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(25);

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the user may take to answer a certificate prompt.
const CERT_DECISION_TIMEOUT: Duration = Duration::from_secs(600);
/// Commands handled per loop turn before the network is polled again.
const COMMAND_BATCH: usize = 128;
/// Notices the reader may be ahead of the session task before it waits (backpressure).
const NOTICE_QUEUE: usize = 8;

/// The user may try this many passwords before the connection fails.
const MAX_PROMPT_ATTEMPTS: u32 = 5;

/// VNC Authentication has no user name; a credential saved from that prompt gets this
/// placeholder. (VeNCrypt Plain does have one, and saves what the user typed.)
pub const SAVED_USERNAME: &str = "VNC";

/// The session asks the user for the credentials (WP11): used when the connection has none.
#[derive(Clone)]
pub struct VncAsk {
    pub hooks: PromptHooks,
}

/// Everything a session needs, resolved by the caller (the IPC layer reads the connection
/// document and the vault).
#[derive(Clone)]
pub struct VncConfig {
    pub host: String,
    pub port: u16,
    /// The user name for VeNCrypt Plain. Ignored by the other security types.
    pub username: Option<String>,
    /// The password for VNC Authentication and VeNCrypt Plain. Ignored when the server needs
    /// none.
    pub password: Option<SecretString>,
    /// With no `password` (or, for Plain, no user name): ask the user when (and only when) the
    /// server wants credentials.
    pub ask: Option<VncAsk>,
    /// Where accepted server certificates are pinned. `None`: nothing is remembered and every
    /// connection asks again.
    pub cert_store: Option<CertStore>,
    /// Where the user's agreement to authenticate without encryption is remembered, per host.
    /// `None`: the session asks on every connection.
    pub plain_store: Option<PlainStore>,
    /// Ignore all input.
    pub view_only: bool,
}

impl VncConfig {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            username: None,
            password: None,
            ask: None,
            cert_store: None,
            plain_store: None,
            view_only: false,
        }
    }
}

impl std::fmt::Debug for VncConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VncConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username.is_some())
            .field("password", &self.password.is_some())
            .field("ask", &self.ask.is_some())
            .field("view_only", &self.view_only)
            .finish()
    }
}

/// A copy of the backend image (RGBA, tightly packed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

enum Cmd {
    Pointer {
        x: u16,
        y: u16,
        buttons: u8,
        wheel: i16,
    },
    Key {
        code: &'static str,
        down: bool,
    },
    Visible(bool),
    Snapshot(oneshot::Sender<Snapshot>),
}

/// Handle of a running session. Dropping it closes the session.
pub struct VncSession {
    cmd: mpsc::UnboundedSender<Cmd>,
    prompt: Arc<PromptSlot>,
    cert: mpsc::UnboundedSender<bool>,
    plain: mpsc::UnboundedSender<bool>,
    close: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}

fn closed_error() -> VncError {
    VncError::Io("the session is closed".into())
}

impl VncSession {
    /// Starts the session on the current tokio runtime. Progress, prompts and failures are
    /// reported through `sink`; this never blocks.
    pub fn start(config: VncConfig, sink: EventSink) -> Self {
        let (cmd, cmd_rx) = mpsc::unbounded_channel();
        let (cert, cert_rx) = mpsc::unbounded_channel();
        let (plain, plain_rx) = mpsc::unbounded_channel();
        let (close, close_rx) = watch::channel(false);
        let (done_tx, done) = watch::channel(false);
        let prompt = Arc::new(PromptSlot::new());
        let task_prompt = prompt.clone();
        tokio::spawn(async move {
            let decisions = Decisions { cert_rx, plain_rx };
            run(config, sink, cmd_rx, decisions, task_prompt, close_rx).await;
            // Receivers may all be gone already; nothing to report then.
            done_tx.send_replace(true);
        });
        Self {
            cmd,
            prompt,
            cert,
            plain,
            close,
            done,
        }
    }

    fn send(&self, cmd: Cmd) -> Result<(), VncError> {
        self.cmd.send(cmd).map_err(|_| closed_error())
    }

    /// `buttons` uses the DOM `MouseEvent.buttons` bits. `wheel` is in 120-unit notches,
    /// positive scrolls down.
    pub fn pointer(&self, x: u16, y: u16, buttons: u8, wheel: i16) -> Result<(), VncError> {
        self.send(Cmd::Pointer {
            x,
            y,
            buttons,
            wheel,
        })
    }

    /// `code` is `KeyboardEvent.code`. Keys without a keysym are ignored.
    pub fn key(&self, code: &str, down: bool) -> Result<(), VncError> {
        match keymap::lookup(code) {
            Some((name, _, _)) => self.send(Cmd::Key { code: name, down }),
            None => {
                tracing::debug!(code, "ignoring a key without a keysym");
                Ok(())
            }
        }
    }

    /// A no-op: the server dictates the desktop size and the UI scales the image.
    pub fn resize(&self, _width: u16, _height: u16) -> Result<(), VncError> {
        Ok(())
    }

    /// Hidden: nothing is sent. Visible: one full redraw.
    pub fn set_visible(&self, visible: bool) -> Result<(), VncError> {
        self.send(Cmd::Visible(visible))
    }

    /// Answer to a `CertPrompt`.
    pub fn cert_decision(&self, accept: bool) -> Result<(), VncError> {
        self.cert.send(accept).map_err(|_| closed_error())
    }

    /// Answer to an `UnencryptedPrompt`: `true` agrees to send the VNC password without
    /// encryption (and remembers it for this host), `false` closes the session.
    pub fn unencrypted_decision(&self, allow: bool) -> Result<(), VncError> {
        self.plain.send(allow).map_err(|_| closed_error())
    }

    /// Answer to a `CredentialPrompt`. The secret is used for this session only.
    pub fn credential(
        &self,
        answer: CredentialAnswer,
        save_to_vault: bool,
    ) -> Result<(), VncError> {
        self.prompt
            .reply(PromptReply::Answer {
                answer,
                save_to_vault,
            })
            .map_err(|e| VncError::InvalidInput(e.message))
    }

    /// The user dismissed the `CredentialPrompt`: the session ends with `cancelled`.
    pub fn credential_cancel(&self) -> Result<(), VncError> {
        self.prompt
            .reply(PromptReply::Cancel)
            .map_err(|e| VncError::InvalidInput(e.message))
    }

    /// Asks the session to end. Does not block; see [`VncSession::closed`].
    pub fn close(&self) {
        self.close.send_replace(true);
    }

    /// Resolves when the session task has finished.
    pub async fn closed(&self) {
        let mut done = self.done.clone();
        // An error means the task is gone, which is what we are waiting for.
        if done.wait_for(|finished| *finished).await.is_err() {
            tracing::trace!("session task already dropped");
        }
    }

    /// A copy of the backend image, `None` when the session is not active.
    pub async fn snapshot(&self) -> Option<Snapshot> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::Snapshot(tx)).ok()?;
        rx.await.ok()
    }
}

impl Drop for VncSession {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------------------

/// The answers of the user to the yes/no prompts of the session.
struct Decisions {
    cert_rx: mpsc::UnboundedReceiver<bool>,
    plain_rx: mpsc::UnboundedReceiver<bool>,
}

async fn run(
    config: VncConfig,
    sink: EventSink,
    cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    decisions: Decisions,
    prompt: Arc<PromptSlot>,
    close_rx: watch::Receiver<bool>,
) {
    let result = run_inner(&config, &sink, cmd_rx, decisions, &prompt, close_rx).await;
    // `false` from the sink only means nobody is listening: nothing left to do.
    let delivered = match result {
        Ok(()) | Err(VncError::Cancelled) => true,
        // A changed certificate was reported as a prompt already: nothing more to say.
        Err(VncError::CertificateChanged) => true,
        Err(VncError::CertDeclined) => sink(FbEvent::Closed {
            reason: "the server certificate was not accepted".into(),
        }),
        Err(VncError::UnencryptedDeclined) => sink(FbEvent::Closed {
            reason: "the connection is not encrypted and you did not allow that".into(),
        }),
        Err(VncError::ServerClosed(reason)) => sink(FbEvent::Closed { reason }),
        Err(e) => {
            if !matches!(e, VncError::PromptCancelled) {
                tracing::warn!(host = %config.host, code = e.code().as_str(), "VNC session failed: {e}");
            }
            sink(FbEvent::Error(e.into()))
        }
    };
    if !delivered {
        tracing::debug!("VNC session ended and nobody was listening");
    }
}

/// Runs `fut` unless the session is closed first.
async fn cancellable<T>(
    close: &mut watch::Receiver<bool>,
    fut: impl Future<Output = Result<T, VncError>>,
) -> Result<T, VncError> {
    if *close.borrow() {
        return Err(VncError::Cancelled);
    }
    tokio::select! {
        biased;
        _ = close.changed() => Err(VncError::Cancelled),
        r = fut => r,
    }
}

async fn with_timeout<T>(
    what: &str,
    limit: Duration,
    fut: impl Future<Output = Result<T, VncError>>,
) -> Result<T, VncError> {
    match tokio::time::timeout(limit, fut).await {
        Ok(r) => r,
        Err(_) => Err(VncError::Io(format!("{what} timed out"))),
    }
}

async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, VncError> {
    let stream =
        match tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect((host, port))).await {
            Err(_) => {
                return Err(VncError::HostUnreachable(format!(
                    "connecting to {host}:{port} timed out"
                )))
            }
            Ok(Err(e)) => {
                return Err(VncError::HostUnreachable(format!(
                    "cannot connect to {host}:{port}: {e}"
                )))
            }
            Ok(Ok(stream)) => stream,
        };
    stream
        .set_nodelay(true)
        .map_err(|e| VncError::Io(format!("cannot configure the socket: {e}")))?;
    Ok(stream)
}

struct Established {
    reader: ReadHalf<Conn>,
    writer: WriteHalf<Conn>,
    init: rfb::ServerInit,
    auth: Auth,
    method: Method,
    encrypted: bool,
}

/// The secrets known so far (from the vault, the connection or the user).
#[derive(Clone, Default)]
struct Secrets {
    username: Option<String>,
    password: Option<SecretString>,
}

/// What the session remembers about certificate decisions between connection attempts.
struct Trust<'a> {
    cert_rx: &'a mut mpsc::UnboundedReceiver<bool>,
    plain_rx: &'a mut mpsc::UnboundedReceiver<bool>,
    /// The user agreed (or had agreed) to authenticate without encryption for this session.
    plain_ok: bool,
    /// The fingerprint the user (or the store) accepted for this session: a reconnect after a
    /// credential prompt must not ask about the same certificate again.
    accepted: Option<String>,
    /// The user answered a certificate prompt during the current attempt.
    prompted: bool,
}

/// Compares the presented certificate with the store and asks the user when needed. Nothing
/// secret has been sent yet. A refusal is `CertDeclined` and leaves the store untouched.
async fn check_certificate(
    config: &VncConfig,
    sink: &EventSink,
    trust: &mut Trust<'_>,
    fingerprint: &str,
    close: &mut watch::Receiver<bool>,
) -> Result<(), VncError> {
    if trust.accepted.as_deref() == Some(fingerprint) {
        return Ok(());
    }
    if let Some(store) = &config.cert_store {
        match store
            .check(&config.host, config.port, fingerprint)
            .map_err(|e| VncError::Io(format!("cannot read the trusted certificates: {e}")))?
        {
            CertCheck::Matches => {
                trust.accepted = Some(fingerprint.to_owned());
                return Ok(());
            }
            CertCheck::Unknown => {}
            // Like a changed SSH host key: no way to carry on from here. The user has to
            // remove the stored certificate on purpose and connect again.
            CertCheck::Changed { stored } => {
                let delivered = sink(FbEvent::CertPrompt {
                    fingerprint: fingerprint.to_owned(),
                    changed: true,
                    stored: Some(stored),
                });
                return Err(if delivered {
                    VncError::CertificateChanged
                } else {
                    VncError::Cancelled
                });
            }
        }
    }
    // A decision sent while no prompt was pending must not answer this one.
    while trust.cert_rx.try_recv().is_ok() {}
    if !sink(FbEvent::CertPrompt {
        fingerprint: fingerprint.to_owned(),
        changed: false,
        stored: None,
    }) {
        return Err(VncError::Cancelled);
    }
    let rx = &mut *trust.cert_rx;
    let accept = cancellable(close, async {
        match tokio::time::timeout(CERT_DECISION_TIMEOUT, rx.recv()).await {
            Ok(Some(accept)) => Ok(accept),
            Ok(None) => Err(VncError::Cancelled),
            Err(_) => Err(VncError::Io("no answer to the certificate prompt".into())),
        }
    })
    .await?;
    if !accept {
        return Err(VncError::CertDeclined);
    }
    if let Some(store) = &config.cert_store {
        store
            .remember(&config.host, config.port, fingerprint)
            .map_err(|e| VncError::Io(format!("cannot store the certificate: {e}")))?;
    }
    trust.accepted = Some(fingerprint.to_owned());
    trust.prompted = true;
    Ok(())
}

/// Whether a certificate is pinned for the host: it has used TLS before.
fn is_pinned(config: &VncConfig) -> Result<bool, VncError> {
    match &config.cert_store {
        None => Ok(false),
        Some(store) => store
            .has_pin(&config.host, config.port)
            .map_err(|e| VncError::Io(format!("cannot read the trusted certificates: {e}"))),
    }
}

fn downgrade_error(config: &VncConfig) -> VncError {
    VncError::SecurityDowngrade(format!(
        "security downgrade refused: {}:{} used an encrypted connection before and now offers \
         none, so the password was not sent. If the server was reconfigured on purpose, remove \
         its stored certificate first",
        config.host, config.port
    ))
}

/// Asks the user, before anything derived from the password is sent, whether VNC
/// Authentication may be used without encryption for this host. Asked once per host and
/// remembered. Declining ends the session cleanly.
async fn confirm_unencrypted(
    config: &VncConfig,
    sink: &EventSink,
    trust: &mut Trust<'_>,
    close: &mut watch::Receiver<bool>,
) -> Result<(), VncError> {
    if trust.plain_ok {
        return Ok(());
    }
    if let Some(store) = &config.plain_store {
        if store
            .allowed(&config.host, config.port)
            .map_err(|e| VncError::Io(format!("cannot read the allowed hosts: {e}")))?
        {
            trust.plain_ok = true;
            return Ok(());
        }
    }
    // A decision sent while no prompt was pending must not answer this one.
    while trust.plain_rx.try_recv().is_ok() {}
    if !sink(FbEvent::UnencryptedPrompt) {
        return Err(VncError::Cancelled);
    }
    let rx = &mut *trust.plain_rx;
    let allow = cancellable(close, async {
        match tokio::time::timeout(CERT_DECISION_TIMEOUT, rx.recv()).await {
            Ok(Some(allow)) => Ok(allow),
            Ok(None) => Err(VncError::Cancelled),
            Err(_) => Err(VncError::Io("no answer to the encryption prompt".into())),
        }
    })
    .await?;
    if !allow {
        return Err(VncError::UnencryptedDeclined);
    }
    if let Some(store) = &config.plain_store {
        store
            .allow(&config.host, config.port)
            .map_err(|e| VncError::Io(format!("cannot store the decision: {e}")))?;
    }
    trust.plain_ok = true;
    trust.prompted = true;
    Ok(())
}

/// One connection attempt: TCP, security negotiation, TLS and the certificate decision when
/// VeNCrypt is used, then authentication.
async fn establish(
    config: &VncConfig,
    secrets: &Secrets,
    vencrypt: VeNCrypt,
    sink: &EventSink,
    trust: &mut Trust<'_>,
    close: &mut watch::Receiver<bool>,
) -> Result<Established, VncError> {
    let mut tcp = cancellable(close, connect_tcp(&config.host, config.port)).await?;
    let negotiated = cancellable(
        close,
        with_timeout(
            "the VNC handshake",
            HANDSHAKE_TIMEOUT,
            rfb::negotiate(&mut tcp, vencrypt, secrets.password.is_some()),
        ),
    )
    .await;
    let plan = match negotiated {
        // The server wants VNC Authentication over plain TCP: for a host that used TLS
        // before, that is a downgrade (and no password prompt is worth showing).
        Err(VncError::PasswordRequired) if is_pinned(config)? => {
            return Err(downgrade_error(config))
        }
        other => other?,
    };
    if !plan.tls {
        // A host with a stored certificate never falls back to a weaker scheme.
        if is_pinned(config)? {
            return Err(downgrade_error(config));
        }
        // The password exchange of VNC Authentication is derived from the password: the user
        // agrees to it first.
        if plan.method == Method::VncPassword {
            confirm_unencrypted(config, sink, trust, close).await?;
        }
    }
    let mut conn = if plan.tls {
        let upgraded = cancellable(
            close,
            with_timeout(
                "the TLS handshake",
                HANDSHAKE_TIMEOUT,
                tls::upgrade(tcp, &config.host),
            ),
        )
        .await?;
        // Nothing secret goes out before the user has decided on this certificate.
        check_certificate(config, sink, trust, &upgraded.fingerprint, close).await?;
        Conn::Tls(Box::new(upgraded.stream))
    } else {
        Conn::Plain(tcp)
    };
    let credentials = Credentials {
        username: secrets.username.as_deref(),
        password: secrets.password.as_ref(),
    };
    let (init, auth) = cancellable(
        close,
        with_timeout(
            "the VNC authentication",
            HANDSHAKE_TIMEOUT,
            rfb::authenticate(&mut conn, plan, &credentials),
        ),
    )
    .await?;
    let encrypted = conn.is_encrypted();
    let (reader, writer) = tokio::io::split(conn);
    Ok(Established {
        reader,
        writer,
        init,
        auth,
        method: plan.method,
        encrypted,
    })
}

struct Asked {
    kind: PromptKind,
    /// Set for `PromptKind::UsernamePassword`.
    username: Option<String>,
    password: SecretString,
    save: bool,
}

/// What a credential prompt shows.
struct PromptSpec {
    kind: PromptKind,
    username_hint: Option<String>,
    error: Option<String>,
}

/// Emits a credential prompt and waits for the reply: the password only (`Password`) or user
/// name and password (`UsernamePassword`). A cancellation is `VncError::PromptCancelled`;
/// closing the session meanwhile is `VncError::Cancelled`.
async fn ask_credentials(
    config: &VncConfig,
    ask: &VncAsk,
    sink: &EventSink,
    slot: &PromptSlot,
    close: &mut watch::Receiver<bool>,
    spec: PromptSpec,
) -> Result<Asked, VncError> {
    let PromptSpec {
        kind,
        username_hint,
        mut error,
    } = spec;
    loop {
        let reply = slot.arm();
        let prompt = CredentialPrompt {
            kind,
            username_hint: username_hint.clone(),
            domain_hint: None,
            target: format!("{}:{}", config.host, config.port),
            error: error.take(),
            can_save_to_vault: (ask.hooks.can_save)(),
        };
        if !sink(FbEvent::CredentialPrompt(prompt)) {
            return Err(VncError::Cancelled);
        }
        let reply = cancellable(close, async {
            reply.await.map_err(|_| VncError::Cancelled)
        })
        .await?;
        let (answer, save) = match reply {
            PromptReply::Answer {
                answer,
                save_to_vault,
            } => (answer, save_to_vault),
            PromptReply::Cancel => return Err(VncError::PromptCancelled),
        };
        match credentials_from_answer(kind, answer) {
            Ok((username, password)) => {
                return Ok(Asked {
                    kind,
                    username,
                    password,
                    save,
                })
            }
            Err(message) => error = Some(message),
        }
    }
}

/// Validates a typed answer. `Err` is a message for the next prompt.
fn credentials_from_answer(
    kind: PromptKind,
    answer: CredentialAnswer,
) -> Result<(Option<String>, SecretString), String> {
    let password = match answer.password {
        Some(p) if !p.expose_secret().is_empty() => p,
        _ if kind == PromptKind::UsernamePassword => {
            return Err("Enter a user name and a password.".to_owned())
        }
        _ => return Err("Enter a password.".to_owned()),
    };
    if kind == PromptKind::UsernamePassword {
        let username = answer.username.trim();
        if username.is_empty() {
            return Err("Enter a user name and a password.".to_owned());
        }
        return Ok((Some(username.to_owned()), password));
    }
    Ok((None, password))
}

/// Whether a failed attempt with typed credentials is followed by a new prompt.
fn asks_again(error: &VncError, attempts: u32) -> bool {
    matches!(error, VncError::AuthFailed(_)) && attempts < MAX_PROMPT_ATTEMPTS
}

/// Connects, asking for credentials when the server wants some and they are not known. Wrong
/// typed credentials mean a new connection (servers close the connection after a failure) and
/// a new prompt that carries the error; an accepted certificate is not asked about again.
/// Credentials that came from the vault are tried once.
async fn establish_asking(
    config: &VncConfig,
    sink: &EventSink,
    slot: &PromptSlot,
    cert_rx: &mut mpsc::UnboundedReceiver<bool>,
    plain_rx: &mut mpsc::UnboundedReceiver<bool>,
    close: &mut watch::Receiver<bool>,
) -> Result<Established, VncError> {
    let mut secrets = Secrets {
        username: config.username.clone(),
        password: config.password.clone(),
    };
    let mut trust = Trust {
        cert_rx,
        plain_rx,
        plain_ok: false,
        accepted: None,
        prompted: false,
    };
    let mut vencrypt = VeNCrypt::Allowed;
    let mut typed: Option<Asked> = None;
    let mut attempts = 0u32;
    let mut reconnected_after_prompt = false;
    loop {
        trust.prompted = false;
        match establish(config, &secrets, vencrypt, sink, &mut trust, close).await {
            Ok(established) => {
                // Only credentials the server actually checked are worth keeping.
                if let (Some(asked), Auth::Password) = (typed.take(), established.auth) {
                    if asked.save {
                        if let Some(ask) = &config.ask {
                            offer_to_save(ask, asked, established.method).await;
                        }
                    }
                }
                return Ok(established);
            }
            Err(e) => {
                // The server offered VeNCrypt but nothing usable in it: use another type.
                if let VncError::VeNCryptUnusable { anonymous } = e {
                    // A host that used TLS before must not drop to a weaker type.
                    if is_pinned(config)? {
                        return Err(downgrade_error(config));
                    }
                    vencrypt = VeNCrypt::Skipped {
                        anonymous_offered: anonymous,
                    };
                    continue;
                }
                // The server may have dropped a connection that waited for the user's
                // certificate decision: the certificate is trusted now, so reconnect once.
                if matches!(e, VncError::Io(_)) && trust.prompted && !reconnected_after_prompt {
                    reconnected_after_prompt = true;
                    continue;
                }
                let Some(ask) = &config.ask else {
                    return Err(match e {
                        VncError::PasswordRequired => VncError::AuthFailed(
                            "the server needs a password and the connection has none".into(),
                        ),
                        VncError::PlainCredentialsRequired => VncError::AuthFailed(
                            "the server needs a user name and a password and the connection has none"
                                .into(),
                        ),
                        other => other,
                    });
                };
                let (kind, error) = match &e {
                    VncError::PasswordRequired => (PromptKind::Password, None),
                    VncError::PlainCredentialsRequired => (PromptKind::UsernamePassword, None),
                    // Only typed credentials are asked again; ones from the vault are final.
                    e if typed.is_some() && asks_again(e, attempts) => (
                        typed.as_ref().map_or(PromptKind::Password, |a| a.kind),
                        Some(e.to_string()),
                    ),
                    _ => return Err(e),
                };
                attempts += 1;
                let spec = PromptSpec {
                    kind,
                    username_hint: secrets.username.clone(),
                    error,
                };
                let asked = ask_credentials(config, ask, sink, slot, close, spec).await?;
                secrets.password = Some(asked.password.clone());
                if asked.username.is_some() {
                    secrets.username.clone_from(&asked.username);
                }
                typed = Some(asked);
            }
        }
    }
}

/// Hands the credentials the user chose to keep to the host application, off the async
/// workers, and waits for it: the desktop only starts after the credential is saved, so the UI
/// can re-read the document as soon as the session shows signs of life.
async fn offer_to_save(ask: &VncAsk, asked: Asked, method: Method) {
    let saved = ask.hooks.saved.clone();
    let username = match (method, asked.username) {
        (Method::Plain, Some(username)) => username,
        _ => SAVED_USERNAME.to_owned(),
    };
    let credential = SavedCredential {
        username,
        domain: None,
        secret: SavedSecret::Password(asked.password),
    };
    if let Err(e) = tokio::task::spawn_blocking(move || saved(credential)).await {
        tracing::warn!("saving the typed credential failed: {e}");
    }
}

/// Drops pointer moves that a later pointer command supersedes (same buttons, no wheel).
fn coalesce(cmds: Vec<Cmd>) -> Vec<Cmd> {
    let mut out: Vec<Cmd> = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        if let (
            Some(Cmd::Pointer {
                buttons: b0,
                wheel: w0,
                ..
            }),
            Cmd::Pointer { buttons, wheel, .. },
        ) = (out.last(), &cmd)
        {
            if b0 == buttons && *w0 == 0 && *wheel == 0 {
                out.pop();
            }
        }
        out.push(cmd);
    }
    out
}

async fn run_inner(
    config: &VncConfig,
    sink: &EventSink,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    mut decisions: Decisions,
    prompt: &PromptSlot,
    mut close: watch::Receiver<bool>,
) -> Result<(), VncError> {
    // The receivers live as long as the session: a late decision is dropped, not an error.
    let established = establish_asking(
        config,
        sink,
        prompt,
        &mut decisions.cert_rx,
        &mut decisions.plain_rx,
        &mut close,
    )
    .await?;
    tracing::debug!(
        host = %config.host,
        name = %established.init.name,
        encrypted = established.encrypted,
        "VNC connected"
    );
    if !sink(FbEvent::Security {
        encrypted: established.encrypted,
        auth: None,
    }) {
        return Ok(());
    }

    // Commands sent while connecting are stale, except the visibility.
    let mut visible = true;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if let Cmd::Visible(v) = cmd {
            visible = v;
        }
    }

    let (width, height) = (established.init.width, established.init.height);
    if !sink(FbEvent::Resize {
        w: width,
        h: height,
    }) {
        return Ok(());
    }
    let image: SharedImage = Arc::new(std::sync::Mutex::new(Image::new(width, height)));
    let (notice_tx, notice_rx) = mpsc::channel(NOTICE_QUEUE);
    let reader = ReaderTask(tokio::spawn(read_loop(
        established.reader,
        image.clone(),
        notice_tx,
    )));

    let mut active = Active {
        image,
        writer: established.writer,
        sink: sink.clone(),
        dirty: DirtyRegion::default(),
        visible,
        last_flush: Instant::now(),
        last_cursor: None,
        input: InputState::default(),
        view_only: config.view_only,
        outstanding: 0,
        resized: false,
        sink_gone: false,
    };
    let result = active.run(notice_rx, &mut cmd_rx, &mut close).await;
    drop(reader);
    result
}

/// Aborts the reader task when the session ends (that closes the socket).
struct ReaderTask(JoinHandle<()>);

impl Drop for ReaderTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum ReaderMsg {
    Notice(Notice),
    Failed(VncError),
}

/// Decodes server messages for as long as the connection lives and reports what changed.
/// Ends with a `Failed` message (a clean close included).
async fn read_loop<R: AsyncRead + Unpin>(
    reader: R,
    image: SharedImage,
    tx: mpsc::Sender<ReaderMsg>,
) {
    let mut reader = BufReader::with_capacity(256 * 1024, reader);
    let mut zrle = ZrleStream::default();
    let mut notices = Vec::new();
    loop {
        notices.clear();
        let result = rfb::read_server_message(&mut reader, &image, &mut zrle, &mut notices).await;
        for notice in notices.drain(..) {
            if tx.send(ReaderMsg::Notice(notice)).await.is_err() {
                return;
            }
        }
        if let Err(e) = result {
            // The session task may be gone already; then nobody needs the reason.
            if tx.send(ReaderMsg::Failed(e)).await.is_err() {
                tracing::trace!("reader stopped after the session ended");
            }
            return;
        }
    }
}

// ---------------------------------------------------------------------------------------
// Active stage
// ---------------------------------------------------------------------------------------

struct Active {
    image: SharedImage,
    writer: WriteHalf<Conn>,
    sink: EventSink,
    dirty: DirtyRegion,
    visible: bool,
    last_flush: Instant,
    last_cursor: Option<FbEvent>,
    input: InputState,
    view_only: bool,
    /// FramebufferUpdateRequests sent that have not been answered yet.
    outstanding: u32,
    /// The desktop was resized by the current update: ask for everything again.
    resized: bool,
    sink_gone: bool,
}

impl Active {
    fn emit(&mut self, event: FbEvent) {
        if !(self.sink)(event) {
            self.sink_gone = true;
        }
    }

    fn size(&self) -> (u16, u16) {
        let img = rfb::lock(&self.image);
        (img.width, img.height)
    }

    fn rect_event(&self, r: Rect) -> Option<FbEvent> {
        let (x, y) = (u16::try_from(r.x).ok()?, u16::try_from(r.y).ok()?);
        let (w, h) = (u16::try_from(r.w).ok()?, u16::try_from(r.h).ok()?);
        let rgba = rfb::lock(&self.image).crop(x, y, w, h)?;
        Some(FbEvent::Rect { x, y, w, h, rgba })
    }

    fn flush(&mut self) {
        self.last_flush = Instant::now();
        if !self.visible {
            self.dirty.clear();
            return;
        }
        let (w, h) = self.size();
        for r in self.dirty.take(w, h) {
            match self.rect_event(r) {
                Some(event) => self.emit(event),
                None => tracing::warn!("dirty rectangle outside the image, skipped"),
            }
            if self.sink_gone {
                return;
            }
        }
    }

    fn flush_deadline(&self) -> Option<Instant> {
        (self.visible && !self.dirty.is_empty()).then(|| self.last_flush + FLUSH_INTERVAL)
    }

    fn snapshot(&self) -> Snapshot {
        let img = rfb::lock(&self.image);
        Snapshot {
            width: img.width,
            height: img.height,
            rgba: img.rgba.clone(),
        }
    }

    fn set_cursor(&mut self, event: FbEvent) {
        if self.visible {
            self.emit(event.clone());
        }
        self.last_cursor = Some(event);
    }

    /// Appends a request for the whole desktop.
    fn request_all(&mut self, out: &mut Vec<u8>) {
        let (w, h) = self.size();
        out.extend_from_slice(&rfb::update_request(false, 0, 0, w, h));
        self.outstanding += 1;
    }

    /// Digests what the reader decoded. Messages to write are appended to `out`.
    fn notice(&mut self, notice: Notice, out: &mut Vec<u8>) {
        match notice {
            Notice::Resized { w, h } => {
                // The image is new (black): everything must be drawn again.
                self.dirty.mark_full();
                self.resized = true;
                self.emit(FbEvent::Resize { w, h });
            }
            Notice::Cursor(event) => self.set_cursor(event),
            Notice::Update(rects) => {
                for r in rects {
                    self.dirty.add(r);
                }
                self.outstanding = self.outstanding.saturating_sub(1);
                if self.visible {
                    if std::mem::take(&mut self.resized) {
                        self.request_all(out);
                    } else if self.outstanding == 0 {
                        let (w, h) = self.size();
                        out.extend_from_slice(&rfb::update_request(true, 0, 0, w, h));
                        self.outstanding += 1;
                    }
                }
            }
        }
    }

    fn release_input(&mut self, out: &mut Vec<u8>) {
        self.input.release_all(out);
    }

    fn set_visible(&mut self, visible: bool, out: &mut Vec<u8>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if visible {
            // The server stopped being asked while hidden: ask for everything, and show the
            // image we have at once without waiting for the answer.
            self.resized = false;
            self.request_all(out);
            self.dirty.mark_full();
            self.flush();
            if let Some(cursor) = self.last_cursor.clone() {
                self.emit(cursor);
            }
        } else {
            self.dirty.clear();
            // Do not leave keys or buttons pressed on the remote side while hidden.
            self.release_input(out);
        }
    }

    /// Handles one command; messages to write are appended to `out`.
    fn command(&mut self, cmd: Cmd, out: &mut Vec<u8>) {
        match cmd {
            Cmd::Pointer {
                x,
                y,
                buttons,
                wheel,
            } => {
                if !self.view_only {
                    let size = self.size();
                    self.input.pointer(x, y, buttons, wheel, size, out);
                }
            }
            Cmd::Key { code, down } => {
                if !self.view_only {
                    self.input.key(code, down, out);
                }
            }
            Cmd::Visible(v) => self.set_visible(v, out),
            Cmd::Snapshot(reply) => {
                // The requester may have given up; that is fine.
                if reply.send(self.snapshot()).is_err() {
                    tracing::trace!("snapshot requester is gone");
                }
            }
        }
    }

    async fn write(
        &mut self,
        out: &[u8],
        close: &mut watch::Receiver<bool>,
    ) -> Result<(), VncError> {
        if out.is_empty() {
            return Ok(());
        }
        let writer = &mut self.writer;
        cancellable(
            close,
            with_timeout("writing to the server", WRITE_TIMEOUT, async {
                writer
                    .write_all(out)
                    .await
                    .map_err(|e| VncError::Io(format!("cannot write to the server: {e}")))?;
                // TLS buffers: nothing leaves before the flush.
                writer
                    .flush()
                    .await
                    .map_err(|e| VncError::Io(format!("cannot write to the server: {e}")))
            }),
        )
        .await
    }

    async fn run(
        &mut self,
        mut notices: mpsc::Receiver<ReaderMsg>,
        cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
        close: &mut watch::Receiver<bool>,
    ) -> Result<(), VncError> {
        // Pixel format and encodings first, then the first full request (a tab that starts
        // hidden asks when it is shown).
        let mut out = rfb::set_pixel_format();
        out.extend_from_slice(&rfb::set_encodings());
        if self.visible {
            self.request_all(&mut out);
        }
        self.write(&out, close).await?;
        loop {
            if self.sink_gone {
                return Ok(());
            }
            let deadline = self.flush_deadline();
            let mut out: Vec<u8> = Vec::new();
            tokio::select! {
                _ = close.changed() => {
                    // Lift what is held, best effort, then drop the connection.
                    self.release_input(&mut out);
                    if let Err(e) = tokio::time::timeout(
                        Duration::from_millis(500),
                        async {
                            self.writer.write_all(&out).await?;
                            self.writer.flush().await
                        },
                    ).await {
                        tracing::trace!("could not release the input on close: {e}");
                    }
                    return Ok(());
                }
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { return Ok(()) };
                    let mut batch = vec![cmd];
                    while batch.len() < COMMAND_BATCH {
                        let Ok(next) = cmd_rx.try_recv() else { break };
                        batch.push(next);
                    }
                    for cmd in coalesce(batch) {
                        self.command(cmd, &mut out);
                    }
                }
                msg = notices.recv() => {
                    match msg {
                        None => return Ok(()),
                        Some(ReaderMsg::Notice(n)) => self.notice(n, &mut out),
                        Some(ReaderMsg::Failed(e)) => {
                            // Show what arrived before the end.
                            self.flush();
                            return Err(e);
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.flush();
                }
            }
            self.write(&out, close).await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use portolano_core_model::ErrorCode;

    use super::*;

    #[test]
    fn coalescing_keeps_the_last_move_and_every_other_command() {
        let moves = |x| Cmd::Pointer {
            x,
            y: 0,
            buttons: 0,
            wheel: 0,
        };
        let out = coalesce(vec![
            moves(1),
            moves(2),
            Cmd::Key {
                code: "KeyA",
                down: true,
            },
            moves(3),
            moves(4),
            Cmd::Pointer {
                x: 9,
                y: 9,
                buttons: 1,
                wheel: 0,
            },
            Cmd::Pointer {
                x: 9,
                y: 9,
                buttons: 1,
                wheel: -120,
            },
            moves(5),
        ]);
        let xs: Vec<u16> = out
            .iter()
            .map(|c| match c {
                Cmd::Pointer { x, .. } => *x,
                _ => 0,
            })
            .collect();
        assert_eq!(xs, vec![2, 0, 4, 9, 9, 5]);
    }

    #[test]
    fn config_debug_does_not_show_the_password() {
        let mut config = VncConfig::new("h", 5900);
        config.password = Some(SecretString::from("hunter2-very-secret".to_owned()));
        assert!(!format!("{config:?}").contains("hunter2"));
    }

    fn answer(password: Option<&str>) -> CredentialAnswer {
        CredentialAnswer {
            username: String::new(),
            password: password.map(|p| SecretString::from(p.to_owned())),
            key_path: None,
            passphrase: None,
            domain: None,
        }
    }

    #[test]
    fn answers_need_a_password() {
        assert_eq!(
            credentials_from_answer(PromptKind::Password, answer(Some("pw")))
                .expect("valid")
                .1
                .expose_secret(),
            "pw"
        );
        assert!(credentials_from_answer(PromptKind::Password, answer(None)).is_err());
        assert!(credentials_from_answer(PromptKind::Password, answer(Some(""))).is_err());
    }

    #[test]
    fn plain_answers_need_a_user_name_and_a_password() {
        let with_user = |user: &str, password: Option<&str>| CredentialAnswer {
            username: user.to_owned(),
            ..answer(password)
        };
        let (user, password) = credentials_from_answer(
            PromptKind::UsernamePassword,
            with_user("  tester ", Some("pw")),
        )
        .expect("valid");
        assert_eq!(user.as_deref(), Some("tester"));
        assert_eq!(password.expose_secret(), "pw");
        for bad in [
            with_user("", Some("pw")),
            with_user("u", None),
            with_user("u", Some("")),
        ] {
            assert!(credentials_from_answer(PromptKind::UsernamePassword, bad).is_err());
        }
    }

    #[test]
    fn only_an_authentication_failure_asks_again_and_only_a_few_times() {
        let auth = VncError::AuthFailed("x".into());
        assert!(asks_again(&auth, 1));
        assert!(asks_again(&auth, MAX_PROMPT_ATTEMPTS - 1));
        assert!(!asks_again(&auth, MAX_PROMPT_ATTEMPTS));
        assert!(!asks_again(&VncError::Io("x".into()), 1));
        assert!(!asks_again(&VncError::HostUnreachable("x".into()), 1));
    }

    fn collecting_sink() -> (EventSink, Arc<Mutex<Vec<FbEvent>>>) {
        let events: Arc<Mutex<Vec<FbEvent>>> = Arc::default();
        let sink_events = events.clone();
        let sink: EventSink = Arc::new(move |event| {
            if let Ok(mut v) = sink_events.lock() {
                v.push(event);
            }
            true
        });
        (sink, events)
    }

    fn ask() -> VncAsk {
        VncAsk {
            hooks: PromptHooks::no_vault(),
        }
    }

    #[tokio::test]
    async fn the_prompt_asks_for_a_password_only_and_the_reply_comes_back() {
        let (sink, events) = collecting_sink();
        let slot = Arc::new(PromptSlot::new());
        let (_close_tx, mut close) = watch::channel(false);
        let ask_spec = ask();
        let config = VncConfig::new("vnc.example", 5901);
        let waiting = {
            let slot = slot.clone();
            async move {
                loop {
                    if slot
                        .reply(PromptReply::Answer {
                            answer: answer(Some("pw-1")),
                            save_to_vault: true,
                        })
                        .is_ok()
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }
        };
        let (asked, ()) = tokio::join!(
            ask_credentials(
                &config,
                &ask_spec,
                &sink,
                &slot,
                &mut close,
                PromptSpec {
                    kind: PromptKind::Password,
                    username_hint: None,
                    error: Some("earlier failure".into())
                }
            ),
            waiting
        );
        let asked = asked.expect("asked");
        assert_eq!(asked.password.expose_secret(), "pw-1");
        assert!(asked.save);
        let events = events.lock().expect("events");
        let [FbEvent::CredentialPrompt(prompt)] = events.as_slice() else {
            panic!("expected exactly one prompt, got {events:?}");
        };
        assert_eq!(prompt.kind, PromptKind::Password);
        assert_eq!(prompt.username_hint, None);
        assert_eq!(prompt.domain_hint, None);
        assert_eq!(prompt.target, "vnc.example:5901");
        assert_eq!(prompt.error.as_deref(), Some("earlier failure"));
        assert!(!prompt.can_save_to_vault);
    }

    #[tokio::test]
    async fn an_empty_answer_is_asked_again_and_cancel_is_distinct() {
        let (sink, events) = collecting_sink();
        let slot = Arc::new(PromptSlot::new());
        let (_close_tx, mut close) = watch::channel(false);
        let ask_spec = ask();
        let config = VncConfig::new("h", 5900);
        let driver = {
            let slot = slot.clone();
            let events = events.clone();
            async move {
                let wait_for = |n: usize| {
                    let events = events.clone();
                    async move {
                        while events.lock().map(|e| e.len()).unwrap_or(0) < n {
                            tokio::task::yield_now().await;
                        }
                    }
                };
                wait_for(1).await;
                slot.reply(PromptReply::Answer {
                    answer: answer(Some("")),
                    save_to_vault: false,
                })
                .expect("first reply");
                wait_for(2).await;
                slot.reply(PromptReply::Cancel).expect("cancel");
            }
        };
        let (result, ()) = tokio::join!(
            ask_credentials(
                &config,
                &ask_spec,
                &sink,
                &slot,
                &mut close,
                PromptSpec {
                    kind: PromptKind::Password,
                    username_hint: None,
                    error: None
                }
            ),
            driver
        );
        assert!(matches!(result, Err(VncError::PromptCancelled)));
        assert_eq!(VncError::PromptCancelled.code(), ErrorCode::Cancelled);
        let events = events.lock().expect("events");
        let [FbEvent::CredentialPrompt(first), FbEvent::CredentialPrompt(second)] =
            events.as_slice()
        else {
            panic!("expected two prompts, got {events:?}");
        };
        assert_eq!(first.error, None);
        assert_eq!(second.error.as_deref(), Some("Enter a password."));
    }

    #[tokio::test]
    async fn closing_the_session_while_asking_is_not_a_cancellation_by_the_user() {
        let (sink, _events) = collecting_sink();
        let slot = PromptSlot::new();
        let (close_tx, mut close) = watch::channel(false);
        close_tx.send_replace(true);
        let ask_spec = ask();
        let result = ask_credentials(
            &VncConfig::new("h", 5900),
            &ask_spec,
            &sink,
            &slot,
            &mut close,
            PromptSpec {
                kind: PromptKind::Password,
                username_hint: None,
                error: None,
            },
        )
        .await;
        assert!(matches!(result, Err(VncError::Cancelled)));
    }
}
