//! One RDP session: connection sequence, certificate trust, active stage and the handle used
//! by the IPC commands.
//!
//! # Design
//!
//! The decoded desktop lives in the backend (`DecodedImage`). The UI only ever receives
//! `FbEvent`s through an [`EventSink`]:
//!
//! - dirty rectangles are collected in a [`DirtyRegion`], merged, and flushed at most every
//!   [`FLUSH_INTERVAL`]; there is no per-PDU message and no queue: whatever changed since the
//!   last flush is read from the current image (latest wins);
//! - while the tab is hidden nothing is sent. The server is told to stop painting when it
//!   supports Suppress Output; decoding goes on either way so the image is always current.
//!   When the tab is shown again one full-image rectangle is sent;
//! - input arrives through an unbounded command channel; consecutive pointer moves are
//!   collapsed before they are encoded.
//!
//! The server certificate is accepted at the TLS layer only to be inspected: its fingerprint
//! is checked against the trust store (and the user is asked) **before** any credential is
//! sent, because credentials only travel in `connect_finalize`.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ironrdp_cliprdr::Cliprdr;
use ironrdp_connector::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationState,
};
use ironrdp_connector::{ClientConnector, Config, Credentials, DesktopSize};
use ironrdp_core::WriteBuf;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_input::{Database, MouseButton, MousePosition, Operation, WheelRotations};
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::rdp::refresh_rectangle::RefreshRectanglePdu;
use ironrdp_pdu::rdp::suppress_output::SuppressOutputPdu;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_svc::{SvcMessage, SvcProcessorMessages};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{single_sequence_step, FramedWrite as _, TokioFramed};
use portolano_core_model::fb::FbEvent;
use portolano_core_model::prompt::{
    CredentialAnswer, CredentialPrompt, PromptHooks, PromptKind, PromptReply, PromptSlot,
    SavedCredential, SavedSecret,
};
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

use crate::cert::{fingerprint_sha256, CertCheck, CertStore};
use crate::clipboard::backend::ForwardBackend;
use crate::clipboard::engine::{self, Engine, Internal};
use crate::clipboard::guard::GuardedCliprdr;
use crate::clipboard::{ClipboardConfig, Command as ClipboardCommand};
use crate::dirty::{DirtyRegion, Rect};
use crate::error::{self, from_connector, RdpError};
use crate::kerberos::{self, TimeoutNetworkClient};
use crate::keymap;

/// Receives the events of the session. Returns `false` when nobody listens any more, which
/// ends the session.
pub type EventSink = Arc<dyn Fn(FbEvent) -> bool + Send + Sync>;

/// Minimum time between two flushes of dirty rectangles (40 Hz at most).
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(25);

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TLS_TIMEOUT: Duration = Duration::from_secs(15);
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(60);
const REACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);
const CERT_DECISION_TIMEOUT: Duration = Duration::from_secs(600);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(600);
/// Commands handled per loop turn before the network is polled again.
const COMMAND_BATCH: usize = 128;

const DEFAULT_KEYBOARD_LAYOUT: u32 = 0x0409;

/// The user may try this many credentials before the connection fails.
const MAX_PROMPT_ATTEMPTS: u32 = 5;

#[derive(Clone)]
pub struct RdpCredentials {
    pub username: String,
    pub password: SecretString,
}

/// The session asks the user for its credentials (WP11): used when the connection has none.
#[derive(Clone)]
pub struct RdpAsk {
    /// Pre-filled user name (the connection's `username`).
    pub username_hint: Option<String>,
    pub hooks: PromptHooks,
}

/// Everything a session needs, resolved by the caller (the IPC layer reads the connection
/// document and the vault).
#[derive(Clone)]
pub struct RdpConfig {
    pub host: String,
    pub port: u16,
    /// Desktop size to request. Rounded to an even width and clamped to 200..=8192.
    pub width: u16,
    pub height: u16,
    /// With a fixed size, `resize` requests are ignored (the connection sets width/height).
    pub fixed_size: bool,
    /// `None`: no credential; NLA is not attempted and the server's own login screen is used.
    pub credentials: Option<RdpCredentials>,
    /// With no `credentials`: ask the user before connecting (and again when they fail).
    /// Without it, no credential means the server's own login screen.
    pub ask: Option<RdpAsk>,
    pub domain: Option<String>,
    /// Skip the certificate check (the connection's `ignoreCert` option).
    pub ignore_cert: bool,
    /// Accept a server that only offers TLS-level security (no NLA) even though a credential
    /// is sent. Off by default: with a credential only CredSSP (NLA) is offered, so an
    /// attacker cannot downgrade the session and receive the password in the Client Info PDU.
    pub allow_without_nla: bool,
    /// The connection's optional "Kerberos KDC", as typed (see [`crate::kerberos::normalize_kdc`]).
    /// Used only when the sign-in uses Kerberos; `None`: the KDC is found through DNS.
    pub kerberos_kdc: Option<String>,
    pub cert_store: CertStore,
    pub keyboard_layout: u32,
    /// Share the clipboard with the remote desktop. `None`: the clipboard channel is not
    /// negotiated at all.
    pub clipboard: Option<ClipboardConfig>,
}

impl RdpConfig {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        width: u16,
        height: u16,
        cert_store: CertStore,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            width,
            height,
            fixed_size: false,
            credentials: None,
            ask: None,
            domain: None,
            ignore_cert: false,
            allow_without_nla: false,
            kerberos_kdc: None,
            cert_store,
            keyboard_layout: DEFAULT_KEYBOARD_LAYOUT,
            clipboard: None,
        }
    }
}

impl std::fmt::Debug for RdpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RdpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("size", &(self.width, self.height))
            .field("credentials", &self.credentials.is_some())
            .field("ask", &self.ask.is_some())
            .field("ignore_cert", &self.ignore_cert)
            .field("allow_without_nla", &self.allow_without_nla)
            .field("clipboard", &self.clipboard.is_some())
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
    Resize {
        width: u16,
        height: u16,
    },
    Visible(bool),
    Snapshot(oneshot::Sender<Snapshot>),
    Clipboard(ClipboardCommand),
}

/// Handle of a running session. Dropping it closes the session.
pub struct RdpSession {
    cmd: mpsc::UnboundedSender<Cmd>,
    cert: mpsc::UnboundedSender<bool>,
    prompt: Arc<PromptSlot>,
    close: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}

fn closed_error() -> RdpError {
    RdpError::Io("the session is closed".into())
}

impl RdpSession {
    /// Starts the session on the current tokio runtime. Progress, certificate prompts and
    /// failures are reported through `sink`; this never blocks.
    pub fn start(config: RdpConfig, sink: EventSink) -> Self {
        let (cmd, cmd_rx) = mpsc::unbounded_channel();
        let (cert, cert_rx) = mpsc::unbounded_channel();
        let (close, close_rx) = watch::channel(false);
        let (done_tx, done) = watch::channel(false);
        let prompt = Arc::new(PromptSlot::new());
        let task_prompt = prompt.clone();
        tokio::spawn(async move {
            run(config, sink, cmd_rx, cert_rx, task_prompt, close_rx).await;
            // Receivers may all be gone already; nothing to report then.
            done_tx.send_replace(true);
        });
        Self {
            cmd,
            cert,
            prompt,
            close,
            done,
        }
    }

