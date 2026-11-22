//! Password-protected backup file of one signing identity.
//!
//! JSON `{ format, version, kdf, salt, nonce, ciphertext }` (binary fields in base64). The key
//! is `Argon2id(password, salt)` through the vault's own `MasterPassword`, the cipher is the
//! vault's XChaCha20-Poly1305; the clear fields are bound as associated data.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::XNonce;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::VaultError;
use crate::format::{cipher, random_array, NONCE_LEN, SALT_LEN};
use crate::payload::CountingSink;
use crate::types::{clean_identity_name, MAX_IDENTITY_NAME_LEN};
use crate::unlock::{KdfParams, MasterPassword, UnlockMethod};

const FORMAT: &str = "portolano-identity";
const VERSION: u32 = 1;
/// A backup is a few hundred bytes; anything bigger is not one.
const MAX_BACKUP_LEN: usize = 64 * 1024;
/// Tighter than the vault's own caps: a backup comes from someone else and its key derivation
/// runs before it can be authenticated. Production values (64 MiB, 3, 1) fit well inside.
const MAX_BACKUP_MEMORY_KIB: u32 = 128 * 1024;
const MAX_BACKUP_ITERATIONS: u32 = 6;
const MAX_BACKUP_PARALLELISM: u32 = 4;

#[derive(Serialize, Deserialize)]
struct BackupFile {
    format: String,
    version: u32,
    kdf: KdfParams,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OutContent<'a> {
    name: &'a str,
    secret_key: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InContent {
    name: String,
    secret_key: SecretString,
}

/// Associated data: binds the ciphertext to the format, the version and the KDF parameters.
fn aad(kdf: &KdfParams, salt: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "{FORMAT}\0{VERSION}\0{}\0{}\0{}\0",
        kdf.memory_kib, kdf.iterations, kdf.parallelism
    )
    .into_bytes();
    out.extend_from_slice(salt);
    out
}

/// Encrypts `name` and `secret` under `password` with the production KDF parameters.
pub fn encrypt(
    name: &str,
    secret: &[u8; 32],
    password: &SecretString,
) -> Result<Vec<u8>, VaultError> {
    encrypt_with(name, secret, password, KdfParams::default())
}

pub(crate) fn encrypt_with(
    name: &str,
    secret: &[u8; 32],
    password: &SecretString,
    kdf: KdfParams,
) -> Result<Vec<u8>, VaultError> {
    if password.expose_secret().is_empty() {
        return Err(VaultError::InvalidInput(
            "the backup password must not be empty".into(),
        ));
    }
    let name = clean_identity_name(name)?;
    check_backup_kdf(&kdf)?;
    let salt = random_array::<SALT_LEN>()?;
    let nonce = random_array::<NONCE_LEN>()?;
    let key = MasterPassword(password.clone()).derive_kek(&kdf, &salt)?;

    let secret_text = Zeroizing::new(STANDARD.encode(secret));
    let serialize_failed = |_| VaultError::Internal("backup serialization failed".into());
    let content = OutContent {
        name,
        secret_key: &secret_text,
    };
    // Sized up front, so the buffer never reallocates and leaves unwiped copies behind.
    let mut sink = CountingSink(0);
    serde_json::to_writer(&mut sink, &content).map_err(serialize_failed)?;
    let mut plaintext = Zeroizing::new(Vec::with_capacity(sink.0));
    serde_json::to_writer(&mut *plaintext, &content).map_err(serialize_failed)?;
    let sealed = cipher(key.expose_secret())?
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &plaintext,
                aad: &aad(&kdf, &salt),
            },
        )
        .map_err(|_| VaultError::Internal("backup encryption failed".into()))?;

    serde_json::to_vec(&BackupFile {
        format: FORMAT.into(),
        version: VERSION,
        kdf,
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(sealed),
    })
    .map_err(serialize_failed)
}

fn check_backup_kdf(kdf: &KdfParams) -> Result<(), VaultError> {
    let in_range = kdf.validate().is_ok();
    if !in_range
        || kdf.memory_kib > MAX_BACKUP_MEMORY_KIB
        || kdf.iterations > MAX_BACKUP_ITERATIONS
        || kdf.parallelism > MAX_BACKUP_PARALLELISM
    {
        return Err(VaultError::Corrupt("not an identity backup"));
    }
    Ok(())
}

