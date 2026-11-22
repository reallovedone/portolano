//! `AppState` and the `SessionManager`.
//!
//! Every live session (terminal, framebuffer, web) is registered here under a `SessionId`.
//! The three families expose a small handle trait each; the command groups downcast nothing:
//! `fb_*`, `term_*` and `web_*` commands look the handle up by kind and call the trait. A
//! protocol implementation (SSH, PTY, RDP, VNC, child webview) only has to implement its
//! family's trait in its own file and register the handle with `SessionManager::insert`.

// The session API is consumed by later work packages; remove once they use all of it.
#![allow(dead_code)]

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use portolano_core_model::prompt::CredentialAnswer;
use portolano_core_model::web::{Bounds, WebNavAction};
use portolano_core_model::{connections_path, vault_path, DocumentStore};
use portolano_importers::PreviewCache;
use portolano_vault::Vault;
use uuid::Uuid;

use crate::clipboard::SystemClipboard;
use crate::error::{poisoned, AppError};
use crate::quick_unlock::QuickUnlock;
use crate::shares::ShareService;

pub type SessionId = Uuid;

/// Boxed future so that handle traits stay object-safe without extra dependencies.
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Terminal family (local shell, SSH).
pub trait TermSession: Send + Sync {
    /// Raw bytes typed by the user. The UI sends one `term_write` at a time.
    fn write(&self, data: Vec<u8>) -> BoxFut<'_, Result<(), AppError>>;
    /// The UI consumed `bytes` of output (flow control).
    fn ack(&self, bytes: u64) -> Result<(), AppError>;
    fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError>;
    /// Answer to a `HostKeyPrompt` (unknown key only).
    fn host_key_decision(&self, accept: bool) -> Result<(), AppError>;
    /// Answer to a `CredentialPrompt`. The secrets stay in the backend, for this session.
    fn credential(&self, answer: CredentialAnswer, save_to_vault: bool) -> Result<(), AppError>;
    /// The user dismissed a `CredentialPrompt`.
    fn credential_cancel(&self) -> Result<(), AppError>;
    /// Stops the session and releases its resources. Must not block.
    fn close(&self);
}

/// Framebuffer family (RDP, VNC). Implemented in `commands/rdp.rs` and `commands/vnc.rs`.
pub trait FbSession: Send + Sync {
    /// `buttons` uses the DOM `MouseEvent.buttons` bits; `wheel` is the vertical scroll delta
    /// (positive scrolls down).
    fn pointer(&self, x: u16, y: u16, buttons: u8, wheel: i16) -> Result<(), AppError>;
    /// `code` is `KeyboardEvent.code`.
    fn key(&self, code: &str, down: bool) -> Result<(), AppError>;
    fn resize(&self, width: u16, height: u16) -> Result<(), AppError>;
    /// Hidden: stop sending. Visible: send a full redraw.
    fn set_visible(&self, visible: bool) -> Result<(), AppError>;
    /// Answer to a `CertPrompt`.
    fn cert_decision(&self, accept: bool) -> Result<(), AppError>;
    /// Answer to an `UnencryptedPrompt` (VNC): agree to send the password without encryption.
    fn unencrypted_decision(&self, _allow: bool) -> Result<(), AppError> {
        Err(AppError::invalid_input(
            "this session never asks about encryption",
        ))
    }
    /// Removes the stored certificate of the session's host (the deliberate step after a
    /// changed certificate or a refused downgrade).
    fn forget_certificate(&self) -> Result<(), AppError>;
    /// Saves the files on the remote clipboard below `dir`, a folder the user chose (RDP with
    /// clipboard sharing only).
    fn clipboard_save(&self, _dir: PathBuf) -> Result<(), AppError> {
        Err(AppError::invalid_input(
            "this session does not share a clipboard",
        ))
    }
    /// Stops the save started by `clipboard_save`.
    fn clipboard_cancel(&self) -> Result<(), AppError> {
        Err(AppError::invalid_input(
            "this session does not share a clipboard",
        ))
    }
    /// Answer to a `CredentialPrompt`. The secrets stay in the backend, for this session.
    fn credential(&self, answer: CredentialAnswer, save_to_vault: bool) -> Result<(), AppError>;
    /// The user dismissed a `CredentialPrompt`.
    fn credential_cancel(&self) -> Result<(), AppError>;
    fn close(&self);
}

/// Web family (child webview).
pub trait WebSession: Send + Sync {
    /// `bounds` are in the UI viewport's CSS px; `ui_viewport_height` is the UI's
    /// `innerHeight` at the time `bounds` was measured (used for the title bar correction).
    fn set_bounds(
        &self,
        bounds: Bounds,
        ui_viewport_height: Option<f64>,
        visible: bool,
    ) -> Result<(), AppError>;
    fn navigate(&self, action: WebNavAction) -> Result<(), AppError>;
    fn close(&self);
}