    fn send(&self, cmd: Cmd) -> Result<(), RdpError> {
        self.cmd.send(cmd).map_err(|_| closed_error())
    }

    /// `buttons` uses the DOM `MouseEvent.buttons` bits. `wheel` is in 120-unit notches,
    /// positive scrolls down.
    pub fn pointer(&self, x: u16, y: u16, buttons: u8, wheel: i16) -> Result<(), RdpError> {
        self.send(Cmd::Pointer {
            x,
            y,
            buttons,
            wheel,
        })
    }

    /// `code` is `KeyboardEvent.code`. Keys without an RDP scancode are ignored.
    pub fn key(&self, code: &str, down: bool) -> Result<(), RdpError> {
        match keymap::TABLE.iter().find(|(name, _)| *name == code) {
            Some((name, _)) => self.send(Cmd::Key { code: name, down }),
            None => {
                tracing::debug!(code, "ignoring a key without an RDP scancode");
                Ok(())
            }
        }
    }

    /// Asks the server for a new desktop size (Display Control). A no-op when the size is fixed
    /// or the server does not support it; the UI then scales the image.
    pub fn resize(&self, width: u16, height: u16) -> Result<(), RdpError> {
        self.send(Cmd::Resize { width, height })
    }

    /// Hidden: nothing is sent. Visible: one full redraw.
    pub fn set_visible(&self, visible: bool) -> Result<(), RdpError> {
        self.send(Cmd::Visible(visible))
    }

    /// Saves the files on the remote clipboard below `dir`, a folder the user chose. Progress
    /// and the outcome arrive as `FbEvent::Clipboard`. A no-op when clipboard sharing is off.
    pub fn clipboard_save(&self, dir: std::path::PathBuf) -> Result<(), RdpError> {
        self.send(Cmd::Clipboard(ClipboardCommand::SaveTo(dir)))
    }

    /// Stops the save in progress.
    pub fn clipboard_cancel(&self) -> Result<(), RdpError> {
        self.send(Cmd::Clipboard(ClipboardCommand::Cancel))
    }

    /// Answer to a `CertPrompt`.
    pub fn cert_decision(&self, accept: bool) -> Result<(), RdpError> {
        self.cert.send(accept).map_err(|_| closed_error())
    }

    /// Answer to a `CredentialPrompt`. The secrets are used for this session only.
    pub fn credential(
        &self,
        answer: CredentialAnswer,
        save_to_vault: bool,
    ) -> Result<(), RdpError> {
        self.prompt
            .reply(PromptReply::Answer {
                answer,
                save_to_vault,
            })
            .map_err(|e| RdpError::InvalidInput(e.message))
    }

    /// The user dismissed the `CredentialPrompt`: the session ends with `cancelled`.
    pub fn credential_cancel(&self) -> Result<(), RdpError> {
        self.prompt
            .reply(PromptReply::Cancel)
            .map_err(|e| RdpError::InvalidInput(e.message))
    }

