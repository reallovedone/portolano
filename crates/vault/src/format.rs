//! `vault.bin` byte layout, header parsing, AEAD sealing and atomic file writes.
//!
//! ```text
//! offset  size  field
//!      0     8  magic "PRTLVLT\0"
//!      8     2  format version, u16 little-endian (currently 1)
//!     10     4  Argon2id memory cost in KiB, u32 LE
//!     14     4  Argon2id iterations, u32 LE
//!     18     4  Argon2id parallelism, u32 LE
//!     22    16  salt
//!     38    24  nonce used to wrap the DEK
//!     62    48  wrapped DEK = XChaCha20-Poly1305(KEK, nonce, DEK) = 32 bytes + 16 byte tag
//!                (AAD = bytes 0..62)
//!    110    24  payload nonce (fresh random value on every save)
//!    134     n  payload = XChaCha20-Poly1305(DEK, payload nonce, JSON), n >= 16 (tag)
//!                (AAD = bytes 0..110, the whole clear header)
//! ```
//!
//! Everything before byte 110 is the "header". The payload nonce is not part of the AAD: it is
//! the nonce itself and is authenticated implicitly by the AEAD.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use secrecy::{ExposeSecret, SecretBox};
use zeroize::Zeroizing;

use crate::error::VaultError;
use crate::unlock::{KdfParams, Kek};

pub const MAGIC: [u8; 8] = *b"PRTLVLT\0";
pub const FORMAT_VERSION: u16 = 1;

pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const DEK_LEN: usize = 32;
pub const WRAPPED_DEK_LEN: usize = DEK_LEN + TAG_LEN;

const OFF_VERSION: usize = 8;
const OFF_KDF: usize = 10;
const OFF_SALT: usize = 22;
const OFF_DEK_NONCE: usize = OFF_SALT + SALT_LEN; // 38
const OFF_WRAPPED: usize = OFF_DEK_NONCE + NONCE_LEN; // 62
pub const HEADER_LEN: usize = OFF_WRAPPED + WRAPPED_DEK_LEN; // 110
pub const PAYLOAD_NONCE_OFFSET: usize = HEADER_LEN;
pub const PAYLOAD_OFFSET: usize = HEADER_LEN + NONCE_LEN; // 134
pub const MIN_FILE_LEN: usize = PAYLOAD_OFFSET + TAG_LEN;

/// Largest vault file that is read at all.
pub const MAX_FILE_LEN: u64 = 64 * 1024 * 1024;

/// Data encryption key: 32 random bytes, zeroized on drop.
pub type Dek = SecretBox<[u8; DEK_LEN]>;

/// The clear header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub dek_nonce: [u8; NONCE_LEN],
    pub wrapped_dek: [u8; WRAPPED_DEK_LEN],
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..OFF_VERSION].copy_from_slice(&MAGIC);
        out[OFF_VERSION..OFF_KDF].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        out[OFF_KDF..OFF_KDF + 4].copy_from_slice(&self.kdf.memory_kib.to_le_bytes());
        out[OFF_KDF + 4..OFF_KDF + 8].copy_from_slice(&self.kdf.iterations.to_le_bytes());
        out[OFF_KDF + 8..OFF_SALT].copy_from_slice(&self.kdf.parallelism.to_le_bytes());
        out[OFF_SALT..OFF_DEK_NONCE].copy_from_slice(&self.salt);
        out[OFF_DEK_NONCE..OFF_WRAPPED].copy_from_slice(&self.dek_nonce);
        out[OFF_WRAPPED..HEADER_LEN].copy_from_slice(&self.wrapped_dek);
        out
    }

    /// Defensive parse: every length is checked, the version is checked, and the Argon2
    /// parameters are range-checked before anything is derived from them.
    pub fn parse(file: &[u8]) -> Result<Self, VaultError> {
        if file.len() < MIN_FILE_LEN {
            return Err(VaultError::Corrupt("file too short"));
        }
        if file[..OFF_VERSION] != MAGIC {
            return Err(VaultError::Corrupt("bad magic"));
        }
        let version = u16::from_le_bytes([file[OFF_VERSION], file[OFF_VERSION + 1]]);
        if version != FORMAT_VERSION {
            return Err(VaultError::UnsupportedVersion(version));
        }
        let kdf = KdfParams {
            memory_kib: read_u32(file, OFF_KDF)?,
            iterations: read_u32(file, OFF_KDF + 4)?,
            parallelism: read_u32(file, OFF_KDF + 8)?,
        };
        kdf.validate()?;
        Ok(Self {
            kdf,
            salt: read_array(file, OFF_SALT)?,
            dek_nonce: read_array(file, OFF_DEK_NONCE)?,
            wrapped_dek: read_array(file, OFF_WRAPPED)?,
        })
    }

    /// Associated data that binds the DEK wrapping to every clear field before it.
    fn wrap_aad(encoded: &[u8; HEADER_LEN]) -> &[u8] {
        &encoded[..OFF_WRAPPED]
    }
}

