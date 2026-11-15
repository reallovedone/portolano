//! Terminal family: local PTY, SSH and the known_hosts store (WP5).
//!
//! The crate knows nothing about Tauri. A [`Session`] pushes encoded `TermEvent` frames into a
//! [`FrameSink`] and is driven through `write`, `ack`, `resize`, `host_key_decision`, `close`.
//! Flow control (credit window, coalescing) lives in [`flow`]; the protocol backends are
//! [`pty`] (local shell) and [`ssh`].

mod flow;
mod known_hosts;
mod pty;
mod session;
mod ssh;

use portolano_core_model::fb::json_frame;
use portolano_core_model::prompt::CredentialPrompt;
use portolano_core_model::AppError;
use serde::{Deserialize, Serialize};

pub use known_hosts::{HostKeyStatus, KnownHosts, StoredKey};
pub use session::{FrameSink, LocalSpec, Session, SessionSpec};
pub use ssh::{SshAuth, SshHop, SshSpec};
// Prompt types are shared with the RDP crate and the app.
pub use portolano_core_model::prompt;

/// The backend keeps at most this many bytes sent and not yet confirmed by `term_ack`.
pub const MAX_UNACKED_BYTES: usize = 1024 * 1024;
/// Outgoing data is coalesced up to roughly this size per message.
pub const COALESCE_BYTES: usize = 64 * 1024;

/// Wire tags of `TermEvent` frames. Layout: `[tag u8][body]`; `Data` carries raw bytes, the
/// other events carry UTF-8 JSON. The TypeScript decoder is `ui/src/lib/ipc/frames.ts`.
pub const TAG_DATA: u8 = 0;
pub const TAG_HOST_KEY_PROMPT: u8 = 1;
pub const TAG_EXIT: u8 = 2;
pub const TAG_ERROR: u8 = 3;
/// JSON of a `CredentialPrompt` (WP11).
pub const TAG_CREDENTIAL_PROMPT: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    Data(Vec<u8>),
    /// Unknown host key only. A changed key fails with `host_key_mismatch` instead.
    HostKeyPrompt {
        fingerprint: String,
        algorithm: String,
        /// The host and port of the hop whose key this is: with jump hosts the first prompt is
        /// for the first jump host, not for the final target (additive fields).
        host: String,
        port: u16,
    },
    Exit {
        code: Option<i32>,
    },
    Error(AppError),
    /// The connection has no credential (or the last one failed): ask the user.
    CredentialPrompt(CredentialPrompt),
}

impl TermEvent {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Data(bytes) => {
                let mut out = Vec::with_capacity(1 + bytes.len());
                out.push(TAG_DATA);
                out.extend_from_slice(bytes);
                out
            }
            Self::HostKeyPrompt {
                fingerprint,
                algorithm,
                host,
                port,
            } => json_frame(
                TAG_HOST_KEY_PROMPT,
                &serde_json::json!({
                    "fingerprint": fingerprint,
                    "algorithm": algorithm,
                    "host": host,
                    "port": port,
                }),
            ),
            Self::Exit { code } => json_frame(TAG_EXIT, &serde_json::json!({"code": code})),
            Self::Error(err) => json_frame(TAG_ERROR, err),
            Self::CredentialPrompt(prompt) => json_frame(TAG_CREDENTIAL_PROMPT, prompt),
        }
    }
}

/// One saved host key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostEntry {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

/// Errors of the known_hosts store.
#[derive(Debug, thiserror::Error)]
pub enum TermError {
    #[error("known_hosts i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

impl From<TermError> for AppError {
    fn from(e: TermError) -> Self {
        let code = match e {
            TermError::Io(_) => portolano_core_model::ErrorCode::Io,
            TermError::Invalid(_) => portolano_core_model::ErrorCode::InvalidInput,
        };
        AppError::new(code, e.to_string())
    }
}

/// Lists the entries of the known_hosts file at `path` (empty when the file does not exist).
pub fn known_hosts_list(path: &std::path::Path) -> Result<Vec<KnownHostEntry>, TermError> {
    KnownHosts::new(path).list()
}

/// Removes every key stored for `host:port`. Not an error when nothing was stored.
pub fn known_hosts_remove(path: &std::path::Path, host: &str, port: u16) -> Result<(), TermError> {
    KnownHosts::new(path).remove(host, port).map(|_| ())
}

#[cfg(test)]
mod tests {
    use portolano_core_model::ErrorCode;

    use super::*;

    #[test]
    fn data_frame_is_tag_plus_raw_bytes() {
        assert_eq!(TermEvent::Data(vec![1, 2, 3]).encode(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn json_frames() {
        let frame = TermEvent::HostKeyPrompt {
            fingerprint: "SHA256:x".into(),
            algorithm: "ssh-ed25519".into(),
            host: "jump.example".into(),
            port: 2222,
        }
        .encode();
        assert_eq!(frame[0], TAG_HOST_KEY_PROMPT);
        let json: serde_json::Value = serde_json::from_slice(&frame[1..]).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "fingerprint": "SHA256:x",
                "algorithm": "ssh-ed25519",
                "host": "jump.example",
                "port": 2222
            })
        );

        let frame = TermEvent::Exit { code: Some(3) }.encode();
        assert_eq!(frame[0], TAG_EXIT);
        let json: serde_json::Value = serde_json::from_slice(&frame[1..]).expect("json");
        assert_eq!(json, serde_json::json!({"code": 3}));

        let frame = TermEvent::Error(AppError::new(ErrorCode::HostKeyMismatch, "m")).encode();
        assert_eq!(frame[0], TAG_ERROR);
    }

    #[test]
    fn credential_prompt_frame_layout() {
        use portolano_core_model::prompt::PromptKind;
        let frame = TermEvent::CredentialPrompt(CredentialPrompt {
            kind: PromptKind::SshKey,
            username_hint: Some("tester".into()),
            domain_hint: None,
            target: "h:22".into(),
            error: None,
            can_save_to_vault: false,
        })
        .encode();
        assert_eq!(frame[0], TAG_CREDENTIAL_PROMPT);
        assert_eq!(frame[0], 4);
        // The exact bytes; the TypeScript decoder test (ui/src/lib/ipc/frames.test.ts) decodes
        // this same vector (with `canSaveToVault` false and no error).
        let expected = r#"{"kind":"sshKey","usernameHint":"tester","domainHint":null,"target":"h:22","error":null,"canSaveToVault":false}"#;
        assert_eq!(&frame[1..], expected.as_bytes());
        let json: serde_json::Value = serde_json::from_slice(&frame[1..]).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "sshKey", "usernameHint": "tester", "domainHint": null,
                "target": "h:22", "error": null, "canSaveToVault": false
            })
        );
    }

    #[test]
    fn known_host_entry_json_shape() {
        let e = KnownHostEntry {
            host: "h".into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:x".into(),
        };
        assert_eq!(
            serde_json::to_value(e).expect("serialize"),
            serde_json::json!({"host": "h", "port": 22, "algorithm": "ssh-ed25519", "fingerprint": "SHA256:x"})
        );
    }
}