    /// Asks the session to end. Does not block; see [`RdpSession::closed`].
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

impl Drop for RdpSession {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------------------

enum End {
    /// Closed from our side (or the listener went away).
    Local,
    /// The server ended the session.
    Server(String),
    /// The user declined the certificate.
    CertDeclined,
}

type Framed = TokioFramed<crate::tls::TlsStream<TcpStream>>;

async fn run(
    config: RdpConfig,
    sink: EventSink,
    cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    cert_rx: mpsc::UnboundedReceiver<bool>,
    prompt: Arc<PromptSlot>,
    close_rx: watch::Receiver<bool>,
) {
    let result = run_inner(&config, &sink, cmd_rx, cert_rx, &prompt, close_rx).await;
    // `false` from the sink only means nobody is listening: nothing left to do.
    let delivered = match result {
        // A changed certificate was reported as a prompt already: nothing more to say.
        Ok(End::Local) | Err(RdpError::Cancelled | RdpError::CertificateChanged) => true,
        Ok(End::Server(reason)) => sink(FbEvent::Closed { reason }),
        Ok(End::CertDeclined) => sink(FbEvent::Closed {
            reason: "the server certificate was not accepted".into(),
        }),
        Err(e) => {
            if !matches!(e, RdpError::PromptCancelled) {
                tracing::warn!(host = %config.host, code = e.code().as_str(), "RDP session failed: {e}");
            }
            sink(FbEvent::Error(e.into()))
        }
    };
    if !delivered {
        tracing::debug!("RDP session ended and nobody was listening");
    }
}

/// Runs `fut` unless the session is closed first.
async fn cancellable<T>(
    close: &mut watch::Receiver<bool>,
    fut: impl Future<Output = Result<T, RdpError>>,
) -> Result<T, RdpError> {
    if *close.borrow() {
        return Err(RdpError::Cancelled);
    }
    tokio::select! {
        biased;
        _ = close.changed() => Err(RdpError::Cancelled),
        r = fut => r,
    }
}

async fn with_timeout<T>(
    what: &str,
    limit: Duration,
    fut: impl Future<Output = Result<T, RdpError>>,
) -> Result<T, RdpError> {
    match tokio::time::timeout(limit, fut).await {
        Ok(r) => r,
        Err(_) => Err(RdpError::Io(format!("{what} timed out"))),
    }
}

/// Whether plain TLS-level security may be selected by the server. With a credential that is
/// only so when the connection explicitly allows it (see [`RdpConfig::allow_without_nla`]);
/// without one nothing secret is sent and the server's own login screen takes over.
fn tls_enabled(c: &RdpConfig) -> bool {
    c.credentials.is_none() || c.allow_without_nla
}

fn build_config(c: &RdpConfig, width: u16, height: u16) -> Result<Config, RdpError> {
    let codecs = ironrdp_pdu::rdp::capability_sets::client_codecs_capabilities(&[])
        .map_err(|e| RdpError::Io(format!("cannot build the codec capabilities: {e}")))?;
    let (credentials, autologon) = match &c.credentials {
        Some(cred) => (
            Credentials::UsernamePassword {
                username: cred.username.clone(),
                password: cred.password.expose_secret().to_owned(),
            },
            true,
        ),
        None => (
            Credentials::UsernamePassword {
                username: String::new(),
                password: String::new(),
            },
            false,
        ),
    };
    Ok(Config {
        credentials,
        domain: c.domain.clone().filter(|d| !d.is_empty()),
        enable_tls: tls_enabled(c),
        // NLA is requested whenever there is a credential to authenticate with.
        enable_credssp: c.credentials.is_some(),
        enable_standard_rdp_security: false,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: c.keyboard_layout,
        keyboard_functional_keys_count: 12,
        connection_type: ironrdp_pdu::gcc::ConnectionType::Lan,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize { width, height },
        monitor_layout: None,
        bitmap: Some(ironrdp_connector::BitmapConfig {
            lossy_compression: true,
            color_depth: 32,
            codecs,
        }),
        client_build: 0,
        client_name: "portolano".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        platform: ironrdp_pdu::rdp::capability_sets::MajorPlatformType::MACINTOSH,
        // Server pointer shapes are forwarded as `FbEvent::Cursor`, not drawn into the image.
        enable_server_pointer: true,
        request_data: None,
        autologon,
        enable_audio_playback: false,
        enable_audio_capture: false,
        compression_type: None,
        pointer_software_rendering: false,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
        performance_flags: ironrdp_pdu::rdp::client_info::PerformanceFlags::default(),
        desktop_scale_factor: 0,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        hardware_id: None,
        license_cache: None,
        timezone_info: ironrdp_pdu::rdp::client_info::TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::empty(),
    })
}

fn clamp_size(width: u16, height: u16) -> (u16, u16) {
    let (w, h) = MonitorLayoutEntry::adjust_display_size(u32::from(width), u32::from(height));
    (
        u16::try_from(w).unwrap_or(u16::MAX),
        u16::try_from(h).unwrap_or(u16::MAX),
    )
}

async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, RdpError> {
    let stream =
        match tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect((host, port))).await {
            Err(_) => {
                return Err(RdpError::HostUnreachable(format!(
                    "connecting to {host}:{port} timed out"
                )))
            }
            Ok(Err(e)) => {
                return Err(RdpError::HostUnreachable(format!(
                    "cannot connect to {host}:{port}: {e}"
                )))
            }
            Ok(Ok(stream)) => stream,
        };
    stream
        .set_nodelay(true)
        .map_err(|e| RdpError::Io(format!("cannot configure the socket: {e}")))?;
    Ok(stream)
}

/// Compares the presented certificate with the trust store and asks the user when needed.
async fn check_certificate(
    config: &RdpConfig,
    sink: &EventSink,
    cert_rx: &mut mpsc::UnboundedReceiver<bool>,
    fingerprint: &str,
) -> Result<bool, RdpError> {
    if config.ignore_cert {
        return Ok(true);
    }
    let check = config
        .cert_store
        .check(&config.host, config.port, fingerprint)
        .map_err(|e| RdpError::Io(format!("cannot read the trusted certificates: {e}")))?;
    match check {
        CertCheck::Matches => return Ok(true),
        CertCheck::Unknown => {}
        // Like a changed SSH host key: no way to carry on from here. The user has to remove
        // the stored certificate on purpose and connect again.
        CertCheck::Changed { stored } => {
            let delivered = sink(FbEvent::CertPrompt {
                fingerprint: fingerprint.to_owned(),
                changed: true,
                stored: Some(stored),
            });
            return Err(if delivered {
                RdpError::CertificateChanged
            } else {
                RdpError::Cancelled
            });
        }
    }
    // A decision sent while no prompt was pending must not answer this one.
    while cert_rx.try_recv().is_ok() {}
    if !sink(FbEvent::CertPrompt {
        fingerprint: fingerprint.to_owned(),
        changed: false,
        stored: None,
    }) {
        return Err(RdpError::Cancelled);
    }
    match tokio::time::timeout(CERT_DECISION_TIMEOUT, cert_rx.recv()).await {
        Ok(Some(true)) => {
            config
                .cert_store
                .remember(&config.host, config.port, fingerprint)
                .map_err(|e| RdpError::Io(format!("cannot store the certificate: {e}")))?;
            Ok(true)
        }
        Ok(Some(false)) => Ok(false),
        Ok(None) => Err(RdpError::Cancelled),
        Err(_) => Err(RdpError::Io("no answer to the certificate prompt".into())),
    }
}

/// The queue of the clipboard channel of one connection attempt.
struct ClipboardLink {
    tx: mpsc::UnboundedSender<Internal>,
    rx: mpsc::UnboundedReceiver<Internal>,
}

struct Established {
    framed: Framed,
    result: ironrdp_connector::ConnectionResult,
    clipboard: Option<ClipboardLink>,
}

async fn establish(
    config: &RdpConfig,
    sink: &EventSink,
    cert_rx: &mut mpsc::UnboundedReceiver<bool>,
    close: &mut watch::Receiver<bool>,
    width: u16,
    height: u16,
) -> Result<Option<Established>, RdpError> {
    match establish_once(config, sink, cert_rx, close, width, height, true).await {
        // The stock network client aborts the login when the KDC cannot be reached, instead of
        // letting sspi fall back to NTLM. The fallback is done here: one new connection with
        // NTLM only, which is what Portolano did before it knew Kerberos. Its own failure is
        // reported as it is (a wrong password still asks again); only a network-class failure
        // is blamed on the domain controller, the likelier cause.
        Err(RdpError::KdcUnreachable) => {
            tracing::info!("the Kerberos KDC could not be reached, trying NTLM");
            let retry = establish_once(config, sink, cert_rx, close, width, height, false).await;
            retry.map_err(after_ntlm_retry)
        }
        other => other,
    }
}

const KDC_UNREACHABLE_NTLM_REFUSED: &str = "Kerberos: the domain controller could not be reached, and the server refused the fallback sign-in. Check the Kerberos KDC setting and the password";

/// The error to report when the NTLM retry, made after the KDC was unreachable, failed too.
/// Only a host or I/O failure is attributed to the unreachable KDC; authentication problems, an
/// unsupported server, a changed certificate or a cancellation are reported unchanged. An
/// authentication failure keeps its kind (the user can be asked again) but says that the KDC
/// was unreachable, which is the likelier real cause where NTLM is restricted.
fn after_ntlm_retry(error: RdpError) -> RdpError {
    match error {
        RdpError::AuthFailed(_) => RdpError::AuthFailed(KDC_UNREACHABLE_NTLM_REFUSED.into()),
        RdpError::HostUnreachable(_) | RdpError::Io(_) => RdpError::KdcUnreachable,
        other => other,
    }
}

/// One connection attempt. `allow_kerberos: false` forces NTLM.
async fn establish_once(
    config: &RdpConfig,
    sink: &EventSink,
    cert_rx: &mut mpsc::UnboundedReceiver<bool>,
    close: &mut watch::Receiver<bool>,
    width: u16,
    height: u16,
    allow_kerberos: bool,
) -> Result<Option<Established>, RdpError> {
    // Decided (and an invalid KDC reported) before anything goes on the network.
    let kerberos = match (&config.credentials, allow_kerberos) {
        (Some(cred), true) => kerberos::config_for(
            &config.host,
            &cred.username,
            config.domain.as_deref(),
            true,
            config.kerberos_kdc.as_deref(),
        )?,
        _ => None,
    };
    let tcp = cancellable(close, connect_tcp(&config.host, config.port)).await?;
    let client_addr: SocketAddr = tcp
        .local_addr()
        .map_err(|e| RdpError::Io(format!("cannot read the local address: {e}")))?;
    let mut framed = TokioFramed::new(tcp);
    let mut connector = ClientConnector::new(build_config(config, width, height)?, client_addr);
    connector.attach_static_channel(
        DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
    );
    // The clipboard channel exists only when the connection asked for it.
    let clipboard = config.clipboard.as_ref().map(|_| {
        let (tx, rx) = mpsc::unbounded_channel();
        connector.attach_static_channel(GuardedCliprdr::new(Cliprdr::new(Box::new(
            ForwardBackend::new(tx.clone()),
        ))));
        ClipboardLink { tx, rx }
    });
    let nla = error::NlaPolicy::of(config.credentials.is_some(), config.allow_without_nla);

    let should_upgrade = cancellable(
        close,
        with_timeout("RDP negotiation", TLS_TIMEOUT, async {
            ironrdp_tokio::connect_begin(&mut framed, &mut connector)
                .await
                .map_err(|e| from_connector(&e, nla))
        }),
    )
    .await?;

    let initial = framed.into_inner_no_leftover();
    // The certificate is inspected right below, before anything secret is sent. The handshake
    // signatures are verified, so the server really owns the certificate it presents.
    let crate::tls::Upgraded {
        stream: tls,
        certificate,
    } = cancellable(
        close,
        with_timeout("TLS handshake", TLS_TIMEOUT, async {
            crate::tls::upgrade(initial, &config.host)
                .await
                .map_err(|e| RdpError::Io(format!("TLS handshake failed: {e}")))
        }),
    )
    .await?;

    let fingerprint = fingerprint_sha256(&certificate);
    let accepted = cancellable(
        close,
        check_certificate(config, sink, cert_rx, &fingerprint),
    )
    .await?;
    if !accepted {
        return Ok(None);
    }

    let server_public_key = crate::tls::server_public_key(&certificate)
        .ok_or_else(|| RdpError::Io("the server certificate has no public key".into()))?;
    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
    let mut framed = TokioFramed::new(tls);
    let mut network_client =
        TimeoutNetworkClient::new(ReqwestNetworkClient::new(), kerberos::KDC_REQUEST_TIMEOUT);
    let outcome = cancellable(
        close,
        with_timeout("RDP login", FINALIZE_TIMEOUT, async {
            ironrdp_tokio::connect_finalize(
                upgraded,
                connector,
                &mut framed,
                &mut network_client,
                config.host.clone().into(),
                server_public_key,
                kerberos,
            )
            .await
            .map_err(|e| from_connector(&e, nla))
        }),
    )
    .await?;
    let result = outcome
        .into_connected()
        .map_err(|e| from_connector(&e, nla))?;
    Ok(Some(Established {
        framed,
        result,
        clipboard,
    }))
}

struct Asked {
    credentials: RdpCredentials,
    domain: Option<String>,
    save: bool,
}

/// Emits a credential prompt and waits for the user's reply. A cancellation is
/// `RdpError::PromptCancelled`; closing the session meanwhile is `RdpError::Cancelled`.
#[allow(clippy::too_many_arguments)]
async fn ask_credentials(
    config: &RdpConfig,
    ask: &RdpAsk,
    sink: &EventSink,
    slot: &PromptSlot,
    close: &mut watch::Receiver<bool>,
    username_hint: Option<&str>,
    domain_hint: Option<&str>,
    mut error: Option<String>,
) -> Result<Asked, RdpError> {
    loop {
        let reply = slot.arm();
        let hint = username_hint.map(str::trim).filter(|h| !h.is_empty());
        let prompt = CredentialPrompt {
            kind: PromptKind::UsernamePassword,
            username_hint: hint.map(str::to_owned),
            domain_hint: domain_hint
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_owned),
            target: format!("{}:{}", config.host, config.port),
            error: error.take(),
            can_save_to_vault: (ask.hooks.can_save)(),
        };
        if !sink(FbEvent::CredentialPrompt(prompt)) {
            return Err(RdpError::Cancelled);
        }
        let reply = cancellable(close, async {
            reply.await.map_err(|_| RdpError::Cancelled)
        })
        .await?;
        let (answer, save) = match reply {
            PromptReply::Answer {
                answer,
                save_to_vault,
            } => (answer, save_to_vault),
            PromptReply::Cancel => return Err(RdpError::PromptCancelled),
        };
        match credentials_from_answer(answer) {
            Ok((credentials, domain)) => {
                return Ok(Asked {
                    credentials,
                    domain,
                    save,
                })
            }
            Err(message) => error = Some(message),
        }
    }
}

