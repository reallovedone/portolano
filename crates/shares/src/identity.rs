//! Signing identities and the public codes used to exchange their public halves.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use ed25519_dalek::{SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::ShareError;

const CODE_PREFIX: &str = "portolano-id";
const CODE_VERSION: &str = "1";

/// A named signing identity. The secret half never leaves this type except through the vault
/// (which derives and stores it); `Debug` shows only the public key.
#[derive(Clone)]
pub struct Identity {
    pub id: Uuid,
    pub name: String,
    pub key: SigningKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("public", &fingerprint(&self.key.verifying_key()))
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// A new identity with a random key from the operating system.
    pub fn generate(name: &str) -> Result<Identity, ShareError> {
        let mut secret = Zeroizing::new([0u8; 32]);
        getrandom::fill(secret.as_mut()).map_err(|_| ShareError::NoRandomSource)?;
        Ok(Identity::from_secret(Uuid::new_v4(), name, &secret))
    }

    pub fn from_secret(id: Uuid, name: &str, secret: &[u8; 32]) -> Identity {
        Identity {
            id,
            name: name.to_owned(),
            key: SigningKey::from_bytes(secret),
        }
    }

    pub fn public(&self) -> PublicIdentity {
        PublicIdentity {
            name: self.name.clone(),
            key: self.key.verifying_key(),
        }
    }
}

/// The public half: a display name and a verifying key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicIdentity {
    pub name: String,
    pub key: VerifyingKey,
}

/// First 8 bytes of the SHA-256 of the public key, uppercase hex in groups of four
/// (`AAAA-BBBB-CCCC-DDDD`), for people to compare out of band.
pub fn fingerprint(key: &VerifyingKey) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02X}")).collect();
    hex.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

/// `portolano-id:1:<base64url(name)>:<base64url(key)>`, no padding.
pub fn encode_public_code(identity: &PublicIdentity) -> String {
    format!(
        "{CODE_PREFIX}:{CODE_VERSION}:{}:{}",
        URL_SAFE_NO_PAD.encode(identity.name.as_bytes()),
        URL_SAFE_NO_PAD.encode(identity.key.as_bytes())
    )
}

pub fn parse_public_code(code: &str) -> Result<PublicIdentity, ShareError> {
    let parts: Vec<&str> = code.trim().split(':').collect();
    let [prefix, version, name, key] = parts.as_slice() else {
        return Err(ShareError::InvalidPublicCode);
    };
    if *prefix != CODE_PREFIX || *version != CODE_VERSION {
        return Err(ShareError::InvalidPublicCode);
    }
    let name = URL_SAFE_NO_PAD
        .decode(name)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|n| !n.trim().is_empty())
        .ok_or(ShareError::InvalidPublicCode)?;
    let key = URL_SAFE_NO_PAD
        .decode(key)
        .map_err(|_| ShareError::InvalidPublicCode)?;
    let key = verifying_key_from_slice(&key).ok_or(ShareError::InvalidPublicCode)?;
    Ok(PublicIdentity { name, key })
}

fn verifying_key_from_slice(bytes: &[u8]) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

/// Standard base64 of a public key, the form stored in files and in the document.
pub fn encode_key(key: &VerifyingKey) -> String {
    STANDARD.encode(key.as_bytes())
}

/// Inverse of [`encode_key`]; `None` when the text is not a valid 32-byte public key.
pub fn decode_key(text: &str) -> Option<VerifyingKey> {
    verifying_key_from_slice(&STANDARD.decode(text).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_has_four_groups_of_four_hex() {
        let fp = fingerprint(&Identity::generate("Ann").expect("rng").key.verifying_key());
        let groups: Vec<&str> = fp.split('-').collect();
        assert_eq!(groups.len(), 4);
        for g in groups {
            assert_eq!(g.len(), 4);
            assert!(g
                .chars()
                .all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)));
        }
    }

    #[test]
    fn public_code_round_trips_and_rejects_bad_prefix() {
        let public = Identity::generate("Zoë Müller").expect("rng").public();
        let code = encode_public_code(&public);
        assert!(code.starts_with("portolano-id:1:"));
        assert!(!code.contains('=') && !code.contains('+') && !code.contains('/'));
        assert_eq!(parse_public_code(&code).expect("parse"), public);

        let bad = code.replacen("portolano-id", "other-id", 1);
        assert_eq!(parse_public_code(&bad), Err(ShareError::InvalidPublicCode));
        let bad = code.replacen(":1:", ":2:", 1);
        assert_eq!(parse_public_code(&bad), Err(ShareError::InvalidPublicCode));
        assert!(parse_public_code("portolano-id:1:QW5u").is_err());
        assert!(parse_public_code("portolano-id:1:QW5u:AAAA").is_err());
    }

    fn known_key() -> VerifyingKey {
        // RFC 8032 section 7.1, test 1 public key.
        let bytes: [u8; 32] = [
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
            0xf7, 0x07, 0x51, 0x1a,
        ];
        VerifyingKey::from_bytes(&bytes).expect("valid key")
    }

    #[test]
    fn fingerprint_known_answer() {
        // Computed independently on the raw 32 bytes:
        //   echo d75a...511a | xxd -r -p | shasum -a 256 | cut -c1-16  ->  21fe31dfa154a261
        assert_eq!(fingerprint(&known_key()), "21FE-31DF-A154-A261");
    }

    #[test]
    fn public_code_known_answer() {
        // Independently: base64url without padding of "Ann" and of the raw key bytes
        //   (`base64 | tr '+/' '-_' | tr -d '='`).
        let public = PublicIdentity {
            name: "Ann".into(),
            key: known_key(),
        };
        assert_eq!(
            encode_public_code(&public),
            "portolano-id:1:QW5u:11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"
        );
    }

    #[test]
    fn debug_does_not_show_the_secret() {
        let secret = [7u8; 32];
        let id = Identity::from_secret(Uuid::nil(), "Ann", &secret);
        let text = format!("{id:?}");
        assert!(!text.contains("secret") && !text.contains("[7, 7"));
    }
}