#[derive(Clone)]
pub enum SessionHandle {
    Term(Arc<dyn TermSession>),
    Fb(Arc<dyn FbSession>),
    Web(Arc<dyn WebSession>),
}

impl SessionHandle {
    fn close(&self) {
        match self {
            Self::Term(s) => s.close(),
            Self::Fb(s) => s.close(),
            Self::Web(s) => s.close(),
        }
    }

    fn family(&self) -> &'static str {
        match self {
            Self::Term(_) => "terminal",
            Self::Fb(_) => "framebuffer",
            Self::Web(_) => "web",
        }
    }
}

#[derive(Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<SessionId, SessionHandle>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a session and returns its new id.
    pub fn insert(&self, handle: SessionHandle) -> Result<SessionId, AppError> {
        let id = Uuid::new_v4();
        self.sessions.lock().map_err(poisoned)?.insert(id, handle);
        Ok(id)
    }

    fn get(&self, id: SessionId) -> Result<SessionHandle, AppError> {
        self.sessions
            .lock()
            .map_err(poisoned)?
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::not_found(format!("session {id} not found")))
    }

    fn wrong_family(id: SessionId, handle: &SessionHandle, wanted: &str) -> AppError {
        AppError::invalid_input(format!(
            "session {id} is a {} session, not a {wanted} one",
            handle.family()
        ))
    }

    pub fn term(&self, id: SessionId) -> Result<Arc<dyn TermSession>, AppError> {
        match self.get(id)? {
            SessionHandle::Term(s) => Ok(s),
            other => Err(Self::wrong_family(id, &other, "terminal")),
        }
    }

    pub fn fb(&self, id: SessionId) -> Result<Arc<dyn FbSession>, AppError> {
        match self.get(id)? {
            SessionHandle::Fb(s) => Ok(s),
            other => Err(Self::wrong_family(id, &other, "framebuffer")),
        }
    }

    pub fn web(&self, id: SessionId) -> Result<Arc<dyn WebSession>, AppError> {
        match self.get(id)? {
            SessionHandle::Web(s) => Ok(s),
            other => Err(Self::wrong_family(id, &other, "web")),
        }
    }

    /// Routes the answer of a credential prompt to the terminal or framebuffer session.
    pub fn credential(
        &self,
        id: SessionId,
        answer: CredentialAnswer,
        save_to_vault: bool,
    ) -> Result<(), AppError> {
        match self.get(id)? {
            SessionHandle::Term(s) => s.credential(answer, save_to_vault),
            SessionHandle::Fb(s) => s.credential(answer, save_to_vault),
            other => Err(Self::wrong_family(id, &other, "terminal or framebuffer")),
        }
    }

    /// Routes the cancellation of a credential prompt.
    pub fn credential_cancel(&self, id: SessionId) -> Result<(), AppError> {
        match self.get(id)? {
            SessionHandle::Term(s) => s.credential_cancel(),
            SessionHandle::Fb(s) => s.credential_cancel(),
            other => Err(Self::wrong_family(id, &other, "terminal or framebuffer")),
        }
    }

    /// Removes and closes a session (`session_close`).
    pub fn close(&self, id: SessionId) -> Result<(), AppError> {
        let handle = self
            .sessions
            .lock()
            .map_err(poisoned)?
            .remove(&id)
            .ok_or_else(|| AppError::not_found(format!("session {id} not found")))?;
        handle.close();
        Ok(())
    }

    /// Closes every session (application exit).
    pub fn close_all(&self) {
        let drained: Vec<SessionHandle> = match self.sessions.lock() {
            Ok(mut map) => map.drain().map(|(_, h)| h).collect(),
            Err(e) => e.into_inner().drain().map(|(_, h)| h).collect(),
        };
        for handle in drained {
            handle.close();
        }
    }

    pub fn len(&self) -> usize {
        self.sessions.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Backend-to-UI notices (Tauri events to the main webview) for code that has no `AppHandle`,
/// such as the hooks of a session. Nothing is delivered until [`Notifier::connect`] ran (in the
/// Tauri `setup` hook), so unit tests that build an `AppState` simply drop the notices.
type NoticeSink = Box<dyn Fn(&str, serde_json::Value) + Send + Sync>;

#[derive(Default)]
pub struct Notifier {
    sink: OnceLock<NoticeSink>,
}

impl Notifier {
    /// Routes notices to a sink. Only the first call has effect.
    pub fn connect(&self, sink: impl Fn(&str, serde_json::Value) + Send + Sync + 'static) {
        if self.sink.set(Box::new(sink)).is_err() {
            tracing::debug!("notifier already connected");
        }
    }

    /// Emits `event` with a JSON payload; a no-op when not connected.
    pub fn emit(&self, event: &str, payload: serde_json::Value) {
        if let Some(sink) = self.sink.get() {
            sink(event, payload);
        }
    }
}

/// Locks the vault mutex. A poisoned mutex means a panic while the vault was being used, so its
/// in-memory state cannot be trusted: fail closed. The guard is recovered, the vault is locked
/// (the key and every secret are zeroised, the user unlocks again from the file) and the
/// poison is cleared, so the vault stays usable and, above all, lockable.
pub(crate) fn lock_vault_mutex(vault: &Mutex<Vault>) -> Result<MutexGuard<'_, Vault>, AppError> {
    match vault.lock() {
        Ok(guard) => Ok(guard),
        Err(poison) => {
            let mut guard = poison.into_inner();
            tracing::error!("vault state was poisoned; locking the vault");
            if let Err(e) = guard.lock() {
                tracing::error!(error = %e, "could not lock a poisoned vault");
                return Err(AppError::from(e));
            }
            vault.clear_poison();
            Ok(guard)
        }
    }
}