/// Validates a typed answer. `Err` is a message for the next prompt.
fn credentials_from_answer(
    answer: CredentialAnswer,
) -> Result<(RdpCredentials, Option<String>), String> {
    let username = answer.username.trim().to_owned();
    if username.is_empty() {
        return Err("Enter a user name.".to_owned());
    }
    let password = match answer.password {
        Some(p) if !p.expose_secret().is_empty() => p,
        _ => return Err("Enter a password.".to_owned()),
    };
    let domain = answer
        .domain
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty());
    Ok((RdpCredentials { username, password }, domain))
}

/// Whether a failed attempt with typed credentials is followed by a new prompt.
fn asks_again(error: &RdpError, attempts: u32) -> bool {
    matches!(error, RdpError::AuthFailed(_)) && attempts < MAX_PROMPT_ATTEMPTS
}

/// Connects with credentials typed by the user. With NLA a wrong credential fails the whole
/// connection, so each retry is a new connection (the certificate is already trusted by then).
#[allow(clippy::too_many_arguments)]
async fn establish_asking(
    config: &RdpConfig,
    ask: &RdpAsk,
    sink: &EventSink,
    slot: &PromptSlot,
    cert_rx: &mut mpsc::UnboundedReceiver<bool>,
    close: &mut watch::Receiver<bool>,
    width: u16,
    height: u16,
) -> Result<Option<Established>, RdpError> {
    let mut username = ask.username_hint.clone();
    let mut domain = config.domain.clone();
    let mut error: Option<String> = None;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let asked = ask_credentials(
            config,
            ask,
            sink,
            slot,
            close,
            username.as_deref(),
            domain.as_deref(),
            error.take(),
        )
        .await?;
        username = Some(asked.credentials.username.clone());
        domain = asked.domain.clone();
        let mut attempt = config.clone();
        attempt.credentials = Some(asked.credentials.clone());
        attempt.domain = asked.domain.clone();
        match establish(&attempt, sink, cert_rx, close, width, height).await {
            Ok(established) => {
                if established.is_some() && asked.save {
                    offer_to_save(ask, asked).await;
                }
                return Ok(established);
            }
            Err(e) if asks_again(&e, attempts) => error = Some(e.to_string()),
            Err(e) => return Err(e),
        }
    }
}

