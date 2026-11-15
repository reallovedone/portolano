//! Plaintext payload: the credentials as JSON. Only ever lives in `Zeroizing` buffers.

use std::io;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::VaultError;
use crate::types::{Credential, Secret};

#[derive(Serialize)]
struct OutFile<'a> {
    credentials: Vec<OutCredential<'a>>,
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
struct CountingSink(usize);

impl io::Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn encode(credentials: &[Credential]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let file = OutFile {
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
pub fn decode(bytes: &[u8]) -> Result<Vec<Credential>, VaultError> {
    let file: InFile =
        serde_json::from_slice(bytes).map_err(|_| VaultError::Corrupt("payload is not valid"))?;
    Ok(file
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
        .collect())
}
