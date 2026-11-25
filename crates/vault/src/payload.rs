//! Plaintext payload: the credentials as JSON. The whole text only ever lives in `Zeroizing`
//! buffers and each parsed secret goes straight into a `SecretString`. The JSON parser may keep
//! short-lived copies of a string while it reads it (to undo escapes), which this crate cannot
//! wipe.

use std::io;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::VaultError;
use crate::types::{Credential, Identity, IdentityRecord, Secret};

#[derive(Serialize)]
struct OutFile<'a> {
    credentials: Vec<OutCredential<'a>>,
    identities: Vec<OutIdentity<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OutIdentity<'a> {
    id: Uuid,
    name: &'a str,
    public_key: String,
    created_at: &'a str,
    secret_key: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OutCredential<'a> {
    id: Uuid,
    name: &'a str,
    username: &'a str,
    domain: Option<&'a str>,
    secret: OutSecret<'a>,
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum OutSecret<'a> {
    Password {
        password: &'a str,
    },
    SshKey {
        private_key_pem: &'a str,
        passphrase: Option<&'a str>,
    },
}

#[derive(Deserialize)]
struct InFile {
    credentials: Vec<InCredential>,
    /// Absent in payloads written before shared folders existed.
    #[serde(default)]
    identities: Vec<InIdentity>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InIdentity {
    id: Uuid,
    name: String,
    public_key: String,
    created_at: String,
    secret_key: SecretString,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InCredential {
    id: Uuid,
    name: String,
    username: String,
    domain: Option<String>,
    secret: InSecret,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum InSecret {
    Password {
        password: SecretString,
    },
    SshKey {
        private_key_pem: SecretString,
        passphrase: Option<SecretString>,
    },
}

/// Counts bytes without storing them, so the real buffer can be allocated once at its final
/// size (a growing `Vec` would leave unwiped copies of the secrets behind).
pub(crate) struct CountingSink(pub(crate) usize);

impl io::Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn encode(
    credentials: &[Credential],
    identities: &[Identity],
) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    // The base64 text of each secret key is a plaintext copy too: keep it wiped.
    let secret_texts: Vec<Zeroizing<String>> = identities
        .iter()
        .map(|i| Zeroizing::new(STANDARD.encode(i.secret.as_slice())))
        .collect();
    let file = OutFile {
        identities: identities
            .iter()
            .zip(&secret_texts)
            .map(|(i, secret)| OutIdentity {
                id: i.record.id,
                name: &i.record.name,
                public_key: STANDARD.encode(i.record.public_key),
                created_at: &i.record.created_at,
                secret_key: secret,
            })
            .collect(),
        credentials: credentials
            .iter()
            .map(|c| OutCredential {
                id: c.id,
                name: &c.name,
                username: &c.username,
                domain: c.domain.as_deref(),
                secret: match &c.secret {
                    Secret::Password(p) => OutSecret::Password {
                        password: p.expose_secret(),
                    },
                    Secret::SshKey {
                        private_key_pem,
                        passphrase,
                    } => OutSecret::SshKey {
                        private_key_pem: private_key_pem.expose_secret(),
                        passphrase: passphrase.as_ref().map(|p| p.expose_secret()),
                    },
                },
            })
            .collect(),
    };
    let serialize_failed = |_| VaultError::Internal("payload serialization failed".into());
    let mut sink = CountingSink(0);
    serde_json::to_writer(&mut sink, &file).map_err(serialize_failed)?;
    let mut buf = Zeroizing::new(Vec::with_capacity(sink.0));
    serde_json::to_writer(&mut *buf, &file).map_err(serialize_failed)?;
    Ok(buf)
}

/// A payload that authenticated but does not parse can only come from a bug or from a file
/// written by a different program with the right key: reported as damaged.
pub fn decode(bytes: &[u8]) -> Result<(Vec<Credential>, Vec<Identity>), VaultError> {
    let file: InFile =
        serde_json::from_slice(bytes).map_err(|_| VaultError::Corrupt("payload is not valid"))?;
    let mut identities: Vec<Identity> = Vec::with_capacity(file.identities.len());
    for decoded in file.identities.into_iter().map(decode_identity) {
        let identity = decoded?;
        // Same rules as `add_identity`: an id or a key already present is not loaded twice.
        let repeated = identities.iter().any(|i| {
            i.record.id == identity.record.id || i.record.public_key == identity.record.public_key
        });
        if repeated {
            tracing::warn!("an identity repeated in the vault was not loaded twice");
        } else {
            identities.push(identity);
        }
    }
    let credentials = file
        .credentials
        .into_iter()
        .map(|c| Credential {
            id: c.id,
            name: c.name,
            username: c.username,
            domain: c.domain,
            secret: match c.secret {
                InSecret::Password { password } => Secret::Password(password),
                InSecret::SshKey {
                    private_key_pem,
                    passphrase,
                } => Secret::SshKey {
                    private_key_pem,
                    passphrase,
                },
            },
        })
        .collect();
    Ok((credentials, identities))
}

fn decode_identity(i: InIdentity) -> Result<Identity, VaultError> {
    const BAD: VaultError = VaultError::Corrupt("payload is not valid");
    let public_key = STANDARD
        .decode(&i.public_key)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or(BAD)?;
    let raw = Zeroizing::new(
        STANDARD
            .decode(i.secret_key.expose_secret())
            .map_err(|_| BAD)?,
    );
    // Filled in place: building the array first would leave unwiped copies on the stack.
    let mut secret = Zeroizing::new([0u8; 32]);
    if raw.len() != secret.len() {
        return Err(BAD);
    }
    secret.copy_from_slice(&raw);
    Ok(Identity {
        record: IdentityRecord {
            id: i.id,
            name: i.name,
            public_key,
            created_at: i.created_at,
        },
        secret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: u128, name: &str, public: u8) -> Identity {
        Identity {
            record: IdentityRecord {
                id: Uuid::from_u128(id),
                name: name.into(),
                public_key: [public; 32],
                created_at: "2026-01-01T00:00:00Z".into(),
            },
            secret: Zeroizing::new([7u8; 32]),
        }
    }

    #[test]
    fn identities_repeated_in_a_payload_are_loaded_once() {
        let list = [
            identity(1, "first", 1),
            identity(1, "same id", 2),
            identity(3, "same key", 1),
            identity(4, "other", 4),
        ];
        let bytes = encode(&[], &list).expect("encode");
        let (_, loaded) = decode(&bytes).expect("decode");
        let names: Vec<&str> = loaded.iter().map(|i| i.record.name.as_str()).collect();
        assert_eq!(names, ["first", "other"]);
    }
}