/// Hands the credential the user chose to keep to the host application, off the async workers,
/// and waits for it: the desktop only starts after the credential is saved, so the UI can
/// re-read the document as soon as the session shows signs of life.
async fn offer_to_save(ask: &RdpAsk, asked: Asked) {
    let saved = ask.hooks.saved.clone();
    let credential = SavedCredential {
        username: asked.credentials.username,
        domain: asked.domain,
        secret: SavedSecret::Password(asked.credentials.password),
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
    config: &RdpConfig,
    sink: &EventSink,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    mut cert_rx: mpsc::UnboundedReceiver<bool>,
    prompt: &PromptSlot,
    mut close: watch::Receiver<bool>,
) -> Result<End, RdpError> {
    let (width, height) = clamp_size(config.width, config.height);
    let established = match (&config.ask, &config.credentials) {
        (Some(ask), None) => {
            establish_asking(
                config,
                ask,
                sink,
                prompt,
                &mut cert_rx,
                &mut close,
                width,
                height,
            )
            .await?
        }
        _ => establish(config, sink, &mut cert_rx, &mut close, width, height).await?,
    };
    let Some(established) = established else {
        return Ok(End::CertDeclined);
    };
    let Established {
        framed,
        result,
        clipboard,
    } = established;
    let (clip_tx, mut clip_rx) = match clipboard {
        Some(link) => (Some(link.tx), Some(link.rx)),
        None => (None, None),
    };

    // Commands sent while connecting are stale, except the visibility.
    let mut visible = true;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if let Cmd::Visible(v) = cmd {
            visible = v;
        }
    }

    let (w, h) = (result.desktop_size.width, result.desktop_size.height);
    if !sink(FbEvent::Resize { w, h }) {
        return Ok(End::Local);
    }
    let mut active = Active::new(config, sink.clone(), result, visible, clip_tx);
    active
        .run(framed, &mut cmd_rx, &mut clip_rx, &mut close)
        .await
}

// ---------------------------------------------------------------------------------------
// Active stage
// ---------------------------------------------------------------------------------------

struct Active {
    stage: ActiveStage,
    image: DecodedImage,
    db: Database,
    sink: EventSink,
    dirty: DirtyRegion,
    visible: bool,
    /// The server was told to stop painting (Suppress Output).
    suppressed: bool,
    last_flush: Instant,
    last_cursor: Option<FbEvent>,
    refresh_rect_support: bool,
    suppress_output_support: bool,
    fixed_size: bool,
    pending_resize: Option<(u16, u16)>,
    activation: ConnectionActivationFactory,
    enable_server_pointer: bool,
    needs_reactivation: bool,
    end: Option<End>,
    sink_gone: bool,
    /// The shared clipboard, when the connection asked for it.
    clip: Option<Engine>,
}

/// Waits for the next clipboard event; never completes when the clipboard is not shared.
async fn next_clipboard_event(
    rx: &mut Option<mpsc::UnboundedReceiver<Internal>>,
) -> Option<Internal> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

impl Active {
    fn new(
        config: &RdpConfig,
        sink: EventSink,
        r: ironrdp_connector::ConnectionResult,
        visible: bool,
        clip_tx: Option<mpsc::UnboundedSender<Internal>>,
    ) -> Self {
        let clip = match (&config.clipboard, clip_tx) {
            (Some(shared), Some(tx)) => {
                Some(Engine::new(shared.local.clone(), sink.clone(), tx, visible))
            }
            _ => None,
        };
        let image = DecodedImage::new(
            PixelFormat::RgbA32,
            r.desktop_size.width,
            r.desktop_size.height,
        );
        let mut stage = ActiveStageBuilder {
            static_channels: r.static_channels,
            user_channel_id: r.user_channel_id,
            io_channel_id: r.io_channel_id,
            message_channel_id: r.message_channel_id,
            share_id: r.share_id,
            compression_type: r.compression_type,
            enable_server_pointer: r.enable_server_pointer,
            pointer_software_rendering: r.pointer_software_rendering,
        }
        .build();
        stage.set_window_support_level(r.window_support_level);
        Self {
            stage,
            image,
            db: Database::new(),
            sink,
            dirty: DirtyRegion::default(),
            visible,
            suppressed: false,
            last_flush: Instant::now(),
            last_cursor: None,
            refresh_rect_support: r.refresh_rect_support,
            suppress_output_support: r.suppress_output_support,
            fixed_size: config.fixed_size,
            pending_resize: None,
            activation: r.activation_factory,
            enable_server_pointer: r.enable_server_pointer,
            needs_reactivation: false,
            end: None,
            sink_gone: false,
            clip,
        }
    }

    fn emit(&mut self, event: FbEvent) {
        if !(self.sink)(event) {
            self.sink_gone = true;
        }
    }

