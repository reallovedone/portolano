//! Rules for trusting a share file: who may sign it and what may change between versions.

use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use uuid::Uuid;

use crate::format::{editors_signing_bytes, SharePayload, SignedShare};
use crate::identity::decode_key;

/// What a subscriber remembers about a share after the first acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub share_id: Uuid,
    pub owner_key: VerifyingKey,
    pub last_revision: u64,
    pub editors_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptMode {
    /// A freshly fetched file: its revision must be higher than the pinned one.
    Update,
    /// A file from the offline cache: the same revision is fine.
    Cached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    WrongShare,
    OwnerChanged,
    /// The editor list is not signed by the owner key (also: its keys cannot be read).
    EditorsNotSignedByOwner,
    EditorsRollback,
    UnknownSigner,
    OldRevision,
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WrongShare => "This file belongs to a different shared folder.",
            Self::OwnerChanged => "The owner of this shared folder changed. Update refused.",
            Self::EditorsNotSignedByOwner => {
                "The list of editors is not signed by the owner of the shared folder."
            }
            Self::EditorsRollback => {
                "The file has an older list of editors than the one you trust."
            }
            Self::UnknownSigner => {
                "The file was signed by someone who is not the owner or a listed editor."
            }
            Self::OldRevision => "The file is older than the version you already have.",
        })
    }
}

impl std::error::Error for Rejection {}

fn editors_signed_by(payload: &SharePayload, owner: &VerifyingKey) -> bool {
    let editors = &payload.editors;
    let Some(bytes) = editors_signing_bytes(payload.share_id, editors.version, &editors.keys)
    else {
        return false;
    };
    let Some(signature) = STANDARD
        .decode(&editors.owner_signature)
        .ok()
        .and_then(|b| Signature::from_slice(&b).ok())
    else {
        return false;
    };
    owner.verify_strict(&bytes, &signature).is_ok()
}

fn signer_allowed(share: &SignedShare, owner: &VerifyingKey) -> bool {
    share.signer == *owner
        || share
            .payload
            .editors
            .keys
            .iter()
            .filter_map(|k| decode_key(&k.key))
            .any(|k| k == share.signer)
}

/// Trusts the owner named in `share` (first use): the editor list must be signed by that owner
/// and the file by the owner or a listed editor.
pub fn first_pin(share: &SignedShare) -> Result<Pinned, Rejection> {
    let payload = &share.payload;
    let owner = decode_key(&payload.owner.key).ok_or(Rejection::EditorsNotSignedByOwner)?;
    if !editors_signed_by(payload, &owner) {
        return Err(Rejection::EditorsNotSignedByOwner);
    }
    if !signer_allowed(share, &owner) {
        return Err(Rejection::UnknownSigner);
    }
    Ok(Pinned {
        share_id: payload.share_id,
        owner_key: owner,
        last_revision: payload.revision,
        editors_version: payload.editors.version,
    })
}

