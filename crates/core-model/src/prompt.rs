//! Credential prompts (WP11): what a session asks the user when the connection has no
//! credential, how the answer travels back, and what is reported when the user wants to keep
//! it in the vault.
//!
//! The prompt itself is plain data and goes to the UI as a JSON-bodied frame of the Terminal
//! or Framebuffer family (`TermEvent::CredentialPrompt`, `FbEvent::CredentialPrompt`). The
//! answer holds secrets: it only ever travels UI -> backend, lives in `SecretString`s, is
//! used for one session and is never serialized, logged or echoed in an error message.

use std::sync::{Arc, Mutex};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::error::AppError;

/// Which fields the prompt asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptKind {
    /// A secret only, no user name (VNC).
    Password,
    /// SSH: user name, then a password or a private key file (with an optional passphrase).
    SshKey,
    /// User name and password, plus an optional domain (RDP).
    UsernamePassword,
}

/// The prompt shown to the user. Contains no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialPrompt {
    pub kind: PromptKind,
    /// Pre-filled user name (the connection's `username`, or what was typed before).
    pub username_hint: Option<String>,
    /// Pre-filled domain (RDP only).
    pub domain_hint: Option<String>,
    /// What the credential is for: `host:port` (for a jump host, the hop).
    pub target: String,
    /// Why the previous answer did not work (never contains secrets).
    pub error: Option<String>,
    /// The dialog may offer "save to vault" (a vault exists and is unlocked).
    pub can_save_to_vault: bool,
}

/// What the user typed. Deserialized from the `session_credential` command.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialAnswer {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: Option<SecretString>,
    /// Path of a private key file; the backend reads it, the UI never sees its content.
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub passphrase: Option<SecretString>,
    #[serde(default)]
    pub domain: Option<String>,
}

impl std::fmt::Debug for CredentialAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialAnswer(<redacted>)")
    }
}

/// The reply of the user to a prompt.
#[derive(Debug)]
pub enum PromptReply {
    Answer {
        answer: CredentialAnswer,
        save_to_vault: bool,
    },
    Cancel,
}

/// A credential the user typed and asked to keep, handed to the host application once the
/// authentication succeeded.
pub struct SavedCredential {
    pub username: String,
    pub domain: Option<String>,
    pub secret: SavedSecret,
}

pub enum SavedSecret {
    Password(SecretString),
    SshKey {
        private_key_pem: SecretString,
        passphrase: Option<SecretString>,
    },
}

impl std::fmt::Debug for SavedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SavedCredential(<redacted>)")
    }
}

/// What the host application plugs into a session that may prompt.
#[derive(Clone)]
pub struct PromptHooks {
    /// Whether "save to vault" should be offered right now.
    pub can_save: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Called (off the async workers) after a successful authentication with a credential
    /// the user chose to keep.
    pub saved: Arc<dyn Fn(SavedCredential) + Send + Sync>,
}

impl PromptHooks {
    /// Hooks that never offer saving (tests, or hosts without a vault).
    pub fn no_vault() -> Self {
        Self {
            can_save: Arc::new(|| false),
            saved: Arc::new(|_| {}),
        }
    }
}

impl std::fmt::Debug for PromptHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PromptHooks")
    }
}

/// The place where a session waits for the answer to its (single) pending prompt.
#[derive(Default)]
pub struct PromptSlot {
    pending: Mutex<Option<oneshot::Sender<PromptReply>>>,
}

impl PromptSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arms the slot for a new prompt. A reply sent while nothing was armed is rejected, so a
    /// stale answer can never satisfy a later prompt.
    pub fn arm(&self) -> oneshot::Receiver<PromptReply> {
        let (tx, rx) = oneshot::channel();
        match self.pending.lock() {
            Ok(mut slot) => *slot = Some(tx),
            Err(e) => *e.into_inner() = Some(tx),
        }
        rx
    }

    /// Delivers the reply to the waiting session.
    pub fn reply(&self, reply: PromptReply) -> Result<(), AppError> {
        let sender = match self.pending.lock() {
            Ok(mut slot) => slot.take(),
            Err(e) => e.into_inner().take(),
        }
        .ok_or_else(|| AppError::invalid_input("no credential prompt is pending"))?;
        sender
            .send(reply)
            .map_err(|_| AppError::invalid_input("the credential prompt is no longer pending"))
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;
    use serde_json::json;

    use super::*;

    #[test]
    fn prompt_json_shape() {
        let prompt = CredentialPrompt {
            kind: PromptKind::SshKey,
            username_hint: Some("tester".into()),
            domain_hint: None,
            target: "jump.example:22".into(),
            error: None,
            can_save_to_vault: true,
        };
        assert_eq!(
            serde_json::to_value(&prompt).expect("serialize"),
            json!({
                "kind": "sshKey",
                "usernameHint": "tester",
                "domainHint": null,
                "target": "jump.example:22",
                "error": null,
                "canSaveToVault": true
            })
        );
        for (kind, text) in [
            (PromptKind::Password, "password"),
            (PromptKind::UsernamePassword, "usernamePassword"),
        ] {
            assert_eq!(serde_json::to_value(kind).expect("kind"), json!(text));
        }
    }

    #[test]
    fn answer_parses_and_never_prints_its_secrets() {
        let answer: CredentialAnswer = serde_json::from_value(json!({
            "username": "bob",
            "password": "hunter2-secret",
            "keyPath": null,
            "passphrase": "pp-secret",
            "domain": "CORP"
        }))
        .expect("parse");
        assert_eq!(answer.username, "bob");
        assert_eq!(
            answer
                .password
                .as_ref()
                .map(|p| p.expose_secret().to_owned()),
            Some("hunter2-secret".to_owned())
        );
        let debug = format!("{answer:?} {:?}", PromptReply::Cancel);
        assert!(!debug.contains("hunter2") && !debug.contains("pp-secret"));
        // Every field but the user name is optional.
        let minimal: CredentialAnswer = serde_json::from_value(json!({})).expect("parse");
        assert!(minimal.username.is_empty() && minimal.password.is_none());
    }

    #[tokio::test]
    async fn a_reply_reaches_only_the_armed_prompt() {
        let slot = PromptSlot::new();
        // Nothing armed yet: the reply is refused.
        assert!(slot.reply(PromptReply::Cancel).is_err());
        let rx = slot.arm();
        slot.reply(PromptReply::Cancel).expect("reply");
        assert!(matches!(rx.await, Ok(PromptReply::Cancel)));
        // Consumed: a second reply is refused again.
        assert!(slot.reply(PromptReply::Cancel).is_err());
    }
}
