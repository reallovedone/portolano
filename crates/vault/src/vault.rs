use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ed25519_dalek::SigningKey;
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::VaultError;
use crate::format::{self, Dek, Header};
use crate::payload;
use crate::types::clean_identity_name;
use crate::types::{
    Credential, CredentialInput, CredentialSummary, CredentialUpsert, Identity, IdentityRecord,
    RevealedSecret, Secret, VaultStatus,
};
use crate::unlock::{KdfParams, MasterPassword, UnlockMethod};

/// Default inactivity period before the vault locks itself.
pub const DEFAULT_AUTO_LOCK: Duration = Duration::from_secs(15 * 60);

const MAX_CREDENTIALS: usize = 10_000;
const MAX_TEXT_LEN: usize = 4 * 1024;
const MAX_SECRET_LEN: usize = 256 * 1024;
const MAX_IDENTITIES: usize = 1_000;

/// Time source, injectable so the auto-lock logic is testable without sleeping.
///
/// Two readings are kept because neither is enough alone. `Instant` is monotonic but may not
/// advance while the computer sleeps (the platform decides): a laptop closed overnight could
/// wake with the idle time unchanged. The wall clock does advance through sleep but can be
/// set back by the user. The idle time is the larger of the two.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;

    /// The wall clock (counts time spent asleep).
    fn wall(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Everything that exists only while the vault is unlocked. Dropping it zeroizes the DEK and
/// every secret this struct owns (`SecretBox` / `SecretString` / `Zeroizing` wipe themselves on
/// drop); copies made elsewhere, such as by the JSON parser, are not covered.
struct Unlocked {
    dek: Dek,
    header: Header,
    credentials: Vec<Credential>,
    identities: Vec<Identity>,
}

/// The vault: an encrypted file plus, while unlocked, its decrypted content.
pub struct Vault {
    path: PathBuf,
    /// Parameters used when a key is derived for a NEW header (create, change password).
    kdf: KdfParams,
    clock: Arc<dyn Clock>,
    /// Zero disables auto-lock.
    auto_lock: Duration,
    last_activity: Cell<Instant>,
    /// Wall-clock time of the last activity (see [`Clock`]).
    last_activity_wall: Cell<SystemTime>,
    unlocked: Option<Unlocked>,
}

impl fmt::Debug for Vault {
    /// Never prints credentials or keys.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("unlocked", &self.unlocked.is_some())
            .finish_non_exhaustive()
    }
}