/// Opens a backup. Runs Argon2 (up to 128 MiB): call it off the UI thread. A wrong password and a modified file are the same `AuthFailed`.
pub fn decrypt(
    bytes: &[u8],
    password: &SecretString,
) -> Result<(String, Zeroizing<[u8; 32]>), VaultError> {
    const NOT_A_BACKUP: VaultError = VaultError::Corrupt("not an identity backup");
    if bytes.len() > MAX_BACKUP_LEN {
        return Err(NOT_A_BACKUP);
    }
    let file: BackupFile = serde_json::from_slice(bytes).map_err(|_| NOT_A_BACKUP)?;
    if file.format != FORMAT {
        return Err(NOT_A_BACKUP);
    }
    if file.version != VERSION {
        return Err(VaultError::UnsupportedVersion(
            u16::try_from(file.version).unwrap_or(u16::MAX),
        ));
    }
    let decode = |text: &str| STANDARD.decode(text).map_err(|_| NOT_A_BACKUP);
    let salt: [u8; SALT_LEN] = decode(&file.salt)?.try_into().map_err(|_| NOT_A_BACKUP)?;
    let nonce: [u8; NONCE_LEN] = decode(&file.nonce)?.try_into().map_err(|_| NOT_A_BACKUP)?;
    let ciphertext = decode(&file.ciphertext)?;

    check_backup_kdf(&file.kdf)?;
    let key = MasterPassword(password.clone()).derive_kek(&file.kdf, &salt)?;
    let plaintext = cipher(key.expose_secret())?
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &ciphertext,
                aad: &aad(&file.kdf, &salt),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| VaultError::AuthFailed)?;

    let content: InContent = serde_json::from_slice(&plaintext)
        .map_err(|_| VaultError::Corrupt("backup content is not valid"))?;
    let raw = Zeroizing::new(
        STANDARD
            .decode(content.secret_key.expose_secret())
            .map_err(|_| VaultError::Corrupt("backup content is not valid"))?,
    );
    let secret = <[u8; 32]>::try_from(raw.as_slice())
        .map_err(|_| VaultError::Corrupt("backup content is not valid"))?;
    let name = content.name.trim();
    if name.is_empty() || name.len() > MAX_IDENTITY_NAME_LEN {
        return Err(VaultError::Corrupt("backup content is not valid"));
    }
    Ok((name.to_owned(), Zeroizing::new(secret)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> KdfParams {
        KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        }
    }

    fn pw(s: &str) -> SecretString {
        SecretString::from(s)
    }

    #[test]
    fn backup_round_trips() {
        let secret = [7u8; 32];
        let bytes = encrypt_with("Alice's laptop", &secret, &pw("fake-pw"), tiny()).expect("enc");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(json["format"], "portolano-identity");
        assert_eq!(json["version"], 1);
        let (name, got) = decrypt(&bytes, &pw("fake-pw")).expect("dec");
        assert_eq!(name, "Alice's laptop");
        assert_eq!(*got, secret);
    }

    #[test]
    fn backup_with_wrong_password_fails_without_detail() {
        let bytes = encrypt_with("n", &[1u8; 32], &pw("fake-right"), tiny()).expect("enc");
        let err = decrypt(&bytes, &pw("fake-wrong")).expect_err("must fail");
        assert!(matches!(err, VaultError::AuthFailed));
        assert_eq!(err.to_string(), "authentication failed");
    }

    #[test]
    fn tampered_or_foreign_files_are_rejected() {
        let bytes = encrypt_with("n", &[1u8; 32], &pw("fake-pw"), tiny()).expect("enc");
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        json["kdf"]["iterations"] = 2.into();
        let tampered = serde_json::to_vec(&json).expect("ser");
        assert!(matches!(
            decrypt(&tampered, &pw("fake-pw")),
            Err(VaultError::AuthFailed)
        ));
        assert!(matches!(
            decrypt(b"{\"format\":\"other\"}", &pw("fake-pw")),
            Err(VaultError::Corrupt(_))
        ));
        assert!(decrypt(b"garbage", &pw("fake-pw")).is_err());
    }

    #[test]
    fn backup_with_excessive_kdf_params_is_rejected() {
        let bytes = encrypt_with("n", &[1u8; 32], &pw("fake-pw"), tiny()).expect("enc");
        for (field, value) in [
            ("memoryKib", 128 * 1024 + 8),
            ("iterations", 7),
            ("parallelism", 5),
        ] {
            let mut json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
            json["kdf"][field] = value.into();
            let hostile = serde_json::to_vec(&json).expect("ser");
            let err = decrypt(&hostile, &pw("fake-pw")).expect_err("must be rejected");
            assert!(matches!(err, VaultError::Corrupt("not an identity backup")));
        }
        // What `encrypt` writes by default is inside the bounds.
        assert!(check_backup_kdf(&KdfParams::default()).is_ok());
        assert!(encrypt_with(
            "n",
            &[1u8; 32],
            &pw("fake-pw"),
            KdfParams {
                memory_kib: 256 * 1024,
                iterations: 3,
                parallelism: 1
            }
        )
        .is_err());
    }

    #[test]
    fn backup_names_are_trimmed_and_bounded() {
        let ok = encrypt_with("  Carol  ", &[2u8; 32], &pw("fake-pw"), tiny()).expect("enc");
        assert_eq!(decrypt(&ok, &pw("fake-pw")).expect("dec").0, "Carol");
        for bad in ["", "   ", &"x".repeat(4097)] {
            assert!(encrypt_with(bad, &[2u8; 32], &pw("fake-pw"), tiny()).is_err());
        }
        // A backup written by someone else with an oversized, empty or blank name is refused
        // on read; a padded name is trimmed.
        for bad in [&"x".repeat(4097), "", "  \t "] {
            assert!(matches!(
                decrypt(&forge(bad), &pw("fake-pw")),
                Err(VaultError::Corrupt(_))
            ));
        }
        assert_eq!(
            decrypt(&forge("  Dave \n"), &pw("fake-pw")).expect("dec").0,
            "Dave"
        );
    }

    /// A backup as someone else's software could write it: any name, no validation.
    fn forge(name: &str) -> Vec<u8> {
        let salt = [0u8; SALT_LEN];
        let nonce = [0u8; NONCE_LEN];
        let kdf = tiny();
        let key = MasterPassword(pw("fake-pw"))
            .derive_kek(&kdf, &salt)
            .expect("kek");
        let content = serde_json::to_vec(&OutContent {
            name,
            secret_key: &STANDARD.encode([3u8; 32]),
        })
        .expect("ser");
        let sealed = cipher(key.expose_secret())
            .expect("cipher")
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &content,
                    aad: &aad(&kdf, &salt),
                },
            )
            .expect("seal");
        serde_json::to_vec(&BackupFile {
            format: FORMAT.into(),
            version: VERSION,
            kdf,
            salt: STANDARD.encode(salt),
            nonce: STANDARD.encode(nonce),
            ciphertext: STANDARD.encode(sealed),
        })
        .expect("ser")
    }
}
