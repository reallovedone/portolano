//! Encrypted credential vault (§4.2). See `format.rs` for the byte layout of `vault.bin`.
//!
//! Unlock is master-password only; no key is ever stored in the OS keyring in the MVP.
//! JSON convention is the one documented in `portolano-core-model`.

mod error;
mod format;
pub mod identity_backup;
mod payload;
mod types;
mod unlock;
mod vault;

pub use error::VaultError;
pub use types::{
    Credential, CredentialInput, CredentialKind, CredentialSummary, CredentialUpsert,
    IdentityRecord, RevealedSecret, Secret, SecretInput, VaultStatus,
};
pub use unlock::{KdfParams, Kek, MasterPassword, UnlockMethod};
pub use vault::{Clock, SystemClock, Vault, DEFAULT_AUTO_LOCK};
