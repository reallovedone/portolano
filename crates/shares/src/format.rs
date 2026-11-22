//! The signed share file: a small JSON envelope around a base64 payload and its Ed25519
//! signature. The signature covers the exact payload bytes (no canonicalisation).

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use portolano_core_model::Node;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ShareError;
use crate::identity::{decode_key, encode_key, Identity};

pub const FORMAT_ID: &str = "portolano-share";
pub const FORMAT_VERSION: u64 = 1;
/// Largest share file read or written, checked before anything is parsed.
pub const MAX_SHARE_BYTES: usize = 5 * 1024 * 1024;

const EDITORS_DOMAIN: &[u8] = b"portolano-share-editors-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    pub name: String,
    /// Standard base64 of the 32-byte public key.
    pub key: String,
}

impl KeyEntry {
    pub fn new(name: &str, key: &VerifyingKey) -> KeyEntry {
        KeyEntry {
            name: name.to_owned(),
            key: encode_key(key),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorList {
    pub version: u64,
    pub keys: Vec<KeyEntry>,
    /// Standard base64 of the owner's signature over [`editors_signing_bytes`].
    pub owner_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePayload {
    pub share_id: Uuid,
    pub name: String,
    pub revision: u64,
    /// RFC 3339.
    pub published_at: String,
    pub signer_name: String,
    pub owner: KeyEntry,
    pub editors: EditorList,
    pub nodes: Vec<Node>,
}

/// A share file whose envelope and signature were verified. The content is still untrusted:
/// run it through [`crate::accept`] and [`crate::sanitize`].
#[derive(Debug, Clone)]
pub struct SignedShare {
    pub payload: SharePayload,
    pub signer: VerifyingKey,
    /// The file exactly as read, for the offline cache.
    pub raw: Vec<u8>,
    /// SHA-256 of the decoded payload bytes: tells two files with the same content apart from
    /// two that merely claim the same revision.
    pub payload_sha256: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct Header {
    format: String,
    version: u64,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u64,
    /// Standard base64 of the payload JSON.
    payload: String,
    /// Standard base64 of the signer's public key.
    signer: String,
    /// Standard base64 of the Ed25519 signature over the decoded payload bytes.
    signature: String,
}

/// The bytes the owner signs for an editor list; `None` when a key is not a 32-byte base64 key
/// or a length does not fit. Layout: domain tag, share id (16), version (u64 BE), count (u32 BE),
/// then per key the name length (u32 BE), the name and the 32 key bytes.
pub fn editors_signing_bytes(share_id: Uuid, version: u64, keys: &[KeyEntry]) -> Option<Vec<u8>> {
    let mut out = EDITORS_DOMAIN.to_vec();
    out.extend_from_slice(share_id.as_bytes());
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&u32::try_from(keys.len()).ok()?.to_be_bytes());
    for entry in keys {
        let key = decode_key(&entry.key)?;
        out.extend_from_slice(&u32::try_from(entry.name.len()).ok()?.to_be_bytes());
        out.extend_from_slice(entry.name.as_bytes());
        out.extend_from_slice(key.as_bytes());
    }
    Some(out)
}

/// The owner's signature (standard base64) over an editor list. An entry with an invalid key
/// yields an empty string, which no verification accepts.
pub fn sign_editors(share_id: Uuid, version: u64, keys: &[KeyEntry], owner: &Identity) -> String {
    match editors_signing_bytes(share_id, version, keys) {
        Some(bytes) => STANDARD.encode(owner.key.sign(&bytes).to_bytes()),
        None => String::new(),
    }
}

/// Serialises and signs `payload` as `signer`.
pub fn encode(payload: &SharePayload, signer: &Identity) -> Result<Vec<u8>, ShareError> {
    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|_| ShareError::Malformed("the share cannot be written".into()))?;
    let signature = signer.key.sign(&payload_bytes);
    let envelope = Envelope {
        format: FORMAT_ID.into(),
        version: FORMAT_VERSION,
        payload: STANDARD.encode(&payload_bytes),
        signer: encode_key(&signer.key.verifying_key()),
        signature: STANDARD.encode(signature.to_bytes()),
    };
    let bytes = serde_json::to_vec(&envelope)
        .map_err(|_| ShareError::Malformed("the share cannot be written".into()))?;
    if bytes.len() > MAX_SHARE_BYTES {
        return Err(ShareError::TooLarge);
    }
    Ok(bytes)
}

/// Checks the size, the envelope and the signature, then parses the payload.
pub fn decode(bytes: &[u8]) -> Result<SignedShare, ShareError> {
    if bytes.len() > MAX_SHARE_BYTES {
        return Err(ShareError::TooLarge);
    }
    let header: Header = serde_json::from_slice(bytes).map_err(|_| ShareError::NotAShare)?;
    if header.format != FORMAT_ID {
        return Err(ShareError::NotAShare);
    }
    if header.version != FORMAT_VERSION {
        return Err(ShareError::UnsupportedVersion(header.version));
    }
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| ShareError::Malformed("the envelope is incomplete".into()))?;
    let payload_bytes = STANDARD
        .decode(&envelope.payload)
        .map_err(|_| ShareError::Malformed("the content is not valid base64".into()))?;
    let signer = decode_key(&envelope.signer)
        .ok_or_else(|| ShareError::Malformed("the signer key is not valid".into()))?;
    let signature = STANDARD
        .decode(&envelope.signature)
        .ok()
        .and_then(|b| Signature::from_slice(&b).ok())
        .ok_or(ShareError::BadSignature)?;
    signer
        .verify_strict(&payload_bytes, &signature)
        .map_err(|_| ShareError::BadSignature)?;
    let payload: SharePayload = serde_json::from_slice(&payload_bytes)
        .map_err(|_| ShareError::Malformed("the content is not a shared folder".into()))?;
    Ok(SignedShare {
        payload,
        signer,
        raw: bytes.to_vec(),
        payload_sha256: Sha256::digest(&payload_bytes).into(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use portolano_core_model::{CredentialRef, Folder};

    pub(crate) fn sample(owner: &Identity, share_id: Uuid, revision: u64) -> SharePayload {
        let keys = Vec::new();
        SharePayload {
            share_id,
            name: "Prod".into(),
            revision,
            published_at: "2026-11-20T10:00:00Z".into(),
            signer_name: owner.name.clone(),
            owner: KeyEntry::new(&owner.name, &owner.key.verifying_key()),
            editors: EditorList {
                version: 1,
                owner_signature: sign_editors(share_id, 1, &keys, owner),
                keys,
            },
            nodes: vec![Node::Folder(Folder {
                id: Uuid::from_u128(1),
                parent_id: None,
                name: "Prod".into(),
                credential: CredentialRef::None,
                sort_index: None,
                icon: None,
                color: None,
            })],
        }
    }

    #[test]
    fn editors_signing_bytes_layout() {
        let share_id = Uuid::from_u128(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        let key_a = VerifyingKey::from_bytes(&[
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
            0xf7, 0x07, 0x51, 0x1a,
        ])
        .expect("key a");
        let key_b = VerifyingKey::from_bytes(&[
            0x3d, 0x40, 0x17, 0xc3, 0xe8, 0x43, 0x89, 0x5a, 0x92, 0xb7, 0x0a, 0xa7, 0x4d, 0x1b,
            0x7e, 0xbc, 0x9c, 0x98, 0x2c, 0xcf, 0x2e, 0xc4, 0x96, 0x8c, 0xc0, 0xcd, 0x55, 0xf1,
            0x2a, 0xf4, 0x66, 0x0c,
        ])
        .expect("key b");
        let keys = vec![KeyEntry::new("Bob", &key_a), KeyEntry::new("Zoë", &key_b)];
        let mut expected = b"portolano-share-editors-v1".to_vec();
        expected.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 2]); // version 258, u64 BE
        expected.extend_from_slice(&[0, 0, 0, 2]); // count, u32 BE
        expected.extend_from_slice(&[0, 0, 0, 3]);
        expected.extend_from_slice(b"Bob");
        expected.extend_from_slice(key_a.as_bytes());
        expected.extend_from_slice(&[0, 0, 0, 4]); // "Zoë" is 4 bytes in UTF-8
        expected.extend_from_slice("Zoë".as_bytes());
        expected.extend_from_slice(key_b.as_bytes());
        assert_eq!(
            editors_signing_bytes(share_id, 258, &keys).expect("bytes"),
            expected
        );
    }

    #[test]
    fn encode_decode_round_trip() {
        let owner = Identity::generate("Ann").expect("rng");
        let payload = sample(&owner, Uuid::from_u128(7), 3);
        let bytes = encode(&payload, &owner).expect("encode");
        let signed = decode(&bytes).expect("decode");
        assert_eq!(signed.payload, payload);
        assert_eq!(signed.signer, owner.key.verifying_key());
        assert_eq!(signed.raw, bytes);
    }

    #[test]
    fn decode_rejects_tampered_payload() {
        let owner = Identity::generate("Ann").expect("rng");
        let bytes = encode(&sample(&owner, Uuid::from_u128(7), 3), &owner).expect("encode");
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        let mut payload = STANDARD
            .decode(value["payload"].as_str().expect("payload"))
            .expect("b64");
        let at = payload.len() / 2;
        payload[at] ^= 1;
        value["payload"] = STANDARD.encode(&payload).into();
        let tampered = serde_json::to_vec(&value).expect("json");
        assert_eq!(decode(&tampered).unwrap_err(), ShareError::BadSignature);
    }

    #[test]
    fn decode_rejects_over_5_mib() {
        let big = vec![b' '; MAX_SHARE_BYTES + 1];
        assert_eq!(decode(&big).unwrap_err(), ShareError::TooLarge);
    }

    #[test]
    fn decode_size_check_boundary() {
        // Exactly 5 MiB passes the size check (and then fails as "not a share").
        let at_limit = vec![b' '; MAX_SHARE_BYTES];
        assert_eq!(decode(&at_limit).unwrap_err(), ShareError::NotAShare);
        let over = vec![b' '; MAX_SHARE_BYTES + 1];
        assert_eq!(decode(&over).unwrap_err(), ShareError::TooLarge);
    }

    #[test]
    fn decode_rejects_signature_checked_against_the_wrong_key() {
        let owner = Identity::generate("Ann").expect("rng");
        let other = Identity::generate("Eve").expect("rng");
        let bytes = encode(&sample(&owner, Uuid::from_u128(7), 3), &owner).expect("encode");
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        value["signer"] = encode_key(&other.key.verifying_key()).into();
        let swapped = serde_json::to_vec(&value).expect("json");
        assert_eq!(decode(&swapped).unwrap_err(), ShareError::BadSignature);
    }

    #[test]
    fn decode_rejects_wrong_format_or_version() {
        let owner = Identity::generate("Ann").expect("rng");
        let bytes = encode(&sample(&owner, Uuid::from_u128(7), 1), &owner).expect("encode");
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        value["format"] = "something-else".into();
        let wrong = serde_json::to_vec(&value).expect("json");
        assert_eq!(decode(&wrong).unwrap_err(), ShareError::NotAShare);
        value["format"] = FORMAT_ID.into();
        value["version"] = 2.into();
        let newer = serde_json::to_vec(&value).expect("json");
        assert_eq!(
            decode(&newer).unwrap_err(),
            ShareError::UnsupportedVersion(2)
        );
        assert_eq!(decode(b"not json").unwrap_err(), ShareError::NotAShare);
    }
}