fn read_array<const N: usize>(buf: &[u8], offset: usize) -> Result<[u8; N], VaultError> {
    buf.get(offset..offset + N)
        .and_then(|s| <[u8; N]>::try_from(s).ok())
        .ok_or(VaultError::Corrupt("truncated header"))
}

fn read_u32(buf: &[u8], offset: usize) -> Result<u32, VaultError> {
    Ok(u32::from_le_bytes(read_array(buf, offset)?))
}

/// Fills a fresh buffer with CSPRNG output.
pub fn random_array<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out)
        .map_err(|_| VaultError::Internal("the system random generator failed".into()))?;
    Ok(out)
}

/// A new random DEK.
pub fn generate_dek() -> Result<Dek, VaultError> {
    let mut failed = false;
    let dek = Dek::init_with_mut(|bytes| {
        if getrandom::fill(bytes).is_err() {
            failed = true;
        }
    });
    if failed {
        return Err(VaultError::Internal(
            "the system random generator failed".into(),
        ));
    }
    Ok(dek)
}

fn cipher(key: &[u8; 32]) -> Result<XChaCha20Poly1305, VaultError> {
    XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| VaultError::Internal("invalid cipher key length".into()))
}

/// Wraps `dek` with `kek` into a header with the given KDF parameters and salt. A fresh nonce
/// is drawn on every call.
pub fn wrap_dek(
    kek: &Kek,
    dek: &Dek,
    kdf: KdfParams,
    salt: [u8; SALT_LEN],
) -> Result<Header, VaultError> {
    let dek_nonce = random_array::<NONCE_LEN>()?;
    let mut header = Header {
        kdf,
        salt,
        dek_nonce,
        wrapped_dek: [0; WRAPPED_DEK_LEN],
    };
    let encoded = header.encode();
    let sealed = cipher(kek.expose_secret())?
        .encrypt(
            &XNonce::from(dek_nonce),
            Payload {
                msg: dek.expose_secret(),
                aad: Header::wrap_aad(&encoded),
            },
        )
        .map_err(|_| VaultError::Internal("DEK wrapping failed".into()))?;
    header.wrapped_dek = <[u8; WRAPPED_DEK_LEN]>::try_from(sealed.as_slice())
        .map_err(|_| VaultError::Internal("unexpected wrapped DEK length".into()))?;
    Ok(header)
}

/// Unwraps the DEK. Any failure (wrong KEK, modified header) is the same `AuthFailed`.
pub fn unwrap_dek(kek: &Kek, header: &Header) -> Result<Dek, VaultError> {
    let encoded = header.encode();
    let plain = cipher(kek.expose_secret())?
        .decrypt(
            &XNonce::from(header.dek_nonce),
            Payload {
                msg: &header.wrapped_dek,
                aad: Header::wrap_aad(&encoded),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| VaultError::AuthFailed)?;
    let mut failed = false;
    let dek = Dek::init_with_mut(|out| {
        if plain.len() == out.len() {
            out.copy_from_slice(&plain);
        } else {
            failed = true;
        }
    });
    if failed {
        return Err(VaultError::AuthFailed);
    }
    Ok(dek)
}

/// Serializes header, a fresh payload nonce and the encrypted `plaintext` into file bytes.
pub fn seal_file(header: &Header, dek: &Dek, plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let encoded = header.encode();
    let nonce = random_array::<NONCE_LEN>()?;
    let sealed = cipher(dek.expose_secret())?
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad: &encoded,
            },
        )
        .map_err(|_| VaultError::Internal("payload encryption failed".into()))?;
    let mut file = Vec::with_capacity(PAYLOAD_OFFSET + sealed.len());
    file.extend_from_slice(&encoded);
    file.extend_from_slice(&nonce);
    file.extend_from_slice(&sealed);
    Ok(file)
}

/// Decrypts the payload of `file`. The clear header bytes of
/// the file itself are the AAD, so a header that parses but was altered fails here.
pub fn open_payload(file: &[u8], dek: &Dek) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let aad = file
        .get(..HEADER_LEN)
        .ok_or(VaultError::Corrupt("file too short"))?;
    let nonce: [u8; NONCE_LEN] = read_array(file, PAYLOAD_NONCE_OFFSET)?;
    let body = file
        .get(PAYLOAD_OFFSET..)
        .ok_or(VaultError::Corrupt("file too short"))?;
    cipher(dek.expose_secret())?
        .decrypt(&XNonce::from(nonce), Payload { msg: body, aad })
        .map(Zeroizing::new)
        .map_err(|_| VaultError::AuthFailed)
}