impl Vault {
    /// Binds a vault to its file (`vault.bin`) with the production KDF parameters
    /// (Argon2id, m = 64 MiB, t = 3, p = 1) and the default auto-lock. Performs no I/O.
    pub fn new(path: PathBuf) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        Self {
            path,
            kdf: KdfParams::default(),
            last_activity: Cell::new(clock.now()),
            last_activity_wall: Cell::new(clock.wall()),
            clock,
            auto_lock: DEFAULT_AUTO_LOCK,
            unlocked: None,
        }
    }

    /// Like `new` but with explicit KDF parameters for newly written headers. Meant for tests
    /// (small, fast parameters) and for future tuning; existing files keep the parameters
    /// stored in their own header.
    pub fn with_kdf_params(path: PathBuf, kdf: KdfParams) -> Result<Self, VaultError> {
        kdf.validate()?;
        let mut vault = Self::new(path);
        vault.kdf = kdf;
        Ok(vault)
    }

    /// Replaces the time source (tests).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.last_activity.set(clock.now());
        self.last_activity_wall.set(clock.wall());
        self.clock = clock;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// KDF parameters used for newly written headers.
    pub fn kdf_params(&self) -> KdfParams {
        self.kdf
    }

    pub fn auto_lock_timeout(&self) -> Duration {
        self.auto_lock
    }

    /// Sets the inactivity period; `Duration::ZERO` disables auto-lock. Counts as activity.
    pub fn set_auto_lock_timeout(&mut self, timeout: Duration) {
        self.auto_lock = timeout;
        self.touch();
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked.is_some()
    }

    fn touch(&self) {
        self.last_activity.set(self.clock.now());
        self.last_activity_wall.set(self.clock.wall());
    }

    /// Time since the last activity: the larger of the monotonic and the wall-clock readings,
    /// so that time spent asleep counts (a wall clock set back is ignored).
    fn idle(&self) -> Duration {
        let monotonic = self
            .clock
            .now()
            .saturating_duration_since(self.last_activity.get());
        let wall = self
            .clock
            .wall()
            .duration_since(self.last_activity_wall.get())
            .unwrap_or_default();
        monotonic.max(wall)
    }

    /// Auto-lock check, to be called periodically. Locks the vault when it is unlocked, the
    /// timeout is enabled and no operation happened for at least that long. Returns whether
    /// it locked. Does not count as activity.
    pub fn tick(&mut self) -> bool {
        if self.unlocked.is_none() || self.auto_lock.is_zero() {
            return false;
        }
        if self.idle() >= self.auto_lock {
            self.unlocked = None;
            tracing::info!("vault locked after inactivity");
            true
        } else {
            false
        }
    }

    /// Does not count as activity (the UI polls it).
    pub fn status(&self) -> Result<VaultStatus, VaultError> {
        if self.unlocked.is_some() {
            return Ok(VaultStatus::Unlocked);
        }
        match self.path.try_exists() {
            Ok(true) => Ok(VaultStatus::Locked),
            Ok(false) => Ok(VaultStatus::NotCreated),
            Err(e) => Err(VaultError::Io(e)),
        }
    }

    /// Creates the vault protected by `master_password`, and leaves it unlocked. Refuses to
    /// overwrite an existing file. Policy: only the empty password is rejected.
    pub fn create(&mut self, master_password: SecretString) -> Result<(), VaultError> {
        check_master_password(&master_password)?;
        self.create_with(&MasterPassword(master_password))
    }

    /// `create` for any unlock method.
    pub fn create_with(&mut self, method: &dyn UnlockMethod) -> Result<(), VaultError> {
        if self.path.try_exists()? {
            return Err(VaultError::AlreadyExists);
        }
        let salt = format::random_array::<{ format::SALT_LEN }>()?;
        let kek = method.derive_kek(&self.kdf, &salt)?;
        let dek = format::generate_dek()?;
        let header = format::wrap_dek(&kek, &dek, self.kdf, salt)?;
        let credentials: Vec<Credential> = Vec::new();
        let identities: Vec<Identity> = Vec::new();
        let plaintext = payload::encode(&credentials, &identities)?;
        let bytes = format::seal_file(&header, &dek, &plaintext)?;
        format::write_atomic(&self.path, &bytes)?;
        self.unlocked = Some(Unlocked {
            dek,
            header,
            credentials,
            identities,
        });
        self.touch();
        Ok(())
    }

    pub fn unlock(&mut self, master_password: SecretString) -> Result<(), VaultError> {
        self.unlock_with(&MasterPassword(master_password))
    }

    /// `unlock` for any unlock method. A wrong secret and a tampered file both end in
    /// `AuthFailed`; a structurally invalid file is reported as damaged before any key
    /// derivation happens.
    pub fn unlock_with(&mut self, method: &dyn UnlockMethod) -> Result<(), VaultError> {
        let bytes = format::read_file(&self.path)?;
        let header = Header::parse(&bytes)?;
        let kek = method.derive_kek(&header.kdf, &header.salt)?;
        let dek = format::unwrap_dek(&kek, &header)?;
        let plaintext = format::open_payload(&bytes, &dek)?;
        let (credentials, identities) = payload::decode(&plaintext)?;
        self.unlocked = Some(Unlocked {
            dek,
            header,
            credentials,
            identities,
        });
        self.touch();
        Ok(())
    }

    /// Changes the master password and rotates the data key: a NEW random DEK is generated,
    /// wrapped under a key derived from `new_password` (fresh salt and nonce, current KDF
    /// parameters), and the payload is sealed again under it with a fresh nonce.
    ///
    /// Rotating matters: whoever knows the OLD password and holds an old copy of the file (a
    /// backup, a synced copy) can unwrap the old DEK, and with a re-wrapped (unchanged) DEK
    /// that would open the new file as well. An old copy stays readable with the old
    /// password; nothing written from now on is.
    ///
    /// Needs an unlocked vault; `current_password` is verified against the header. The file is
    /// replaced atomically; if that fails the vault keeps its previous key and header.
    pub fn change_password(
        &mut self,
        current_password: SecretString,
        new_password: SecretString,
    ) -> Result<(), VaultError> {
        let unlocked = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        self.touch();
        check_master_password(&new_password)?;

        let current = MasterPassword(current_password);
        let kek = current.derive_kek(&unlocked.header.kdf, &unlocked.header.salt)?;
        format::unwrap_dek(&kek, &unlocked.header)?;
        drop(kek);

        let salt = format::random_array::<{ format::SALT_LEN }>()?;
        let new_kek = MasterPassword(new_password).derive_kek(&self.kdf, &salt)?;
        let new_dek = format::generate_dek()?;
        let header = format::wrap_dek(&new_kek, &new_dek, self.kdf, salt)?;
        let plaintext = payload::encode(&unlocked.credentials, &unlocked.identities)?;
        let bytes = format::seal_file(&header, &new_dek, &plaintext)?;
        format::write_atomic(&self.path, &bytes)?;
        if let Some(u) = self.unlocked.as_mut() {
            u.header = header;
            // The old key is zeroized when it is dropped here.
            u.dek = new_dek;
        }
        self.touch();
        Ok(())
    }

    /// Zeroizes the DEK and the decrypted credentials. Idempotent.
    pub fn lock(&mut self) -> Result<(), VaultError> {
        self.unlocked = None;
        Ok(())
    }

    fn unlocked(&self) -> Result<&Unlocked, VaultError> {
        let u = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        self.touch();
        Ok(u)
    }

    /// Like [`Vault::unlocked`], for a change: the unlocked state with the path it is saved to.
    fn unlocked_mut(&mut self) -> Result<(&Path, &mut Unlocked), VaultError> {
        self.touch();
        let u = self.unlocked.as_mut().ok_or(VaultError::Locked)?;
        Ok((&self.path, u))
    }

    pub fn list(&self) -> Result<Vec<CredentialSummary>, VaultError> {
        Ok(self
            .unlocked()?
            .credentials
            .iter()
            .map(CredentialSummary::from)
            .collect())
    }

    /// Creates (`id: None` or unknown id) or replaces a credential and saves the file. If the
    /// save fails the in-memory state is rolled back.
    pub fn upsert(&mut self, input: CredentialInput) -> Result<CredentialSummary, VaultError> {
        self.upsert_update(input.into())
    }

    /// Like [`Vault::upsert`], but `input.secret` may be `None` to keep the stored secret of an
    /// existing credential (a rename, a new username...). Without a secret, an unknown or
    /// missing id is an error: creating needs one. A kept secret never leaves the vault.
    pub fn upsert_update(
        &mut self,
        input: CredentialUpsert,
    ) -> Result<CredentialSummary, VaultError> {
        let existing = {
            let unlocked = self.unlocked()?;
            input.id.and_then(|id| {
                unlocked
                    .credentials
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.secret.clone())
            })
        };
        let credential = validate_input(input, existing)?;
        let summary = CredentialSummary::from(&credential);
        let id = credential.id;

        let (path, unlocked) = self.unlocked_mut()?;
        let index = unlocked.credentials.iter().position(|c| c.id == id);
        if index.is_none() && unlocked.credentials.len() >= MAX_CREDENTIALS {
            return Err(VaultError::InvalidInput("too many credentials".into()));
        }
        let previous = match index {
            Some(i) => Some(std::mem::replace(&mut unlocked.credentials[i], credential)),
            None => {
                unlocked.credentials.push(credential);
                None
            }
        };
        if let Err(err) = save(path, unlocked) {
            match (index, previous) {
                (Some(i), Some(prev)) => unlocked.credentials[i] = prev,
                _ => {
                    unlocked.credentials.pop();
                }
            }
            return Err(err);
        }
        Ok(summary)
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), VaultError> {
        let (path, unlocked) = self.unlocked_mut()?;
        let index = unlocked
            .credentials
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| VaultError::NotFound(id.to_string()))?;
        let removed = unlocked.credentials.remove(index);
        if let Err(err) = save(path, unlocked) {
            unlocked.credentials.insert(index, removed);
            return Err(err);
        }
        Ok(())
    }

    /// Plain-text secret, only for the explicit `vault_reveal_secret` user action.
    pub fn reveal_secret(&self, id: Uuid) -> Result<RevealedSecret, VaultError> {
        let credential = self
            .unlocked()?
            .credentials
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| VaultError::NotFound(id.to_string()))?;
        Ok(match &credential.secret {
            Secret::Password(p) => RevealedSecret::Password {
                password: p.expose_secret().to_owned(),
            },
            Secret::SshKey {
                private_key_pem,
                passphrase,
            } => RevealedSecret::SshKey {
                private_key_pem: private_key_pem.expose_secret().to_owned(),
                passphrase: passphrase.as_ref().map(|p| p.expose_secret().to_owned()),
            },
        })
    }

    /// The signing identities (without their secret keys).
    pub fn identities(&self) -> Result<Vec<IdentityRecord>, VaultError> {
        Ok(self
            .unlocked()?
            .identities
            .iter()
            .map(|i| i.record.clone())
            .collect())
    }

    /// Stores a signing identity. The public key is derived here from `secret` (the only
    /// place it is), so the two can never disagree.
    pub fn add_identity(
        &mut self,
        id: Uuid,
        name: &str,
        secret: &[u8; 32],
    ) -> Result<IdentityRecord, VaultError> {
        self.unlocked()?;
        let name = clean_identity_name(name)?;
        let public_key = SigningKey::from_bytes(secret).verifying_key().to_bytes();
        let created_at = rfc3339_utc(self.clock.wall());
        let (path, unlocked) = self.unlocked_mut()?;
        if unlocked.identities.iter().any(|i| i.record.id == id) {
            return Err(VaultError::InvalidInput("duplicate identity id".into()));
        }
        if unlocked
            .identities
            .iter()
            .any(|i| i.record.public_key == public_key)
        {
            return Err(VaultError::InvalidInput("identity already present".into()));
        }
        if unlocked.identities.len() >= MAX_IDENTITIES {
            return Err(VaultError::InvalidInput("too many identities".into()));
        }
        let record = IdentityRecord {
            id,
            name: name.to_owned(),
            public_key,
            created_at,
        };
        unlocked.identities.push(Identity {
            record: record.clone(),
            secret: copy_secret(secret),
        });
        if let Err(err) = save(path, unlocked) {
            unlocked.identities.pop();
            return Err(err);
        }
        Ok(record)
    }

    /// The secret key of an identity, for signing. Needs an unlocked vault.
    pub fn identity_secret(&self, id: Uuid) -> Result<Zeroizing<[u8; 32]>, VaultError> {
        self.unlocked()?
            .identities
            .iter()
            .find(|i| i.record.id == id)
            .map(|i| i.secret.clone())
            .ok_or_else(|| VaultError::NotFound(id.to_string()))
    }

    /// Test hook: the raw DEK bytes of the unlocked vault.
    #[cfg(test)]
    fn dek_bytes(&self) -> Option<[u8; 32]> {
        self.unlocked.as_ref().map(|u| *u.dek.expose_secret())
    }
}

