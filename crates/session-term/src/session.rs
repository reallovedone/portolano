//! A terminal session: local shell or SSH, driven through one small API.
//!
//! `open` returns at once. A local shell is spawned synchronously (spawn errors are returned);
//! an SSH session connects in a background task and reports everything through the sink
//! (`HostKeyPrompt`, then data, or `Error`). Input sent before an SSH session is ready is
//! discarded; a resize sent meanwhile is remembered and applied when the shell is open.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use portolano_core_model::prompt::{CredentialAnswer, PromptReply, PromptSlot};
use portolano_core_model::{AppError, ErrorCode};
use tokio::sync::{mpsc, oneshot, watch};

use crate::flow::{self, Credit, Source, SOURCE_QUEUE};
use crate::pty::{self, PtyControl};
use crate::ssh::{self, Gate, SshControl, SshSpec};
use crate::{KnownHosts, MAX_UNACKED_BYTES};

/// Receives encoded `TermEvent` frames. Returns `false` once the receiver is gone.
pub type FrameSink = Arc<dyn Fn(Vec<u8>) -> bool + Send + Sync>;

/// A local shell. `None` fields mean the platform default (login shell / PowerShell, home).
#[derive(Debug, Clone, Default)]
pub struct LocalSpec {
    pub program: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SessionSpec {
    Local(LocalSpec),
    Ssh(Box<SshSpec>),
}

enum Backend {
    /// SSH connecting.
    Starting,
    Pty(PtyControl),
    Ssh(SshControl),
    Ended,
}

struct Inner {
    credit: Arc<Credit>,
    backend: Mutex<Backend>,
    /// Latest size requested by the UI.
    size: Mutex<(u16, u16)>,
    decision: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    credential: Arc<PromptSlot>,
    closed: watch::Sender<bool>,
    runtime: tokio::runtime::Handle,
}

fn poisoned() -> AppError {
    AppError::internal("terminal session state is poisoned")
}

impl Inner {
    fn close(&self) {
        self.closed.send_replace(true);
        let previous = match self.backend.lock() {
            Ok(mut b) => std::mem::replace(&mut *b, Backend::Ended),
            Err(e) => std::mem::replace(&mut *e.into_inner(), Backend::Ended),
        };
        if let Backend::Pty(pty) = previous {
            pty.kill();
        }
    }
}

pub struct Session {
    inner: Arc<Inner>,
}

impl Session {
    /// Starts a session. Must be called inside a tokio runtime.
    pub fn open(
        spec: SessionSpec,
        cols: u16,
        rows: u16,
        sink: FrameSink,
    ) -> Result<Self, AppError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| AppError::internal("no async runtime is available"))?;
        let (tx, rx) = mpsc::channel::<Source>(SOURCE_QUEUE);
        let (closed, closed_rx) = watch::channel(false);
        let credit = Arc::new(Credit::new(MAX_UNACKED_BYTES));
        let decision = Arc::new(Mutex::new(None));
        let credential = Arc::new(PromptSlot::new());

        let backend = match &spec {
            SessionSpec::Local(local) => Backend::Pty(pty::spawn(local, cols, rows, tx.clone())?),
            SessionSpec::Ssh(_) => Backend::Starting,
        };
        let inner = Arc::new(Inner {
            credit: credit.clone(),
            backend: Mutex::new(backend),
            size: Mutex::new((cols, rows)),
            decision: decision.clone(),
            credential: credential.clone(),
            closed,
            runtime: runtime.clone(),
        });

        if let SessionSpec::Ssh(ssh_spec) = spec {
            let gate = Gate {
                known_hosts: KnownHosts::new(&ssh_spec.known_hosts_path),
                sink: sink.clone(),
                decision,
                prompting: Arc::new(AtomicBool::new(false)),
                credential,
            };
            runtime.spawn(run_ssh(
                inner.clone(),
                *ssh_spec,
                gate,
                cols,
                rows,
                tx,
                closed_rx.clone(),
            ));
        }