/// Reads the whole vault file with a size cap.
pub fn read_file(path: &Path) -> Result<Vec<u8>, VaultError> {
    let file = File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => VaultError::NotCreated,
        _ => VaultError::Io(e),
    })?;
    let mut buf = Vec::new();
    file.take(MAX_FILE_LEN + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_FILE_LEN {
        return Err(VaultError::Corrupt("file too large"));
    }
    Ok(buf)
}

/// Atomic save: temporary file in the same directory, fsync, rename over the target, then
/// fsync of the directory (Unix). On failure the previous file is untouched.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| VaultError::Internal("vault path has no parent directory".into()))?;
    fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| VaultError::Internal("vault path has no file name".into()))?
        .to_string_lossy();
    let suffix: [u8; 8] = random_array()?;
    let mut tmp_name = format!(".{file_name}.");
    for b in suffix {
        tmp_name.push_str(&format!("{b:02x}"));
    }
    tmp_name.push_str(".tmp");
    let tmp_path = dir.join(tmp_name);

    let result = (|| -> Result<(), std::io::Error> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, path)?;
        #[cfg(unix)]
        File::open(dir)?.sync_all()?;
        Ok(())
    })();

    if let Err(err) = result {
        if let Err(cleanup) = fs::remove_file(&tmp_path) {
            if cleanup.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %cleanup, "could not remove temporary vault file");
            }
        }
        return Err(VaultError::Io(err));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_constants_are_consistent() {
        assert_eq!(HEADER_LEN, 110);
        assert_eq!(PAYLOAD_OFFSET, 134);
        assert_eq!(MIN_FILE_LEN, 150);
    }

    #[test]
    fn header_round_trips() {
        let header = Header {
            kdf: KdfParams::default(),
            salt: [1; SALT_LEN],
            dek_nonce: [2; NONCE_LEN],
            wrapped_dek: [3; WRAPPED_DEK_LEN],
        };
        let mut file = header.encode().to_vec();
        file.resize(MIN_FILE_LEN, 0);
        assert_eq!(Header::parse(&file).expect("parse"), header);
        assert_eq!(&file[..8], b"PRTLVLT\0");
    }

    #[test]
    fn parse_rejects_every_structural_problem() {
        let header = Header {
            kdf: KdfParams {
                memory_kib: 8,
                iterations: 1,
                parallelism: 1,
            },
            salt: [0; SALT_LEN],
            dek_nonce: [0; NONCE_LEN],
            wrapped_dek: [0; WRAPPED_DEK_LEN],
        };
        let mut good = header.encode().to_vec();
        good.resize(MIN_FILE_LEN, 0);
        assert!(Header::parse(&good).is_ok());

        assert!(matches!(
            Header::parse(&good[..MIN_FILE_LEN - 1]),
            Err(VaultError::Corrupt(_))
        ));
        assert!(matches!(Header::parse(&[]), Err(VaultError::Corrupt(_))));

        let mut bad = good.clone();
        bad[0] ^= 1;
        assert!(matches!(Header::parse(&bad), Err(VaultError::Corrupt(_))));

        let mut bad = good.clone();
        bad[OFF_VERSION] = 2;
        assert!(matches!(
            Header::parse(&bad),
            Err(VaultError::UnsupportedVersion(2))
        ));

        let mut bad = good.clone();
        bad[OFF_KDF..OFF_KDF + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(Header::parse(&bad), Err(VaultError::Corrupt(_))));
    }

    #[test]
    fn wrap_unwrap_and_wrong_kek() {
        let kdf = KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        };
        let kek = Kek::new(Box::new([5u8; 32]));
        let other = Kek::new(Box::new([6u8; 32]));
        let dek = generate_dek().expect("dek");
        let header = wrap_dek(&kek, &dek, kdf, [9; SALT_LEN]).expect("wrap");
        let back = unwrap_dek(&kek, &header).expect("unwrap");
        assert_eq!(back.expose_secret(), dek.expose_secret());
        assert!(matches!(
            unwrap_dek(&other, &header),
            Err(VaultError::AuthFailed)
        ));
        // Header fields are bound as AAD: changing the salt makes unwrapping fail.
        let mut altered = header.clone();
        altered.salt[0] ^= 1;
        assert!(matches!(
            unwrap_dek(&kek, &altered),
            Err(VaultError::AuthFailed)
        ));
    }
}
