//! The operating system's clipboard, for RDP sessions that share it.
//!
//! Everything happens here, in the Rust backend: the webview has no clipboard permission for
//! this and never sees what is shared. `arboard` (MIT or Apache-2.0, maintained by 1Password)
//! reads and writes text and the list of copied files on macOS and Windows.
//!
//! # Secrets
//!
//! A secret the user copies out of the vault must not be offered to a remote desktop:
//!
//! - `vault_reveal_secret` registers the SHA-256 of the secret here ([`SecretMarks`]) for
//!   [`SECRET_TTL`]; a clipboard whose text matches is reported as `sensitive` and the
//!   session never announces or serves it. Only the hash is kept, in memory;
//! - content that other password managers mark as concealed (macOS
//!   `org.nspasteboard.ConcealedType`, Windows `ExcludeClipboardContentFromMonitorProcessing`)
//!   is treated the same way.
//!
//! The vault's own "clear after 30 seconds" keeps working: it uses the webview's clipboard
//! API on its own and is not affected by this module.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use portolano_session_rdp::clipboard::{LocalClipboard, LocalClipboardError, LocalContent};
use sha2::{Digest, Sha256};

/// How long a revealed secret is remembered as "do not share" (longer than the 30 seconds the
/// vault leaves it on the clipboard).
pub const SECRET_TTL: Duration = Duration::from_secs(120);
/// Remembered secrets at most (one per reveal; old ones expire first).
const MAX_MARKS: usize = 64;

type Hash = [u8; 32];

fn hash_of(text: &str) -> Hash {
    Sha256::digest(text.as_bytes()).into()
}

/// SHA-256 of recently revealed secrets, to recognise them on the clipboard.
#[derive(Default)]
pub struct SecretMarks {
    entries: Mutex<Vec<(Hash, Instant)>>,
}

impl SecretMarks {
    /// Remembers `secret` for [`SECRET_TTL`]. Empty secrets are ignored.
    pub fn mark(&self, secret: &str) {
        self.mark_at(secret, Instant::now());
    }

    fn mark_at(&self, secret: &str, now: Instant) {
        if secret.is_empty() {
            return;
        }
        // A poisoned lock only means another thread panicked while marking: the list is still
        // a list of hashes.
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|(_, at)| now.duration_since(*at) < SECRET_TTL);
        if entries.len() >= MAX_MARKS {
            entries.remove(0);
        }
        entries.push((hash_of(secret), now));
    }

    /// Whether `text` is a secret revealed recently.
    pub fn matches(&self, text: &str) -> bool {
        self.matches_at(text, Instant::now())
    }

    fn matches_at(&self, text: &str, now: Instant) -> bool {
        let hash = hash_of(text);
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries
            .iter()
            .any(|(h, at)| *h == hash && now.duration_since(*at) < SECRET_TTL)
    }
}

/// The real clipboard.
#[derive(Default)]
pub struct SystemClipboard {
    secrets: SecretMarks,
}

impl SystemClipboard {
    pub fn secrets(&self) -> &SecretMarks {
        &self.secrets
    }
}

fn describe(e: &arboard::Error) -> LocalClipboardError {
    // The kind only: the platform's description could echo more than we want in a log.
    let kind = match e {
        arboard::Error::ContentNotAvailable => "no usable content",
        arboard::Error::ClipboardNotSupported => "not supported",
        arboard::Error::ClipboardOccupied => "occupied by another application",
        arboard::Error::ConversionFailure => "conversion failed",
        _ => "access failed",
    };
    LocalClipboardError(format!("clipboard: {kind}"))
}

impl LocalClipboard for SystemClipboard {
    fn change_token(&self) -> Option<u64> {
        platform::change_token()
    }

