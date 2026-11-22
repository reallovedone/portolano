//! Plaintext payload: the credentials as JSON. Only ever lives in `Zeroizing` buffers.

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
        .map(|i| Zeroizing::new(STANDARD.encode(*i.secret)))
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
    let identities = file
        .identities
        .into_iter()
        .map(decode_identity)
        .collect::<Result<Vec<_>, _>>()?;
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
    let secret = Zeroizing::new(<[u8; 32]>::try_from(raw.as_slice()).map_err(|_| BAD)?);
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