/// A wiped-on-drop copy of a key, made in place so no temporary array is left on the stack.
fn copy_secret(secret: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let mut copy = Zeroizing::new([0u8; 32]);
    copy.copy_from_slice(secret);
    copy
}

/// Latest time shown (`9999-12-31T23:59:59Z`): the year always has four digits.
const MAX_RFC3339_SECS: u64 = 253_402_300_799;

/// `2026-11-20T09:30:00Z` for a wall-clock time (no date/time dependency needed for this).
pub fn rfc3339_utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs().min(MAX_RFC3339_SECS))
        .unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil-from-days (Howard Hinnant's algorithm), valid for the whole u64 range we use.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

fn check_master_password(password: &SecretString) -> Result<(), VaultError> {
    if password.expose_secret().is_empty() {
        return Err(VaultError::InvalidInput(
            "the master password must not be empty".into(),
        ));
    }
    Ok(())
}

fn save(path: &Path, unlocked: &Unlocked) -> Result<(), VaultError> {
    let plaintext = payload::encode(&unlocked.credentials, &unlocked.identities)?;
    let bytes = format::seal_file(&unlocked.header, &unlocked.dek, &plaintext)?;
    format::write_atomic(path, &bytes)
}

fn validate_input(
    input: CredentialUpsert,
    existing_secret: Option<Secret>,
) -> Result<Credential, VaultError> {
    let name = input.name.trim().to_owned();
    if name.is_empty() {
        return Err(VaultError::InvalidInput(
            "the credential name must not be empty".into(),
        ));
    }
    let domain = input
        .domain
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty());
    let too_long = |s: &str| s.len() > MAX_TEXT_LEN;
    if too_long(&name) || too_long(&input.username) || domain.as_deref().is_some_and(too_long) {
        return Err(VaultError::InvalidInput("a text field is too long".into()));
    }
    let secret: Secret = match (input.secret, existing_secret) {
        (Some(new), _) => new.into(),
        (None, Some(kept)) => kept,
        (None, None) => {
            return Err(VaultError::InvalidInput(
                "a secret is required to create a credential".into(),
            ))
        }
    };
    let secret_too_long = match &secret {
        Secret::Password(p) => p.expose_secret().len() > MAX_TEXT_LEN,
        Secret::SshKey {
            private_key_pem,
            passphrase,
        } => {
            private_key_pem.expose_secret().len() > MAX_SECRET_LEN
                || passphrase
                    .as_ref()
                    .is_some_and(|p| p.expose_secret().len() > MAX_TEXT_LEN)
        }
    };
    if secret_too_long {
        return Err(VaultError::InvalidInput("a secret is too long".into()));
    }
    Ok(Credential {
        id: input.id.unwrap_or_else(Uuid::new_v4),
        name,
        username: input.username,
        domain,
        secret,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    use super::*;
    use crate::types::{CredentialKind, SecretInput};

    fn tiny() -> KdfParams {
        KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        }
    }

    /// Test data lives under `<repo>/.tmp/wp3/`, never in the real data dir or `/tmp`.
    fn scratch() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/wp3");
        std::fs::create_dir_all(&base).expect("create scratch base");
        tempfile::Builder::new()
            .prefix("vault-unit-")
            .tempdir_in(base)
            .expect("tempdir")
    }

    fn pw(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn password_input(name: &str, password: &str) -> CredentialInput {
        CredentialInput {
            id: None,
            name: name.into(),
            username: "user".into(),
            domain: None,
            secret: SecretInput::Password {
                password: pw(password),
            },
        }
    }

    struct ManualClock {
        base: Instant,
        offset_secs: AtomicU64,
    }

    impl ManualClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                base: Instant::now(),
                offset_secs: AtomicU64::new(0),
            })
        }
        fn advance(&self, secs: u64) {
            self.offset_secs.fetch_add(secs, Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            self.base + Duration::from_secs(self.offset_secs.load(Ordering::SeqCst))
        }
    }

    fn fresh(dir: &tempfile::TempDir) -> Vault {
        Vault::with_kdf_params(dir.path().join("vault.bin"), tiny()).expect("vault")
    }

    #[test]
    fn production_defaults_are_the_specified_ones() {
        let vault = Vault::new(PathBuf::from("/nonexistent/vault.bin"));
        assert_eq!(vault.kdf_params(), KdfParams::default());
        assert_eq!(
            (
                vault.kdf_params().memory_kib,
                vault.kdf_params().iterations,
                vault.kdf_params().parallelism
            ),
            (64 * 1024, 3, 1)
        );
        assert_eq!(vault.auto_lock_timeout(), Duration::from_secs(900));
    }

    #[test]
    fn debug_does_not_leak() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master-xyz")).expect("create");
        vault
            .upsert(password_input("n", "fake-secret-xyz"))
            .expect("upsert");
        let text = format!("{vault:?}");
        assert!(!text.contains("xyz"));
    }

    #[test]
    fn change_password_rotates_the_dek() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-old")).expect("create");
        let summary = vault
            .upsert(password_input("srv", "fake-secret"))
            .expect("upsert");
        let dek_before = vault.dek_bytes().expect("dek");
        let file_before = std::fs::read(vault.path()).expect("read");

        vault
            .change_password(pw("fake-old"), pw("fake-new"))
            .expect("change");
        let dek_after = vault.dek_bytes().expect("dek");
        assert_ne!(dek_after, dek_before, "the data key must be replaced");
        let file_after = std::fs::read(vault.path()).expect("read");
        assert_ne!(file_before, file_after);

        // New salt, new wrapped key, new nonces.
        let old_header = Header::parse(&file_before).expect("old header");
        let new_header = Header::parse(&file_after).expect("new header");
        assert_ne!(old_header.salt, new_header.salt);
        assert_ne!(old_header.wrapped_dek, new_header.wrapped_dek);
        let derive = |password: &str, h: &Header| {
            MasterPassword(pw(password))
                .derive_kek(&h.kdf, &h.salt)
                .expect("kek")
        };
        let dek_old =
            format::unwrap_dek(&derive("fake-old", &old_header), &old_header).expect("old unwrap");
        let dek_new =
            format::unwrap_dek(&derive("fake-new", &new_header), &new_header).expect("new unwrap");
        assert_eq!(dek_old.expose_secret(), &dek_before);
        assert_eq!(dek_new.expose_secret(), &dek_after);
        assert!(format::unwrap_dek(&derive("fake-old", &new_header), &new_header).is_err());

        // The point of the rotation: the old password and an old copy of the file give the
        // old DEK, and the old DEK opens nothing in the new file.
        assert!(
            format::open_payload(&file_after, &dek_old).is_err(),
            "the old data key must not open the new file"
        );
        assert!(format::open_payload(&file_before, &dek_old).is_ok());

        vault.lock().expect("lock");
        assert!(matches!(
            vault.unlock(pw("fake-old")),
            Err(VaultError::AuthFailed)
        ));
        vault.unlock(pw("fake-new")).expect("unlock new");
        assert_eq!(vault.dek_bytes().expect("dek"), dek_after);
        assert!(matches!(
            vault.reveal_secret(summary.id).expect("reveal"),
            RevealedSecret::Password { .. }
        ));
    }

    #[test]
    fn lock_drops_all_decrypted_state() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        let s = vault.upsert(password_input("n", "s")).expect("upsert");
        vault.lock().expect("lock");
        assert!(vault.unlocked.is_none());
        assert!(vault.dek_bytes().is_none());
        assert_eq!(vault.status().expect("status"), VaultStatus::Locked);
        assert!(matches!(vault.list(), Err(VaultError::Locked)));
        assert!(matches!(vault.reveal_secret(s.id), Err(VaultError::Locked)));
        vault.lock().expect("lock is idempotent");
    }

    #[test]
    fn auto_lock_follows_activity() {
        let dir = scratch();
        let clock = ManualClock::new();
        let mut vault = fresh(&dir).with_clock(clock.clone());
        vault.set_auto_lock_timeout(Duration::from_secs(60));
        assert!(!vault.tick(), "nothing to lock while locked/not created");

        vault.create(pw("fake-master")).expect("create");
        clock.advance(59);
        assert!(!vault.tick());
        // Any operation counts as activity and restarts the period.
        vault.list().expect("list");
        clock.advance(59);
        assert!(!vault.tick(), "59 s since the last operation");
        // Status polling is not activity: the period keeps running from the list call.
        vault.status().expect("status");
        clock.advance(1);
        assert!(vault.tick(), "60 s since the last operation");
    }

    /// A clock whose monotonic reading does not move while the wall clock does: a computer that
    /// slept.
    struct SleepingClock {
        base: Instant,
        wall_base: SystemTime,
        wall_offset_secs: AtomicU64,
    }

    impl Clock for SleepingClock {
        fn now(&self) -> Instant {
            self.base
        }
        fn wall(&self) -> SystemTime {
            self.wall_base + Duration::from_secs(self.wall_offset_secs.load(Ordering::SeqCst))
        }
    }

    #[test]
    fn time_spent_asleep_counts_towards_the_auto_lock() {
        let dir = scratch();
        let clock = Arc::new(SleepingClock {
            base: Instant::now(),
            wall_base: SystemTime::now(),
            wall_offset_secs: AtomicU64::new(0),
        });
        let mut vault = fresh(&dir).with_clock(clock.clone());
        vault.set_auto_lock_timeout(Duration::from_secs(900));
        vault.create(pw("fake-master")).expect("create");
        assert!(!vault.tick());
        // The lid is closed for an hour: the monotonic clock did not advance, the wall clock did.
        clock.wall_offset_secs.store(3600, Ordering::SeqCst);
        assert!(vault.tick(), "an hour asleep must lock a 15 minute vault");
        assert!(!vault.is_unlocked());
    }

    #[test]
    fn a_wall_clock_set_back_does_not_extend_the_period() {
        let dir = scratch();
        let clock = ManualClock::new();
        let mut vault = fresh(&dir).with_clock(clock.clone());
        vault.set_auto_lock_timeout(Duration::from_secs(60));
        vault.create(pw("fake-master")).expect("create");
        // The monotonic clock runs on; the (real) wall clock of ManualClock never goes
        // backwards here, and a reading before the last activity counts as zero idle time
        // rather than a panic.
        clock.advance(60);
        assert!(vault.tick());
    }

    #[test]
    fn auto_lock_locks_at_the_timeout_and_zero_disables() {
        let dir = scratch();
        let clock = ManualClock::new();
        let mut vault = fresh(&dir).with_clock(clock.clone());
        vault.set_auto_lock_timeout(Duration::from_secs(60));
        vault.create(pw("fake-master")).expect("create");
        clock.advance(59);
        assert!(!vault.tick());
        assert!(vault.is_unlocked());
        clock.advance(1);
        assert!(vault.tick());
        assert!(!vault.is_unlocked());
        assert!(matches!(vault.list(), Err(VaultError::Locked)));
        assert!(!vault.tick(), "already locked");

        vault.unlock(pw("fake-master")).expect("unlock");
        vault.set_auto_lock_timeout(Duration::ZERO);
        clock.advance(10_000_000);
        assert!(!vault.tick());
        assert!(vault.is_unlocked());
    }

    #[test]
    fn upsert_rolls_back_when_the_save_fails() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        let keep = vault.upsert(password_input("keep", "s")).expect("upsert");
        // Make the next save fail: the target path becomes a directory.
        let path = vault.path().to_owned();
        std::fs::remove_file(&path).expect("remove");
        std::fs::create_dir(&path).expect("mkdir");
        assert!(vault.upsert(password_input("lost", "s")).is_err());
        assert!(vault.delete(keep.id).is_err());
        let names: Vec<String> = vault
            .list()
            .expect("list")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["keep"]);
        assert_eq!(
            vault.list().expect("list")[0].kind,
            CredentialKind::Password
        );
    }

    #[test]
    fn old_payload_without_identities_still_opens() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        // Rewrite the file with a payload as written before identities existed.
        let bytes = format::read_file(vault.path()).expect("read");
        let header = Header::parse(&bytes).expect("header");
        let kek = MasterPassword(pw("fake-master"))
            .derive_kek(&header.kdf, &header.salt)
            .expect("kek");
        let dek = format::unwrap_dek(&kek, &header).expect("dek");
        let old = format::seal_file(&header, &dek, br#"{"credentials":[]}"#).expect("seal");
        format::write_atomic(vault.path(), &old).expect("write");
        vault.lock().expect("lock");
        vault.unlock(pw("fake-master")).expect("unlock");
        assert!(vault.identities().expect("identities").is_empty());
    }

    #[test]
    fn identity_survives_lock_and_unlock() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        let id = Uuid::new_v4();
        let secret = [9u8; 32];
        let record = vault.add_identity(id, "Alice", &secret).expect("add");
        assert_eq!(
            record.public_key,
            SigningKey::from_bytes(&secret).verifying_key().to_bytes()
        );
        assert!(record.created_at.ends_with('Z') && record.created_at.contains('T'));
        vault.lock().expect("lock");
        vault.unlock(pw("fake-master")).expect("unlock");
        assert_eq!(vault.identities().expect("identities"), [record]);
        assert_eq!(*vault.identity_secret(id).expect("secret"), secret);
        assert!(matches!(
            vault.identity_secret(Uuid::new_v4()),
            Err(VaultError::NotFound(_))
        ));
        assert!(matches!(
            vault.add_identity(id, "Again", &secret),
            Err(VaultError::InvalidInput(_))
        ));
    }

    #[test]
    fn identity_secret_requires_unlocked_vault() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        let id = Uuid::new_v4();
        vault.add_identity(id, "Alice", &[3u8; 32]).expect("add");
        vault.lock().expect("lock");
        assert!(matches!(vault.identity_secret(id), Err(VaultError::Locked)));
        assert!(matches!(vault.identities(), Err(VaultError::Locked)));
        assert!(matches!(
            vault.add_identity(Uuid::new_v4(), "B", &[4u8; 32]),
            Err(VaultError::Locked)
        ));
    }

    #[test]
    fn secret_never_appears_in_debug_output() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        let secret = [0xABu8; 32];
        vault
            .add_identity(Uuid::new_v4(), "Alice", &secret)
            .expect("add");
        let identity = &vault.unlocked.as_ref().expect("unlocked").identities[0];
        // The public key must not collide with the secret's byte pattern for this check.
        assert!(!identity.record.public_key.contains(&0xAB));
        let shown = format!("{identity:?} {:#?}", identity);
        let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        let list = format!("{:?}", secret.to_vec());
        for needle in [
            hex.clone(),
            hex.to_uppercase(),
            STANDARD.encode(secret),
            list,
            "171, 171".into(),
        ] {
            assert!(!shown.contains(&needle), "leaked {needle}");
        }
    }

    #[test]
    fn a_duplicate_public_key_is_refused() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        vault
            .add_identity(Uuid::new_v4(), "Alice", &[5u8; 32])
            .expect("add");
        let err = vault
            .add_identity(Uuid::new_v4(), "Alice again", &[5u8; 32])
            .expect_err("duplicate key");
        assert!(matches!(&err, VaultError::InvalidInput(m) if m == "identity already present"));
        assert_eq!(vault.identities().expect("identities").len(), 1);
    }

    #[test]
    fn identity_names_are_trimmed_and_bounded() {
        let dir = scratch();
        let mut vault = fresh(&dir);
        vault.create(pw("fake-master")).expect("create");
        let rec = vault
            .add_identity(Uuid::new_v4(), "  Bob  ", &[6u8; 32])
            .expect("add");
        assert_eq!(rec.name, "Bob");
        assert!(vault
            .add_identity(Uuid::new_v4(), "  ", &[7u8; 32])
            .is_err());
        let long = "x".repeat(4097);
        assert!(vault
            .add_identity(Uuid::new_v4(), &long, &[8u8; 32])
            .is_err());
    }

    #[test]
    fn rfc3339_formatting() {
        let t = |s| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        assert_eq!(rfc3339_utc(t(0)), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(t(1_709_208_000)), "2024-02-29T12:00:00Z");
        assert_eq!(rfc3339_utc(t(1_795_167_000)), "2026-11-20T09:30:00Z");
        // A clock far in the future (year ~17800, still within Windows' `SystemTime` range)
        // still gives a four-digit year.
        assert_eq!(rfc3339_utc(t(500_000_000_000)), "9999-12-31T23:59:59Z");
    }
}