    fn read(&self) -> Result<LocalContent, LocalClipboardError> {
        let concealed = platform::is_concealed();
        let mut clipboard = arboard::Clipboard::new().map_err(|e| describe(&e))?;
        // No file list is not an error: most of the time there is text or nothing.
        let files = clipboard.get().file_list().unwrap_or_default();
        let text = if files.is_empty() {
            match clipboard.get_text() {
                Ok(text) => Some(text),
                Err(arboard::Error::ContentNotAvailable | arboard::Error::ConversionFailure) => {
                    None
                }
                Err(e) => return Err(describe(&e)),
            }
        } else {
            None
        };
        let sensitive = concealed || text.as_deref().is_some_and(|t| self.secrets.matches(t));
        Ok(LocalContent {
            text,
            files,
            sensitive,
        })
    }

    fn write_text(&self, text: &str) -> Result<(), LocalClipboardError> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.set_text(text.to_owned()))
            .map_err(|e| describe(&e))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;

    /// `NSPasteboard.changeCount`: changes with every write, readable without any prompt.
    pub fn change_token() -> Option<u64> {
        let count = NSPasteboard::generalPasteboard().changeCount();
        u64::try_from(count).ok()
    }

    /// Password managers mark what they put on the pasteboard with these types.
    pub fn is_concealed() -> bool {
        let Some(types) = NSPasteboard::generalPasteboard().types() else {
            return false;
        };
        let concealed = NSString::from_str("org.nspasteboard.ConcealedType");
        let transient = NSString::from_str("org.nspasteboard.TransientType");
        types
            .iter()
            .any(|t| t.isEqualToString(&concealed) || t.isEqualToString(&transient))
    }
}

#[cfg(windows)]
mod platform {
    /// `GetClipboardSequenceNumber`: changes with every clipboard change, no clipboard lock.
    pub fn change_token() -> Option<u64> {
        clipboard_win::seq_num().map(|n| u64::from(n.get()))
    }

    /// Password managers (KeePass, 1Password, Bitwarden) add this format to keep their content
    /// out of clipboard history and monitors.
    pub fn is_concealed() -> bool {
        clipboard_win::register_format("ExcludeClipboardContentFromMonitorProcessing")
            .is_some_and(|f| clipboard_win::is_format_avail(f.get()))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub fn change_token() -> Option<u64> {
        None
    }

    pub fn is_concealed() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_secret_is_recognised_until_it_expires() {
        let marks = SecretMarks::default();
        let t0 = Instant::now();
        marks.mark_at("hunter2", t0);
        assert!(marks.matches_at("hunter2", t0));
        assert!(marks.matches_at("hunter2", t0 + SECRET_TTL - Duration::from_secs(1)));
        assert!(!marks.matches_at("hunter2", t0 + SECRET_TTL));
        assert!(!marks.matches_at("hunter3", t0));
    }

    #[test]
    fn only_hashes_are_kept_and_empty_secrets_are_ignored() {
        let marks = SecretMarks::default();
        marks.mark("");
        assert!(!marks.matches(""));
        marks.mark("correct horse");
        let entries = marks.entries.lock().expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, hash_of("correct horse"));
    }

    #[test]
    fn the_list_of_marks_is_bounded() {
        let marks = SecretMarks::default();
        let t0 = Instant::now();
        for i in 0..(MAX_MARKS + 10) {
            marks.mark_at(&format!("secret-{i}"), t0);
        }
        assert_eq!(marks.entries.lock().expect("entries").len(), MAX_MARKS);
        // The newest survive.
        assert!(marks.matches_at(&format!("secret-{}", MAX_MARKS + 9), t0));
        assert!(!marks.matches_at("secret-0", t0));
    }

    /// Reads and writes the REAL system clipboard: never run it while someone is working on
    /// the machine (`cargo test -- --ignored clipboard::tests::real_clipboard`).
    #[test]
    #[ignore = "touches the real system clipboard"]
    fn real_clipboard_round_trip() {
        let clipboard = SystemClipboard::default();
        clipboard.write_text("portolano test").expect("write");
        let content = clipboard.read().expect("read");
        assert_eq!(content.text.as_deref(), Some("portolano test"));
    }
}