pub struct AppState {
    pub data_dir: PathBuf,
    /// `Arc` so that a session can hold them (the "save to vault" hook of a credential prompt
    /// updates the document and the vault after the command that opened it returned).
    pub store: Arc<Mutex<DocumentStore>>,
    pub vault: Arc<Mutex<Vault>>,
    pub sessions: SessionManager,
    /// WP9: parsed import previews waiting for `import_apply` (oldest evicted).
    pub import_previews: Mutex<PreviewCache>,
    /// Notices for the UI from code that runs outside a command (see [`Notifier`]).
    pub notifier: Arc<Notifier>,
    /// The operating system's clipboard, for RDP sessions that share it (WP14).
    pub clipboard: Arc<SystemClipboard>,
    /// The key behind Touch ID / Windows Hello unlock; in memory only (see `quick_unlock`).
    pub quick_unlock: QuickUnlock,
    /// Shared folders: publishing, subscriptions, sync and the merged tree.
    pub shares: Arc<ShareService>,
}

impl AppState {
    /// Builds the state for `data_dir`. Performs no I/O: the store and the vault bind to their
    /// files lazily (their real loading is owned by WP2 and WP3).
    pub fn new(data_dir: PathBuf) -> Self {
        let vault = Vault::new(vault_path(&data_dir));
        Self::from_parts(data_dir, vault)
    }

    /// Like [`AppState::new`] with a vault built by the caller (tests use a cheap key derivation).
    pub fn from_parts(data_dir: PathBuf, vault: Vault) -> Self {
        let store = Arc::new(Mutex::new(DocumentStore::new(connections_path(&data_dir))));
        let vault = Arc::new(Mutex::new(vault));
        let notifier = Arc::new(Notifier::default());
        let shares = Arc::new(ShareService::new(
            data_dir.clone(),
            Arc::clone(&store),
            Arc::clone(&vault),
            Arc::clone(&notifier),
        ));
        Self {
            data_dir,
            store,
            vault,
            shares,
            sessions: SessionManager::new(),
            import_previews: Mutex::new(PreviewCache::default()),
            notifier,
            clipboard: Arc::new(SystemClipboard::default()),
            quick_unlock: QuickUnlock::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use portolano_core_model::ErrorCode;

    use super::*;

    struct DummyWeb(Arc<AtomicUsize>);

    impl WebSession for DummyWeb {
        fn set_bounds(&self, _: Bounds, _: Option<f64>, _: bool) -> Result<(), AppError> {
            Ok(())
        }
        fn navigate(&self, _: WebNavAction) -> Result<(), AppError> {
            Ok(())
        }
        fn close(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn insert_lookup_close() {
        let closed = Arc::new(AtomicUsize::new(0));
        let mgr = SessionManager::new();
        let id = mgr
            .insert(SessionHandle::Web(Arc::new(DummyWeb(closed.clone()))))
            .expect("insert");
        assert!(mgr.web(id).is_ok());
        assert_eq!(mgr.len(), 1);
        mgr.close(id).expect("close");
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        assert!(mgr.is_empty());
        assert_eq!(mgr.close(id).expect_err("gone").code, ErrorCode::NotFound);
    }

    #[test]
    fn wrong_family_is_invalid_input() {
        let mgr = SessionManager::new();
        let id = mgr
            .insert(SessionHandle::Web(Arc::new(DummyWeb(Arc::default()))))
            .expect("insert");
        let err = mgr.fb(id).err().expect("wrong family");
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn close_all_closes_everything() {
        let closed = Arc::new(AtomicUsize::new(0));
        let mgr = SessionManager::new();
        for _ in 0..3 {
            mgr.insert(SessionHandle::Web(Arc::new(DummyWeb(closed.clone()))))
                .expect("insert");
        }
        mgr.close_all();
        assert_eq!(closed.load(Ordering::SeqCst), 3);
        assert!(mgr.is_empty());
    }

    #[test]
    fn app_state_does_no_io() {
        let state = AppState::new(PathBuf::from("/nonexistent/portolano"));
        assert!(state.sessions.is_empty());
    }
}