    fn rect_event(&self, r: Rect) -> Option<FbEvent> {
        let (x, y, w, h) = (r.x as usize, r.y as usize, r.w as usize, r.h as usize);
        let stride = self.image.stride();
        let data = self.image.data();
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let start = (y + row) * stride + x * 4;
            rgba.extend_from_slice(data.get(start..start + w * 4)?);
        }
        Some(FbEvent::Rect {
            x: u16::try_from(r.x).ok()?,
            y: u16::try_from(r.y).ok()?,
            w: u16::try_from(r.w).ok()?,
            h: u16::try_from(r.h).ok()?,
            rgba,
        })
    }

    fn flush(&mut self) {
        self.last_flush = Instant::now();
        if !self.visible {
            self.dirty.clear();
            return;
        }
        let rects = self.dirty.take(self.image.width(), self.image.height());
        for r in rects {
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
        let (w, h) = (
            usize::from(self.image.width()),
            usize::from(self.image.height()),
        );
        let stride = self.image.stride();
        let data = self.image.data();
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            if let Some(line) = data.get(row * stride..row * stride + w * 4) {
                rgba.extend_from_slice(line);
            }
        }
        Snapshot {
            width: self.image.width(),
            height: self.image.height(),
            rgba,
        }
    }

    fn cursor_event(pointer: &DecodedPointer) -> Option<FbEvent> {
        let expected = usize::from(pointer.width) * usize::from(pointer.height) * 4;
        (pointer.bitmap_data.len() == expected).then(|| FbEvent::Cursor {
            hotspot_x: pointer.hotspot_x,
            hotspot_y: pointer.hotspot_y,
            w: pointer.width,
            h: pointer.height,
            rgba: pointer.bitmap_data.clone(),
        })
    }

    fn set_cursor(&mut self, event: FbEvent) {
        if self.visible {
            self.emit(event.clone());
        }
        self.last_cursor = Some(event);
    }

    /// Frames to write after the outputs were digested; flags are set on `self`.
    fn digest(&mut self, outputs: Vec<ActiveStageOutput>) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        for output in outputs {
            match output {
                ActiveStageOutput::ResponseFrame(frame) if !frame.is_empty() => frames.push(frame),
                ActiveStageOutput::GraphicsUpdate(rect) => {
                    self.dirty.add(Rect::new(
                        u32::from(rect.left),
                        u32::from(rect.top),
                        u32::from(rect.width()),
                        u32::from(rect.height()),
                    ));
                }
                ActiveStageOutput::PointerBitmap(pointer) => {
                    if let Some(event) = Self::cursor_event(&pointer) {
                        self.set_cursor(event);
                    }
                }
                // An empty cursor means "use the default (local) cursor".
                ActiveStageOutput::PointerDefault => self.set_cursor(FbEvent::Cursor {
                    hotspot_x: 0,
                    hotspot_y: 0,
                    w: 0,
                    h: 0,
                    rgba: Vec::new(),
                }),
                // A fully transparent 1x1 cursor hides the pointer.
                ActiveStageOutput::PointerHidden => self.set_cursor(FbEvent::Cursor {
                    hotspot_x: 0,
                    hotspot_y: 0,
                    w: 1,
                    h: 1,
                    rgba: vec![0, 0, 0, 0],
                }),
                ActiveStageOutput::Terminate(reason) => {
                    self.end = Some(End::Server(reason.description()));
                }
                ActiveStageOutput::DeactivateAll => self.needs_reactivation = true,
                ActiveStageOutput::ServerRedirection(_) => {
                    self.end = Some(End::Server("the server asked for a redirection".into()));
                }
                // Responses, pointer positions, session info, auto-detect, multitransport and
                // windowing updates need nothing from a plain desktop client.
                _ => {}
            }
        }
        frames
    }

    fn input_frames(&mut self, events: &[FastPathInputEvent]) -> Result<Vec<Vec<u8>>, RdpError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let outputs = self
            .stage
            .process_fastpath_input(&mut self.image, events)
            .map_err(|e| RdpError::Io(format!("cannot encode the input: {e}")))?;
        Ok(self.digest(outputs))
    }

    fn pointer_ops(&self, x: u16, y: u16, buttons: u8, wheel: i16) -> Vec<Operation> {
        let x = x.min(self.image.width().saturating_sub(1));
        let y = y.min(self.image.height().saturating_sub(1));
        let mut ops = vec![Operation::MouseMove(MousePosition { x, y })];
        const BUTTONS: [(u8, MouseButton); 5] = [
            (1, MouseButton::Left),
            (2, MouseButton::Right),
            (4, MouseButton::Middle),
            (8, MouseButton::X1),
            (16, MouseButton::X2),
        ];
        for (bit, button) in BUTTONS {
            // The input database ignores transitions that change nothing.
            ops.push(if buttons & bit != 0 {
                Operation::MouseButtonPressed(button)
            } else {
                Operation::MouseButtonReleased(button)
            });
        }
        if wheel != 0 {
            // RDP rotation units are positive away from the user (scroll up).
            ops.push(Operation::WheelRotations(WheelRotations {
                is_vertical: true,
                rotation_units: wheel.saturating_neg(),
            }));
        }
        ops
    }

    fn suppress_frame(&self, allow: bool) -> Result<Vec<u8>, RdpError> {
        let rect = allow.then(|| InclusiveRectangle {
            left: 0,
            top: 0,
            right: self.image.width().saturating_sub(1),
            bottom: self.image.height().saturating_sub(1),
        });
        let mut buf = WriteBuf::new();
        self.stage
            .encode_static(
                &mut buf,
                ShareDataPdu::SuppressOutput(SuppressOutputPdu { desktop_rect: rect }),
            )
            .map_err(|e| RdpError::Io(format!("cannot encode Suppress Output: {e}")))?;
        Ok(buf.into_inner())
    }

    fn refresh_frame(&self) -> Result<Vec<u8>, RdpError> {
        let mut buf = WriteBuf::new();
        self.stage
            .encode_static(
                &mut buf,
                ShareDataPdu::RefreshRectangle(RefreshRectanglePdu {
                    areas_to_refresh: vec![InclusiveRectangle {
                        left: 0,
                        top: 0,
                        right: self.image.width().saturating_sub(1),
                        bottom: self.image.height().saturating_sub(1),
                    }],
                }),
            )
            .map_err(|e| RdpError::Io(format!("cannot encode Refresh Rect: {e}")))?;
        Ok(buf.into_inner())
    }

    fn set_visible(&mut self, visible: bool) -> Result<Vec<Vec<u8>>, RdpError> {
        let mut frames = Vec::new();
        if visible == self.visible {
            return Ok(frames);
        }
        self.visible = visible;
        if let Some(clip) = self.clip.as_mut() {
            clip.set_visible(visible);
        }
        if visible {
            if self.suppressed {
                frames.push(self.suppress_frame(true)?);
                if self.refresh_rect_support {
                    frames.push(self.refresh_frame()?);
                }
                self.suppressed = false;
            }
            // The image is current: show it all at once without waiting for the server.
            self.dirty.mark_full();
            self.flush();
            if let Some(cursor) = self.last_cursor.clone() {
                self.emit(cursor);
            }
        } else {
            self.dirty.clear();
            // Do not leave modifiers or buttons pressed on the remote side while hidden.
            let release = self.db.release_all();
            frames.extend(self.input_frames(&release)?);
            if self.suppress_output_support && !self.suppressed {
                frames.push(self.suppress_frame(false)?);
                self.suppressed = true;
            }
        }
        Ok(frames)
    }

    fn resize_frame(&mut self, width: u16, height: u16) -> Result<Option<Vec<u8>>, RdpError> {
        if self.fixed_size {
            return Ok(None);
        }
        let (w, h) = clamp_size(width, height);
        if (w, h) == (self.image.width(), self.image.height()) {
            self.pending_resize = None;
            return Ok(None);
        }
        match self.stage.display_control_ready() {
            None => Ok(None),
            Some(false) => {
                self.pending_resize = Some((w, h));
                Ok(None)
            }
            Some(true) => {
                self.pending_resize = None;
                match self
                    .stage
                    .encode_resize(u32::from(w), u32::from(h), None, None)
                {
                    Some(Ok(frame)) => Ok(Some(frame)),
                    Some(Err(e)) => Err(RdpError::Io(format!("cannot encode the resize: {e}"))),
                    None => Ok(None),
                }
            }
        }
    }

    /// Handles one command; returns the frames to write.
    fn command(&mut self, cmd: Cmd) -> Result<Vec<Vec<u8>>, RdpError> {
        match cmd {
            Cmd::Pointer {
                x,
                y,
                buttons,
                wheel,
            } => {
                let ops = self.pointer_ops(x, y, buttons, wheel);
                let events = self.db.apply(ops);
                self.input_frames(&events)
            }
            Cmd::Key { code, down } => {
                let Some(scancode) = keymap::scancode_for(code) else {
                    return Ok(Vec::new());
                };
                let op = if down {
                    Operation::KeyPressed(scancode)
                } else {
                    Operation::KeyReleased(scancode)
                };
                let events = self.db.apply([op]);
                self.input_frames(&events)
            }
            Cmd::Resize { width, height } => {
                Ok(self.resize_frame(width, height)?.into_iter().collect())
            }
            Cmd::Visible(v) => self.set_visible(v),
            Cmd::Clipboard(command) => {
                self.clipboard_frames(|engine, channel| engine.command(command, channel))
            }
            Cmd::Snapshot(reply) => {
                // The requester may have given up; that is fine.
                if reply.send(self.snapshot()).is_err() {
                    tracing::trace!("snapshot requester is gone");
                }
                Ok(Vec::new())
            }
        }
    }

    /// Runs `work` on the clipboard engine and encodes what it wants to send. Nothing happens
    /// when the clipboard is not shared (or the channel was not set up by the server).
    fn clipboard_frames(
        &mut self,
        work: impl FnOnce(&mut Engine, &mut ironrdp_cliprdr::CliprdrClient) -> Vec<SvcMessage>,
    ) -> Result<Vec<Vec<u8>>, RdpError> {
        let (Some(engine), Some(channel)) = (
            self.clip.as_mut(),
            self.stage.get_svc_processor_mut::<GuardedCliprdr>(),
        ) else {
            return Ok(Vec::new());
        };
        let messages = work(engine, channel.inner_mut());
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let frame = self
            .stage
            .process_svc_processor_messages(SvcProcessorMessages::<GuardedCliprdr>::new(messages))
            .map_err(|e| RdpError::Io(format!("cannot encode a clipboard message: {e}")))?;
        Ok(vec![frame])
    }

    async fn write_all(framed: &mut Framed, frames: Vec<Vec<u8>>) -> Result<(), RdpError> {
        for frame in frames {
            framed
                .write_all(&frame)
                .await
                .map_err(|e| RdpError::Io(format!("cannot write to the server: {e}")))?;
        }
        Ok(())
    }

    /// Deactivation-Reactivation Sequence: the server changed the desktop (typically after a
    /// resize). The session stays up; the image is recreated at the new size.
    async fn reactivate(&mut self, framed: &mut Framed) -> Result<(), RdpError> {
        let mut sequence = self.activation.create();
        let mut buf = WriteBuf::new();
        let state = tokio::time::timeout(REACTIVATION_TIMEOUT, async {
            loop {
                single_sequence_step(framed, &mut sequence, &mut buf)
                    .await
                    .map_err(|e| RdpError::Io(format!("reactivation failed: {e}")))?;
                match sequence.connection_activation_state() {
                    ConnectionActivationState::Finalized { .. } => {
                        return Ok(sequence.connection_activation_state())
                    }
                    ConnectionActivationState::Redirected { .. } => {
                        return Err(RdpError::Unsupported(
                            "the server asked for a redirection during reactivation".into(),
                        ))
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| RdpError::Io("reactivation timed out".into()))??;

        let ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            static_channel_chunk_size,
            enable_server_pointer,
            pointer_software_rendering,
            refresh_rect_support,
            suppress_output_support,
            window_support_level,
            ..
        } = state
        else {
            return Err(RdpError::Io("reactivation did not finish".into()));
        };
        if !self.stage.reactivate(
            self.activation.io_channel_id(),
            self.activation.user_channel_id(),
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
            static_channel_chunk_size,
        ) {
            return Err(RdpError::Io(
                "the server sent an invalid channel chunk size".into(),
            ));
        }
        self.stage.set_window_support_level(window_support_level);
        self.enable_server_pointer = enable_server_pointer;
        self.refresh_rect_support = refresh_rect_support;
        self.suppress_output_support = suppress_output_support;
        self.image =
            DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
        self.dirty.mark_full();
        self.emit(FbEvent::Resize {
            w: desktop_size.width,
            h: desktop_size.height,
        });
        // A new activation starts with output allowed; hide again if the tab is hidden.
        self.suppressed = false;
        if !self.visible && self.suppress_output_support {
            let frame = self.suppress_frame(false)?;
            self.suppressed = true;
            Self::write_all(framed, vec![frame]).await?;
        }
        Ok(())
    }

    async fn graceful_shutdown(&mut self, framed: &mut Framed) {
        let frames = match self.stage.graceful_shutdown() {
            Ok(outputs) => outputs
                .into_iter()
                .filter_map(|o| match o {
                    ActiveStageOutput::ResponseFrame(f) => Some(f),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            Err(e) => {
                tracing::debug!("cannot encode the shutdown request: {e}");
                return;
            }
        };
        if Self::write_all(framed, frames).await.is_err() {
            return;
        }
        // The server answers (denied) or drops the connection; either way we are done.
        if tokio::time::timeout(SHUTDOWN_GRACE, framed.read_pdu())
            .await
            .is_err()
        {
            tracing::debug!("no answer to the shutdown request");
        }
    }

    async fn run(
        &mut self,
        mut framed: Framed,
        cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
        clip_rx: &mut Option<mpsc::UnboundedReceiver<Internal>>,
        close: &mut watch::Receiver<bool>,
    ) -> Result<End, RdpError> {
        let mut clip_tick = tokio::time::interval(engine::TICK);
        clip_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // A tab that starts hidden is suppressed right away.
        if !self.visible && self.suppress_output_support {
            let frame = self.suppress_frame(false)?;
            self.suppressed = true;
            Self::write_all(&mut framed, vec![frame]).await?;
        }
        loop {
            if self.sink_gone {
                return Ok(End::Local);
            }
            if let Some(end) = self.end.take() {
                return Ok(end);
            }
            if self.needs_reactivation {
                self.needs_reactivation = false;
                self.reactivate(&mut framed).await?;
            }
            let deadline = self.flush_deadline();
            let mut frames: Vec<Vec<u8>> = Vec::new();
            tokio::select! {
                _ = close.changed() => {
                    self.graceful_shutdown(&mut framed).await;
                    return Ok(End::Local);
                }
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else {
                        self.graceful_shutdown(&mut framed).await;
                        return Ok(End::Local);
                    };
                    let mut batch = vec![cmd];
                    while batch.len() < COMMAND_BATCH {
                        let Ok(next) = cmd_rx.try_recv() else { break };
                        batch.push(next);
                    }
                    for cmd in coalesce(batch) {
                        frames.extend(self.command(cmd)?);
                    }
                }
                pdu = framed.read_pdu() => {
                    let (action, payload) = pdu.map_err(|e| match e.kind() {
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset => {
                            RdpError::Io("the server closed the connection".into())
                        }
                        _ => RdpError::Io(format!("network error: {e}")),
                    })?;
                    let outputs = self
                        .stage
                        .process(&mut self.image, action, &payload, framed.last_read_at())
                        .map_err(|e| RdpError::Io(format!("cannot process a server update: {e}")))?;
                    frames.extend(self.digest(outputs));
                    if let Some((w, h)) = self.pending_resize {
                        frames.extend(self.resize_frame(w, h)?);
                    }
                }
                Some(event) = next_clipboard_event(clip_rx) => {
                    frames.extend(self.clipboard_frames(|engine, channel| engine.on_internal(event, channel))?);
                }
                _ = clip_tick.tick(), if self.clip.is_some() => {
                    frames.extend(self.clipboard_frames(|engine, channel| engine.on_tick(channel))?);
                }
                _ = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.flush();
                }
            }
            Self::write_all(&mut framed, frames).await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use portolano_core_model::ErrorCode;

    use super::*;

    #[test]
    fn sizes_are_clamped_and_even() {
        assert_eq!(clamp_size(1281, 721), (1280, 721));
        assert_eq!(clamp_size(10, 10), (200, 200));
        assert_eq!(clamp_size(20000, 20000), (8192, 8192));
    }

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
        let mut config = RdpConfig::new("h", 3389, 800, 600, CertStore::new("x"));
        config.credentials = Some(RdpCredentials {
            username: "alice".into(),
            password: SecretString::from("hunter2-very-secret".to_owned()),
        });
        let text = format!("{config:?}");
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("alice"));
    }

    // ---- credential prompts (WP11) ----

    fn answer(user: &str, password: Option<&str>, domain: Option<&str>) -> CredentialAnswer {
        CredentialAnswer {
            username: user.into(),
            password: password.map(|p| SecretString::from(p.to_owned())),
            key_path: None,
            passphrase: None,
            domain: domain.map(str::to_owned),
        }
    }

    #[test]
    fn answers_are_validated_and_trimmed() {
        let (creds, domain) =
            credentials_from_answer(answer(" bob ", Some("pw"), Some(" CORP "))).expect("valid");
        assert_eq!(creds.username, "bob");
        assert_eq!(creds.password.expose_secret(), "pw");
        assert_eq!(domain.as_deref(), Some("CORP"));
        let (_, domain) =
            credentials_from_answer(answer("bob", Some("pw"), Some("  "))).expect("ok");
        assert_eq!(domain, None);
        assert!(credentials_from_answer(answer("  ", Some("pw"), None)).is_err());
        assert!(credentials_from_answer(answer("bob", None, None)).is_err());
        assert!(credentials_from_answer(answer("bob", Some(""), None)).is_err());
    }

    #[test]
    fn a_failed_ntlm_retry_keeps_its_own_error_unless_it_is_a_network_one() {
        // The credential re-prompt depends on AuthFailed surviving the retry.
        let auth = after_ntlm_retry(RdpError::AuthFailed("wrong".into()));
        assert!(matches!(&auth, RdpError::AuthFailed(m) if m == KDC_UNREACHABLE_NTLM_REFUSED));
        assert!(!KDC_UNREACHABLE_NTLM_REFUSED.contains("wrong"));
        assert!(asks_again(&auth, 1));
        assert!(matches!(
            after_ntlm_retry(RdpError::Unsupported("no nla".into())),
            RdpError::Unsupported(_)
        ));
        assert!(matches!(
            after_ntlm_retry(RdpError::CertificateChanged),
            RdpError::CertificateChanged
        ));
        assert!(matches!(
            after_ntlm_retry(RdpError::Cancelled),
            RdpError::Cancelled
        ));
        assert!(matches!(
            after_ntlm_retry(RdpError::InvalidInput("x".into())),
            RdpError::InvalidInput(_)
        ));
        assert!(matches!(
            after_ntlm_retry(RdpError::PromptCancelled),
            RdpError::PromptCancelled
        ));
        assert!(matches!(
            after_ntlm_retry(RdpError::Io("reset".into())),
            RdpError::KdcUnreachable
        ));
        assert!(matches!(
            after_ntlm_retry(RdpError::HostUnreachable("down".into())),
            RdpError::KdcUnreachable
        ));
    }

    #[test]
    fn only_an_authentication_failure_asks_again_and_only_a_few_times() {
        let auth = RdpError::AuthFailed("x".into());
        assert!(asks_again(&auth, 1));
        assert!(asks_again(&auth, MAX_PROMPT_ATTEMPTS - 1));
        assert!(!asks_again(&auth, MAX_PROMPT_ATTEMPTS));
        assert!(!asks_again(&RdpError::Io("x".into()), 1));
        assert!(!asks_again(&RdpError::HostUnreachable("x".into()), 1));
        assert!(!asks_again(&RdpError::Unsupported("x".into()), 1));
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

    fn ask_spec(hint: Option<&str>) -> RdpAsk {
        RdpAsk {
            username_hint: hint.map(str::to_owned),
            hooks: PromptHooks::no_vault(),
        }
    }

    fn test_config() -> RdpConfig {
        let store = CertStore::in_data_dir(std::path::Path::new("/nonexistent"));
        RdpConfig::new("rdp.example", 3389, 800, 600, store)
    }

    #[tokio::test]
    async fn the_prompt_carries_the_hints_and_the_reply_comes_back() {
        let (sink, events) = collecting_sink();
        let slot = Arc::new(PromptSlot::new());
        let (_close_tx, mut close) = watch::channel(false);
        let config = test_config();
        let ask = ask_spec(Some("alice"));
        let waiting = {
            let slot = slot.clone();
            async move {
                // Wait for the prompt, then answer it.
                loop {
                    if slot
                        .reply(PromptReply::Answer {
                            answer: answer("alice", Some("pw-1"), Some("CORP")),
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
                &ask,
                &sink,
                &slot,
                &mut close,
                Some("alice"),
                Some("DOM"),
                Some("earlier failure".into()),
            ),
            waiting
        );
        let asked = asked.expect("asked");
        assert_eq!(asked.credentials.username, "alice");
        assert_eq!(asked.domain.as_deref(), Some("CORP"));
        assert!(asked.save);
        let events = events.lock().expect("events");
        let [FbEvent::CredentialPrompt(prompt)] = events.as_slice() else {
            panic!("expected exactly one prompt, got {events:?}");
        };
        assert_eq!(prompt.kind, PromptKind::UsernamePassword);
        assert_eq!(prompt.username_hint.as_deref(), Some("alice"));
        assert_eq!(prompt.domain_hint.as_deref(), Some("DOM"));
        assert_eq!(prompt.target, "rdp.example:3389");
        assert_eq!(prompt.error.as_deref(), Some("earlier failure"));
        assert!(!prompt.can_save_to_vault);
    }

    #[tokio::test]
    async fn an_invalid_answer_is_asked_again_with_a_message_and_cancel_is_distinct() {
        let (sink, events) = collecting_sink();
        let slot = Arc::new(PromptSlot::new());
        let (_close_tx, mut close) = watch::channel(false);
        let config = test_config();
        let ask = ask_spec(None);
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
                    answer: answer("", Some("pw"), None),
                    save_to_vault: false,
                })
                .expect("first reply");
                wait_for(2).await;
                slot.reply(PromptReply::Cancel).expect("cancel");
            }
        };
        let (result, ()) = tokio::join!(
            ask_credentials(&config, &ask, &sink, &slot, &mut close, None, None, None),
            driver
        );
        assert!(matches!(result, Err(RdpError::PromptCancelled)));
        assert_eq!(RdpError::PromptCancelled.code(), ErrorCode::Cancelled);
        let events = events.lock().expect("events");
        let [FbEvent::CredentialPrompt(first), FbEvent::CredentialPrompt(second)] =
            events.as_slice()
        else {
            panic!("expected two prompts, got {events:?}");
        };
        assert_eq!(first.error, None);
        assert_eq!(second.error.as_deref(), Some("Enter a user name."));
    }

    #[tokio::test]
    async fn closing_the_session_while_asking_is_not_a_cancellation_by_the_user() {
        let (sink, _events) = collecting_sink();
        let slot = PromptSlot::new();
        let (close_tx, mut close) = watch::channel(false);
        close_tx.send_replace(true);
        let result = ask_credentials(
            &test_config(),
            &ask_spec(None),
            &sink,
            &slot,
            &mut close,
            None,
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(RdpError::Cancelled)));
    }
}