/// Checks `share` against what was pinned.
pub fn accept(pinned: &Pinned, share: &SignedShare, mode: AcceptMode) -> Result<(), Rejection> {
    let payload = &share.payload;
    if payload.share_id != pinned.share_id {
        return Err(Rejection::WrongShare);
    }
    if decode_key(&payload.owner.key).as_ref() != Some(&pinned.owner_key) {
        return Err(Rejection::OwnerChanged);
    }
    if !editors_signed_by(payload, &pinned.owner_key) {
        return Err(Rejection::EditorsNotSignedByOwner);
    }
    if payload.editors.version < pinned.editors_version {
        return Err(Rejection::EditorsRollback);
    }
    if !signer_allowed(share, &pinned.owner_key) {
        return Err(Rejection::UnknownSigner);
    }
    let fresh = match mode {
        AcceptMode::Update => payload.revision > pinned.last_revision,
        AcceptMode::Cached => payload.revision >= pinned.last_revision,
    };
    if fresh {
        Ok(())
    } else {
        Err(Rejection::OldRevision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{decode, encode, sign_editors, tests::sample, EditorList, KeyEntry};
    use crate::identity::Identity;

    fn share_id() -> Uuid {
        Uuid::from_u128(7)
    }

    fn signed(payload: &SharePayload, signer: &Identity) -> SignedShare {
        decode(&encode(payload, signer).expect("encode")).expect("decode")
    }

    fn with_editors(payload: &mut SharePayload, owner: &Identity, version: u64, who: &[&Identity]) {
        let keys: Vec<KeyEntry> = who
            .iter()
            .map(|i| KeyEntry::new(&i.name, &i.key.verifying_key()))
            .collect();
        payload.editors = EditorList {
            version,
            owner_signature: sign_editors(share_id(), version, &keys, owner),
            keys,
        };
    }

    fn pinned_for(owner: &Identity, revision: u64) -> Pinned {
        first_pin(&signed(&sample(owner, share_id(), revision), owner)).expect("pin")
    }

    #[test]
    fn first_pin_rejects_editor_list_signed_by_editor() {
        let owner = Identity::generate("Ann").expect("rng");
        let editor = Identity::generate("Bob").expect("rng");
        let mut payload = sample(&owner, share_id(), 1);
        // The editor signs a list that names itself.
        let keys = vec![KeyEntry::new("Bob", &editor.key.verifying_key())];
        payload.editors = EditorList {
            version: 2,
            owner_signature: sign_editors(share_id(), 2, &keys, &editor),
            keys,
        };
        assert_eq!(
            first_pin(&signed(&payload, &editor)),
            Err(Rejection::EditorsNotSignedByOwner)
        );
    }

    #[test]
    fn accept_rejects_other_share_id() {
        let owner = Identity::generate("Ann").expect("rng");
        let pinned = pinned_for(&owner, 1);
        let other = signed(&sample(&owner, Uuid::from_u128(8), 2), &owner);
        assert_eq!(
            accept(&pinned, &other, AcceptMode::Update),
            Err(Rejection::WrongShare)
        );
    }

    #[test]
    fn accept_rejects_owner_change() {
        let owner = Identity::generate("Ann").expect("rng");
        let thief = Identity::generate("Mallory").expect("rng");
        let pinned = pinned_for(&owner, 1);
        let forged = signed(&sample(&thief, share_id(), 5), &thief);
        assert_eq!(
            accept(&pinned, &forged, AcceptMode::Update),
            Err(Rejection::OwnerChanged)
        );
    }

    #[test]
    fn accept_rejects_lower_editors_version() {
        let owner = Identity::generate("Ann").expect("rng");
        let mut v3 = sample(&owner, share_id(), 1);
        with_editors(&mut v3, &owner, 3, &[]);
        let pinned = first_pin(&signed(&v3, &owner)).expect("pin");
        let mut v2 = sample(&owner, share_id(), 2);
        with_editors(&mut v2, &owner, 2, &[]);
        assert_eq!(
            accept(&pinned, &signed(&v2, &owner), AcceptMode::Update),
            Err(Rejection::EditorsRollback)
        );
    }

    #[test]
    fn accept_rejects_unknown_signer() {
        let owner = Identity::generate("Ann").expect("rng");
        let stranger = Identity::generate("Eve").expect("rng");
        let pinned = pinned_for(&owner, 1);
        let file = signed(&sample(&owner, share_id(), 2), &stranger);
        assert_eq!(
            accept(&pinned, &file, AcceptMode::Update),
            Err(Rejection::UnknownSigner)
        );
    }

    #[test]
    fn accept_update_requires_higher_revision() {
        let owner = Identity::generate("Ann").expect("rng");
        let pinned = pinned_for(&owner, 4);
        for revision in [3, 4] {
            let file = signed(&sample(&owner, share_id(), revision), &owner);
            assert_eq!(
                accept(&pinned, &file, AcceptMode::Update),
                Err(Rejection::OldRevision)
            );
        }
        let newer = signed(&sample(&owner, share_id(), 5), &owner);
        assert_eq!(accept(&pinned, &newer, AcceptMode::Update), Ok(()));
    }

    #[test]
    fn accept_cached_allows_same_revision() {
        let owner = Identity::generate("Ann").expect("rng");
        let pinned = pinned_for(&owner, 4);
        let same = signed(&sample(&owner, share_id(), 4), &owner);
        assert_eq!(accept(&pinned, &same, AcceptMode::Cached), Ok(()));
        let older = signed(&sample(&owner, share_id(), 3), &owner);
        assert_eq!(
            accept(&pinned, &older, AcceptMode::Cached),
            Err(Rejection::OldRevision)
        );
    }

    #[test]
    fn accept_allows_listed_editor() {
        let owner = Identity::generate("Ann").expect("rng");
        let editor = Identity::generate("Bob").expect("rng");
        let mut payload = sample(&owner, share_id(), 2);
        with_editors(&mut payload, &owner, 2, &[&editor]);
        let pinned = pinned_for(&owner, 1);
        let file = signed(&payload, &editor);
        assert_eq!(accept(&pinned, &file, AcceptMode::Update), Ok(()));
        assert!(first_pin(&file).is_ok());
    }
}
