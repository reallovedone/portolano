//! Shared clipboard of an RDP session (text and files), over the `CLIPRDR` static channel.
//!
//! The channel is negotiated **only** when the connection's `share_clipboard` option is on.
//!
//! # Design
//!
//! - The operating system's clipboard sits behind the [`LocalClipboard`] trait. The app
//!   implements it in the Rust backend (never in the webview); tests use a fake. This crate
//!   never touches a real clipboard.
//! - [`backend::ForwardBackend`] is the `ironrdp-cliprdr` backend: every callback becomes an
//!   event on a queue, so the channel code never calls back into the session while it holds
//!   its own state. [`engine::Engine`] consumes that queue.
//! - [`engine::Engine`] owns the policy: it takes part only while the session's tab is the
//!   active one, announces the local clipboard when it changes, fetches remote text when the
//!   remote machine copies, and serves or saves files. See `engine.rs` for the rules.
//! - [`guard::GuardedCliprdr`] wraps the channel so that a malformed clipboard PDU is an
//!   error logged without content, not the end of the desktop session.
//!
//! # Privacy
//!
//! Clipboard contents, file names and paths are never logged, never put in an error message
//! and never persisted. Files reach the disk only when the user asks, below a folder the user
//! chose, under names sanitised by [`names`], never overwriting anything.

pub(crate) mod backend;
pub mod download;
pub(crate) mod engine;
pub(crate) mod guard;
pub mod local_files;
pub mod names;
pub mod text;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

pub use engine::Command;

/// A failure of the operating system's clipboard. The message never holds clipboard contents.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LocalClipboardError(pub String);

/// What is on the local clipboard that the session can offer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalContent {
    pub text: Option<String>,
    /// Files and folders copied in the file manager.
    pub files: Vec<PathBuf>,
    /// The content is a secret the application itself put there (a copied password): it is
    /// never offered to the remote machine.
    pub sensitive: bool,
}

/// The local clipboard, as the session needs it. Implementations must be cheap to call from
/// any thread; `read` and `write_text` may block briefly (they run off the async tasks).
pub trait LocalClipboard: Send + Sync + 'static {
    /// A number that changes whenever the clipboard content changes, readable without asking
    /// the user for anything (macOS: `NSPasteboard.changeCount`; Windows: the clipboard
    /// sequence number). `None` when the platform has no such thing: the session then reads
    /// the content on every tick.
    fn change_token(&self) -> Option<u64>;

    /// The current content. Text longer than the limits is the implementation's to return;
    /// the session enforces them.
    fn read(&self) -> Result<LocalContent, LocalClipboardError>;

    /// Replaces the clipboard with `text` (what the remote machine copied).
    fn write_text(&self, text: &str) -> Result<(), LocalClipboardError>;
}

/// What a session needs to share the clipboard. `None` in the configuration means: do not
/// negotiate the channel at all.
#[derive(Clone)]
pub struct ClipboardConfig {
    pub local: Arc<dyn LocalClipboard>,
}

impl std::fmt::Debug for ClipboardConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardConfig").finish_non_exhaustive()
    }
}
