use std::fmt;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

/// A stored credential. Never serialized: it only exists in memory while the vault is unlocked.
#[derive(Debug, Clone)]
pub struct Credential {
    pub id: Uuid,
    pub name: String,
    pub username: String,
    pub domain: Option<String>,
    pub secret: Secret,
}

/// `SecretString` redacts itself in `Debug` and zeroizes on drop.
#[derive(Debug, Clone)]
pub enum Secret {
    Password(SecretString),
    SshKey {
        private_key_pem: SecretString,
        passphrase: Option<SecretString>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialKind {
    Password,
    SshKey,
}

impl Secret {
    pub fn kind(&self) -> CredentialKind {
        match self {
            Self::Password(_) => CredentialKind::Password,
            Self::SshKey { .. } => CredentialKind::SshKey,
        }
    }
}

/// What the UI sees of a credential: no secret material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSummary {
    pub id: Uuid,
    pub name: String,
    pub username: String,
    pub domain: Option<String>,
    pub kind: CredentialKind,
}

impl From<&Credential> for CredentialSummary {
    fn from(c: &Credential) -> Self {
        Self {
            id: c.id,
            name: c.name.clone(),
            username: c.username.clone(),
            domain: c.domain.clone(),
            kind: c.secret.kind(),
        }
    }
}

/// Input of `vault_upsert`. `id: None` creates a credential. Secrets arrive from the UI as
/// strings and are wrapped in `SecretString` when deserialized (the JSON parser may briefly
/// hold its own copy of the text while it parses, which this crate cannot wipe).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInput {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub name: String,
    pub username: String,
    #[serde(default)]
    pub domain: Option<String>,
    pub secret: SecretInput,
}

/// Input of `vault_upsert` as sent by the UI: like [`CredentialInput`], but `secret` may be
/// absent or `null` when editing, meaning "keep the current secret". Creating a credential
/// (no `id`, or an id that does not exist) still requires it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialUpsert {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub name: String,
    pub username: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub secret: Option<SecretInput>,
}

impl From<CredentialInput> for CredentialUpsert {
    fn from(input: CredentialInput) -> Self {
        Self {
            id: input.id,
            name: input.name,
            username: input.username,
            domain: input.domain,
            secret: Some(input.secret),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SecretInput {
    Password {
        password: SecretString,
    },
    SshKey {
        private_key_pem: SecretString,
        #[serde(default)]
        passphrase: Option<SecretString>,
    },
}

impl From<SecretInput> for Secret {
    fn from(s: SecretInput) -> Self {
        match s {
            SecretInput::Password { password } => Secret::Password(password),
            SecretInput::SshKey {
                private_key_pem,
                passphrase,
            } => Secret::SshKey {
                private_key_pem,
                passphrase,
            },
        }
    }
}

/// Output of `vault_reveal_secret`: the only place where secret material leaves the backend,
/// and only on an explicit user action. Plain strings because it must be JSON-serialized;
/// `Debug` is redacted.
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RevealedSecret {
    Password {
        password: String,
    },
    SshKey {
        private_key_pem: String,
        passphrase: Option<String>,
    },
}

impl fmt::Debug for RevealedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RevealedSecret(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultStatus {
    NotCreated,
    Locked,
    Unlocked,
}

/// A signing identity as the UI sees it: everything except the secret key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRecord {
    pub id: Uuid,
    pub name: String,
    /// Ed25519 public key.
    pub public_key: [u8; 32],
    /// RFC 3339 UTC time of creation.
    pub created_at: String,
}

/// Longest identity name, in bytes.
pub(crate) const MAX_IDENTITY_NAME_LEN: usize = 4 * 1024;

/// Trims an identity name and checks it is 1 to 4096 bytes. Shared by the vault and the backup.
pub(crate) fn clean_identity_name(name: &str) -> Result<&str, crate::error::VaultError> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_IDENTITY_NAME_LEN {
        return Err(crate::error::VaultError::InvalidInput(
            "the identity name must be 1 to 4096 bytes".into(),
        ));
    }
    Ok(name)
}

/// An identity with its secret key; lives only inside the unlocked vault.
pub struct Identity {
    pub record: IdentityRecord,
    pub secret: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for Identity {
    /// Never prints the secret key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn status_json_shape() {
        assert_eq!(
            serde_json::to_value(VaultStatus::NotCreated).expect("serialize"),
            json!("notCreated")
        );
        assert_eq!(
            serde_json::to_value(VaultStatus::Locked).expect("serialize"),
            json!("locked")
        );
        assert_eq!(
            serde_json::to_value(VaultStatus::Unlocked).expect("serialize"),
            json!("unlocked")
        );
    }

    #[test]
    fn summary_json_shape() {
        let s = CredentialSummary {
            id: Uuid::from_u128(1),
            name: "n".into(),
            username: "u".into(),
            domain: None,
            kind: CredentialKind::SshKey,
        };
        assert_eq!(
            serde_json::to_value(s).expect("serialize"),
            json!({
                "id": "00000000-0000-0000-0000-000000000001",
                "name": "n", "username": "u", "domain": null, "kind": "sshKey"
            })
        );
    }

    #[test]
    fn input_json_shapes() {
        let input: CredentialInput = serde_json::from_value(json!({
            "name": "n", "username": "u",
            "secret": {"type": "password", "password": "pw"}
        }))
        .expect("parse password");
        assert!(input.id.is_none());
        assert!(matches!(input.secret, SecretInput::Password { .. }));

        let input: CredentialInput = serde_json::from_value(json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "n", "username": "u", "domain": "CORP",
            "secret": {"type": "sshKey", "privateKeyPem": "pem"}
        }))
        .expect("parse key");
        assert!(matches!(
            input.secret,
            SecretInput::SshKey {
                passphrase: None,
                ..
            }
        ));
    }

    #[test]
    fn upsert_input_accepts_a_missing_or_null_secret() {
        for body in [
            json!({"id": "00000000-0000-0000-0000-000000000001", "name": "n", "username": "u"}),
            json!({"id": "00000000-0000-0000-0000-000000000001", "name": "n", "username": "u", "secret": null}),
        ] {
            let input: CredentialUpsert = serde_json::from_value(body).expect("parse");
            assert!(input.secret.is_none());
        }
        let input: CredentialUpsert = serde_json::from_value(json!({
            "name": "n", "username": "u",
            "secret": {"type": "password", "password": "pw"}
        }))
        .expect("parse");
        assert!(matches!(input.secret, Some(SecretInput::Password { .. })));
    }

    #[test]
    fn revealed_secret_json_shape_and_redacted_debug() {
        let revealed = RevealedSecret::SshKey {
            private_key_pem: "pem".into(),
            passphrase: None,
        };
        assert_eq!(
            serde_json::to_value(&revealed).expect("serialize"),
            json!({"type": "sshKey", "privateKeyPem": "pem", "passphrase": null})
        );
        assert!(!format!("{revealed:?}").contains("pem"));
    }

    #[test]
    fn debug_of_secret_does_not_leak() {
        let secret = Secret::Password(SecretString::from("hunter2"));
        assert!(!format!("{secret:?}").contains("hunter2"));
    }
}
