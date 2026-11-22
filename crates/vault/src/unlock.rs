//! Key unlock abstraction (§4.2): the DEK is separate from the KEK so that keyring and
//! biometric unlock can be added later without changing the file format.
//!
//! A future keyring method would store the DEK itself (or a key that wraps it) outside the
//! file; the file layout does not change because the wrapped DEK in the header stays valid for
//! the master password path.

use argon2::{Algorithm, Argon2, Block, Params, Version};
use secrecy::{ExposeSecret, SecretBox, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::VaultError;

/// Key-encryption key (32 bytes), zeroized on drop.
pub type Kek = SecretBox<[u8; 32]>;

/// Upper bounds applied to parameters read from a file, so a tampered header cannot make the
/// process allocate gigabytes or spin for minutes.
pub const MAX_MEMORY_KIB: u32 = 512 * 1024;
pub const MAX_ITERATIONS: u32 = 16;
pub const MAX_PARALLELISM: u32 = 8;

/// Argon2id parameters stored in the clear in the file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfParams {
    /// Memory cost in KiB.
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    /// m = 64 MiB, t = 3, p = 1.
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

impl KdfParams {
    /// Range check shared by file parsing and vault creation: at least what Argon2 itself
    /// requires, at most the caps above.
    pub fn validate(&self) -> Result<(), VaultError> {
        let p_ok = (1..=MAX_PARALLELISM).contains(&self.parallelism);
        let t_ok = (1..=MAX_ITERATIONS).contains(&self.iterations);
        // Argon2 needs at least 8 KiB per lane.
        let m_ok =
            p_ok && self.memory_kib >= 8 * self.parallelism && self.memory_kib <= MAX_MEMORY_KIB;
        if p_ok && t_ok && m_ok {
            Ok(())
        } else {
            Err(VaultError::Corrupt(
                "key derivation parameters out of range",
            ))
        }
    }
}

/// A way of obtaining the KEK that wraps the DEK. The MVP has one implementation,
/// `MasterPassword`.
pub trait UnlockMethod {
    /// Stable identifier of the method (for diagnostics and future file metadata).
    fn id(&self) -> &'static str;

    /// Derives or retrieves the KEK for the given header parameters and salt.
    fn derive_kek(&self, params: &KdfParams, salt: &[u8; 16]) -> Result<Kek, VaultError>;
}

/// Master-password unlock: `KEK = Argon2id(password, salt)` with a 32-byte output.
pub struct MasterPassword(pub SecretString);

impl UnlockMethod for MasterPassword {
    fn id(&self) -> &'static str {
        "master-password"
    }

    fn derive_kek(&self, params: &KdfParams, salt: &[u8; 16]) -> Result<Kek, VaultError> {
        params.validate()?;
        let argon_params = Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            Some(32),
        )
        .map_err(|_| VaultError::Corrupt("key derivation parameters rejected"))?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

        // Own the working memory so it can be wiped afterwards (the library does not do it).
        let blocks_len = argon.params().block_count();
        let mut blocks: Vec<Block> = Vec::new();
        blocks
            .try_reserve_exact(blocks_len)
            .map_err(|_| VaultError::Internal("cannot allocate key derivation memory".into()))?;
        blocks.resize(blocks_len, Block::new());

        let mut kek = SecretBox::<[u8; 32]>::init_with_mut(|_| {});
        let result = {
            use secrecy::ExposeSecretMut;
            argon.hash_password_into_with_memory(
                self.0.expose_secret().as_bytes(),
                salt,
                kek.expose_secret_mut(),
                &mut blocks,
            )
        };
        blocks.zeroize();
        // The error text of Argon2 carries no secret, but nothing useful for the caller either.
        result.map_err(|_| VaultError::Internal("key derivation failed".into()))?;
        Ok(kek)
    }
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

    #[test]
    fn default_params_match_the_spec() {
        let p = KdfParams::default();
        assert_eq!((p.memory_kib, p.iterations, p.parallelism), (65536, 3, 1));
        assert!(p.validate().is_ok());
    }

    #[test]
    fn absurd_params_are_rejected_before_any_work() {
        for p in [
            KdfParams {
                memory_kib: u32::MAX,
                ..tiny()
            },
            KdfParams {
                memory_kib: MAX_MEMORY_KIB + 1,
                ..tiny()
            },
            KdfParams {
                iterations: 0,
                ..tiny()
            },
            KdfParams {
                iterations: u32::MAX,
                ..tiny()
            },
            KdfParams {
                parallelism: 0,
                ..tiny()
            },
            KdfParams {
                parallelism: u32::MAX,
                ..tiny()
            },
            KdfParams {
                memory_kib: 4,
                ..tiny()
            },
            KdfParams {
                memory_kib: 8,
                parallelism: 2,
                iterations: 1,
            },
        ] {
            assert!(p.validate().is_err(), "{p:?} must be rejected");
            let m = MasterPassword(SecretString::from("fake-password"));
            assert!(matches!(
                m.derive_kek(&p, &[0; 16]),
                Err(VaultError::Corrupt(_))
            ));
        }
    }

    #[test]
    fn derivation_is_deterministic_and_input_sensitive() {
        let a = MasterPassword(SecretString::from("fake-password-a"));
        let b = MasterPassword(SecretString::from("fake-password-b"));
        let salt = [7u8; 16];
        let k1 = a.derive_kek(&tiny(), &salt).expect("kek");
        let k2 = a.derive_kek(&tiny(), &salt).expect("kek");
        let k3 = b.derive_kek(&tiny(), &salt).expect("kek");
        let k4 = a.derive_kek(&tiny(), &[8u8; 16]).expect("kek");
        assert_eq!(k1.expose_secret(), k2.expose_secret());
        assert_ne!(k1.expose_secret(), k3.expose_secret());
        assert_ne!(k1.expose_secret(), k4.expose_secret());
        assert_eq!(a.id(), "master-password");
    }
}