        let for_forwarder = inner.clone();
        runtime.spawn(async move {
            flow::forward(rx, sink, credit, closed_rx).await;
            // Exit, error or a vanished receiver: the session is over, release everything.
            for_forwarder.close();
        });
        Ok(Self { inner })
    }

    /// Raw bytes typed by the user. Callers send one write at a time.
    pub async fn write(&self, data: Vec<u8>) -> Result<(), AppError> {
        enum Target {
            Pty(Arc<Mutex<Box<dyn std::io::Write + Send>>>),
            Ssh(SshControl),
            NotReady,
            Ended,
        }
        let target = {
            let backend = self.inner.backend.lock().map_err(|_| poisoned())?;
            match &*backend {
                Backend::Pty(p) => Target::Pty(p.writer()),
                Backend::Ssh(s) => Target::Ssh(s.clone()),
                Backend::Starting => Target::NotReady,
                Backend::Ended => Target::Ended,
            }
        };
        match target {
            Target::Pty(writer) => {
                tokio::task::spawn_blocking(move || PtyControl::write_blocking(&writer, &data))
                    .await
                    .map_err(|e| AppError::internal(format!("terminal write task failed: {e}")))?
            }
            Target::Ssh(control) => control.write(data).await,
            Target::NotReady => {
                tracing::debug!("input discarded: the SSH session is not connected yet");
                Ok(())
            }
            Target::Ended => Err(AppError::new(ErrorCode::Io, "The session has ended.")),
        }
    }

    /// The UI consumed `bytes` of output.
    pub fn ack(&self, bytes: u64) {
        self.inner.credit.ack(bytes);
    }

    /// Bytes sent to the UI and not yet acknowledged.
    pub fn unacked(&self) -> u64 {
        self.inner.credit.unacked()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError> {
        if cols == 0 || rows == 0 {
            return Err(AppError::invalid_input("the terminal size cannot be zero"));
        }
        *self.inner.size.lock().map_err(|_| poisoned())? = (cols, rows);
        let backend = self.inner.backend.lock().map_err(|_| poisoned())?;
        match &*backend {
            Backend::Pty(p) => p.resize(cols, rows),
            Backend::Ssh(s) => {
                let control = s.clone();
                self.inner.runtime.spawn(async move {
                    if let Err(e) = control.resize(cols, rows).await {
                        tracing::debug!("resize failed: {e}");
                    }
                });
                Ok(())
            }
            Backend::Starting | Backend::Ended => Ok(()),
        }
    }

    /// Answer to a `HostKeyPrompt`.
    pub fn host_key_decision(&self, accept: bool) -> Result<(), AppError> {
        let sender = self
            .inner
            .decision
            .lock()
            .map_err(|_| poisoned())?
            .take()
            .ok_or_else(|| AppError::invalid_input("no host key decision is pending"))?;
        sender
            .send(accept)
            .map_err(|_| AppError::invalid_input("the host key prompt is no longer pending"))
    }

    /// Answer to a `CredentialPrompt`. The secrets are used for this session only.
    pub fn credential(
        &self,
        answer: CredentialAnswer,
        save_to_vault: bool,
    ) -> Result<(), AppError> {
        self.inner.credential.reply(PromptReply::Answer {
            answer,
            save_to_vault,
        })
    }

    /// The user dismissed the `CredentialPrompt`: the connection ends with `cancelled`.
    pub fn credential_cancel(&self) -> Result<(), AppError> {
        self.inner.credential.reply(PromptReply::Cancel)
    }

    /// Stops the session and releases its resources. Does not block; safe to call twice.
    pub fn close(&self) {
        self.inner.close();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.inner.close();
    }
}

async fn run_ssh(
    inner: Arc<Inner>,
    spec: SshSpec,
    gate: Gate,
    cols: u16,
    rows: u16,
    tx: mpsc::Sender<Source>,
    mut closed: watch::Receiver<bool>,
) {
    let connected = tokio::select! {
        result = ssh::connect(&spec, &gate, cols, rows) => result,
        _ = closed.wait_for(|c| *c) => return,
    };
    let connected = match connected {
        Ok(c) => c,
        Err(e) => {
            if tx.send(Source::Fatal(e)).await.is_err() {
                tracing::debug!("session closed before the connection error was delivered");
            }
            return;
        }
    };
    let control = connected.control();
    match inner.backend.lock() {
        Ok(mut backend) => {
            if matches!(*backend, Backend::Ended) {
                // Closed while connecting: the pump below shuts the connection down.
            } else {
                *backend = Backend::Ssh(control.clone());
            }
        }
        Err(_) => tracing::warn!("cannot publish the SSH control: state is poisoned"),
    }
    // A resize that arrived while connecting.
    let latest = inner.size.lock().map(|s| *s).unwrap_or((cols, rows));
    if latest != (cols, rows) {
        if let Err(e) = control.resize(latest.0, latest.1).await {
            tracing::debug!("initial resize failed: {e}");
        }
    }
    connected.pump(tx, closed).await;
}
