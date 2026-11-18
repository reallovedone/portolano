//! Quick unlock: reopen the vault with Touch ID / Windows Hello while the app stays open.
//!
//! The design (PIANO_TECNICO sections 13.2 P4 and 13.6) in short:
//!
//! * The `vault` crate is untouched. [`CapturingUnlock`] wraps `MasterPassword` and copies the
//!   key-encryption key (KEK) that Argon2id derives during a password unlock.
//! * [`QuickUnlock`] keeps that KEK only in process memory, encrypted with XChaCha20-Poly1305
//!   (the vault's own AEAD) under a random key that also lives only in memory. Nothing is written
//!   to disk or to the system keychain, and every buffer is zeroized.
//! * The key is dropped when the app exits, when the option is turned off, when the master
//!   password changes (the data key rotates, so the old KEK is useless) and after any failed
//!   quick unlock. The master password always keeps working.
//! * A quick unlock asks the operating system for a biometric check first ([`BiometricVerifier`],
//!   implemented per platform) and only then calls `Vault::unlock_with` with [`HeldUnlock`]. The
//!   check never runs while the vault mutex is held.
//!
//! The fingerprint is a gate of the app, not of the system: it is acceptable because the KEK
//! exists in the process only after the user typed the password in this run.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(any(target_os = "macos", windows, test))]
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(any(target_os = "macos", windows, test))]
use std::time::Duration;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use portolano_vault::{KdfParams, Kek, MasterPassword, UnlockMethod, Vault, VaultError};
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox, SecretString};
use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{AppError, ErrorCode};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as platform;
#[cfg(not(any(target_os = "macos", windows)))]
mod other;
#[cfg(not(any(target_os = "macos", windows)))]
use self::other as platform;

/// The reason the system dialog shows (macOS) or the message of the Hello prompt (Windows).
#[cfg(any(target_os = "macos", windows))]
pub const REASON: &str = "unlock the Portolano vault";

/// How long a system prompt may stay open before it is abandoned (and cancelled where the API
/// allows) so that a prompt that never answers cannot block the request forever.
#[cfg(any(target_os = "macos", windows))]
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Consecutive failed biometric checks (not cancellations) after which the held key is dropped
/// and the master password is required.
const MAX_FAILURES: u32 = 5;

/// Waits for the answer of a system prompt. On timeout (or if the sender vanished) `on_timeout`
/// runs (invalidate / cancel the request) and the check counts as failed.
#[cfg(any(target_os = "macos", windows, test))]
pub fn wait_for_reply(
    rx: &Receiver<Result<(), VerifyError>>,
    timeout: Duration,
    on_timeout: impl FnOnce(),
) -> Result<(), VerifyError> {
    match rx.recv_timeout(timeout) {
        Ok(outcome) => outcome,
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            on_timeout();
            Err(VerifyError::Failed)
        }
    }
}

/// Associated data of the in-memory wrapping: binds the ciphertext to its purpose.
const WRAP_AAD: &[u8] = b"portolano quick unlock v1";

/// Why a biometric check did not succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// The user dismissed the request (cancel, "use password", system or app cancel).
    // Only the macOS/Windows verifiers construct it.
    #[cfg_attr(not(any(target_os = "macos", windows, test)), allow(dead_code))]
    Cancelled,
    /// The check failed (no match, lockout, device busy, not configured, ...).
    Failed,
}

/// The operating system's biometric check. The implementations hold no state and never see the
/// vault: they only answer "was this the device owner, right now?".
pub trait BiometricVerifier: Send + Sync {
    /// Whether the system can verify the user now (Touch ID or Windows Hello configured).
    fn available(&self) -> bool;
    /// Shows the system prompt and waits for the answer. `owner` is the native handle of the
    /// main window (Windows only, `None` elsewhere). Blocks: never call it with the vault mutex
    /// held or on the async runtime's core threads.
    fn verify(&self, owner: Option<isize>) -> Result<(), VerifyError>;
}

/// What the UI needs to decide whether to show the toggle and the biometric prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickUnlockInfo {
    /// The system has a usable biometric method (the toggle is shown only then).
    pub available: bool,
    /// The option is on (saved setting).
    pub enabled: bool,
    /// A quick unlock can be attempted right now: enabled and a key is held.
    pub ready: bool,
}

/// The KEK, encrypted under a random key. Both exist only in this process.
struct Held {
    wrap_key: SecretBox<[u8; 32]>,
    nonce: [u8; 24],
    sealed: Vec<u8>,
}

impl Held {
    fn seal(kek: &Kek) -> Result<Self, AppError> {
        let mut wrap_key = SecretBox::<[u8; 32]>::init_with_mut(|_| {});
        let mut nonce = [0u8; 24];
        if getrandom::fill(wrap_key.expose_secret_mut()).is_err()
            || getrandom::fill(&mut nonce).is_err()
        {
            return Err(AppError::internal("the system random generator failed"));
        }
        let cipher = XChaCha20Poly1305::new_from_slice(wrap_key.expose_secret())
            .map_err(|_| AppError::internal("invalid cipher key length"))?;
        let sealed = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: kek.expose_secret(),
                    aad: WRAP_AAD,
                },
            )
            .map_err(|_| AppError::internal("could not protect the quick unlock key"))?;
        Ok(Self {
            wrap_key,
            nonce,
            sealed,
        })
    }

    fn open(&self) -> Result<Kek, AppError> {
        let failed = || AppError::new(ErrorCode::QuickUnlockFailed, "the held key is not usable");
        let cipher = XChaCha20Poly1305::new_from_slice(self.wrap_key.expose_secret())
            .map_err(|_| failed())?;
        let plain = cipher
            .decrypt(
                &XNonce::from(self.nonce),
                Payload {
                    msg: &self.sealed,
                    aad: WRAP_AAD,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| failed())?;
        copy_kek(&plain).ok_or_else(failed)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // `wrap_key` zeroizes itself; the rest is wiped here.
        self.nonce.zeroize();
        self.sealed.zeroize();
    }
}

/// A new `Kek` with the 32 bytes of `bytes`, or `None` for any other length.
fn copy_kek(bytes: &[u8]) -> Option<Kek> {
    if bytes.len() != 32 {
        return None;
    }
    Some(Kek::init_with_mut(|out| out.copy_from_slice(bytes)))
}

/// `MasterPassword` that also copies the KEK it derives, for [`QuickUnlock::hold`].
pub struct CapturingUnlock {
    inner: MasterPassword,
    captured: Mutex<Option<Kek>>,
}

impl CapturingUnlock {
    pub fn new(password: SecretString) -> Self {
        Self {
            inner: MasterPassword(password),
            captured: Mutex::new(None),
        }
    }

    fn take(&self) -> Option<Kek> {
        match self.captured.lock() {
            Ok(mut slot) => slot.take(),
            Err(poison) => poison.into_inner().take(),
        }
    }
}

impl UnlockMethod for CapturingUnlock {
    fn id(&self) -> &'static str {
        self.inner.id()
    }

    fn derive_kek(&self, params: &KdfParams, salt: &[u8; 16]) -> Result<Kek, VaultError> {
        let kek = self.inner.derive_kek(params, salt)?;
        let copy = copy_kek(kek.expose_secret())
            .ok_or_else(|| VaultError::Internal("unexpected key length".into()))?;
        match self.captured.lock() {
            Ok(mut slot) => *slot = Some(copy),
            Err(poison) => *poison.into_inner() = Some(copy),
        }
        Ok(kek)
    }
}

/// The unlock method of a quick unlock: hands back the held KEK. Fails (like a wrong password)
/// when nothing is held, so `unlock_with` never succeeds without a key.
struct HeldUnlock<'a> {
    quick: &'a QuickUnlock,
    missing: AtomicBool,
}

impl UnlockMethod for HeldUnlock<'_> {
    fn id(&self) -> &'static str {
        "quick-unlock"
    }

    fn derive_kek(&self, _params: &KdfParams, _salt: &[u8; 16]) -> Result<Kek, VaultError> {
        let held = self.quick.held();
        match held.as_ref() {
            None => {
                self.missing.store(true, Ordering::SeqCst);
                Err(VaultError::AuthFailed)
            }
            Some(h) => h
                .open()
                .map_err(|_| VaultError::Internal("quick unlock key is not usable".into())),
        }
    }
}

/// Resets the "a request is open" flag when the biometric check ends, however it ends.
struct PromptGuard<'a>(&'a AtomicBool);

impl Drop for PromptGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The in-memory state of quick unlock; lives in `AppState`.
pub struct QuickUnlock {
    verifier: Arc<dyn BiometricVerifier>,
    held: Mutex<Option<Held>>,
    prompting: AtomicBool,
    /// Failed biometric checks since the last successful unlock.
    failures: AtomicU32,
}

impl fmt::Debug for QuickUnlock {
    /// Never prints key material: only whether a key is held.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuickUnlock")
            .field("ready", &self.is_held())
            .finish_non_exhaustive()
    }
}

impl Default for QuickUnlock {
    fn default() -> Self {
        Self::new(Arc::new(platform::SystemVerifier))
    }
}

impl QuickUnlock {
    pub fn new(verifier: Arc<dyn BiometricVerifier>) -> Self {
        Self {
            verifier,
            held: Mutex::new(None),
            prompting: AtomicBool::new(false),
            failures: AtomicU32::new(0),
        }
    }

    /// The slot. A poisoned lock means a panic happened while the key was handled: fail closed
    /// (the key is dropped) and carry on.
    fn held(&self) -> MutexGuard<'_, Option<Held>> {
        match self.held.lock() {
            Ok(guard) => guard,
            Err(poison) => {
                let mut guard = poison.into_inner();
                *guard = None;
                self.held.clear_poison();
                guard
            }
        }
    }

    fn is_held(&self) -> bool {
        self.held().is_some()
    }

    /// Whether the system can verify the user (blocking on Windows: call off the async threads).
    pub fn available(&self) -> bool {
        self.verifier.available()
    }

    /// Whether a quick unlock can be attempted: the option is on and a key is held.
    pub fn ready(&self, enabled: bool) -> bool {
        enabled && self.is_held()
    }

    /// Drops the held key (zeroized). Idempotent.
    pub fn clear(&self) {
        *self.held() = None;
        self.failures.store(0, Ordering::SeqCst);
    }

    fn hold(&self, kek: &Kek) -> Result<(), AppError> {
        let sealed = Held::seal(kek)?;
        *self.held() = Some(sealed);
        Ok(())
    }

    /// Unlocks the vault with the master password. With the option on, the KEK derived for it
    /// is kept for later quick unlocks; with it off nothing is captured and any old key is
    /// dropped. A wrong password changes nothing.
    pub fn unlock_with_password(
        &self,
        vault: &mut Vault,
        password: SecretString,
        enabled: bool,
    ) -> Result<(), VaultError> {
        if !enabled {
            self.clear();
            return vault.unlock(password);
        }
        let method = CapturingUnlock::new(password);
        vault.unlock_with(&method)?;
        self.failures.store(0, Ordering::SeqCst);
        match method.take() {
            Some(kek) => {
                if let Err(e) = self.hold(&kek) {
                    // The unlock itself worked; quick unlock simply is not ready.
                    tracing::warn!(error = %e, "quick unlock key could not be kept");
                    self.clear();
                }
            }
            None => self.clear(),
        }
        Ok(())
    }

    /// Changes the master password. The data key rotates, so the held KEK stops being valid: it
    /// is dropped unless the change was refused before anything happened (wrong current
    /// password, invalid new password, locked vault).
    pub fn change_password(
        &self,
        vault: &mut Vault,
        current: SecretString,
        new: SecretString,
    ) -> Result<(), VaultError> {
        let result = vault.change_password(current, new);
        match &result {
            Err(
                VaultError::AuthFailed
                | VaultError::InvalidInput(_)
                | VaultError::Locked
                | VaultError::NotCreated,
            ) => {}
            _ => self.clear(),
        }
        result
    }

    /// First half of a quick unlock, with no lock held: the option must be on, a key must be
    /// held, and the system must confirm the user. One request at a time.
    pub fn authorize(&self, enabled: bool, owner: Option<isize>) -> Result<(), AppError> {
        if !self.ready(enabled) {
            return Err(not_ready());
        }
        if self
            .prompting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AppError::new(
                ErrorCode::QuickUnlockCancelled,
                "a quick unlock request is already open",
            ));
        }
        let _guard = PromptGuard(&self.prompting);
        match self.verifier.verify(owner) {
            Ok(()) => Ok(()),
            Err(VerifyError::Cancelled) => Err(AppError::new(
                ErrorCode::QuickUnlockCancelled,
                "quick unlock was cancelled",
            )),
            Err(VerifyError::Failed) => {
                if self.failures.fetch_add(1, Ordering::SeqCst) + 1 >= MAX_FAILURES {
                    tracing::warn!("too many failed biometric checks; the key was dropped");
                    self.clear();
                }
                Err(AppError::new(
                    ErrorCode::QuickUnlockFailed,
                    "the biometric check failed",
                ))
            }
        }
    }

    /// Second half, with the vault in hand: reopens it with the held key. `enabled` is read
    /// again by the caller after the check, so an option turned off meanwhile wins. Any failure
    /// of `unlock_with` drops the key: the password is needed again.
    pub fn open_vault(&self, vault: &mut Vault, enabled: bool) -> Result<(), AppError> {
        if !enabled {
            self.clear();
            return Err(not_ready());
        }
        let method = HeldUnlock {
            quick: self,
            missing: AtomicBool::new(false),
        };
        match vault.unlock_with(&method) {
            Ok(()) => {
                self.failures.store(0, Ordering::SeqCst);
                Ok(())
            }
            Err(_) if method.missing.load(Ordering::SeqCst) => Err(not_ready()),
            Err(e) => {
                self.clear();
                tracing::warn!(error = %e, "quick unlock failed; the key was dropped");
                Err(AppError::new(
                    ErrorCode::QuickUnlockFailed,
                    "quick unlock could not open the vault; use the master password",
                ))
            }
        }
    }
}

fn not_ready() -> AppError {
    AppError::new(
        ErrorCode::QuickUnlockNotReady,
        "quick unlock needs the master password first",
    )
}

/// The native handle of the main window, for the Windows Hello prompt (`None` elsewhere).
#[cfg(windows)]
pub fn owner_handle(app: &tauri::AppHandle) -> Option<isize> {
    use tauri::Manager;
    let window = app.get_webview_window("main")?;
    match window.hwnd() {
        Ok(hwnd) => Some(hwnd.0 as isize),
        Err(e) => {
            tracing::warn!(error = %e, "main window handle not available; the prompt has no owner");
            None
        }
    }
}

#[cfg(not(windows))]
pub fn owner_handle(_app: &tauri::AppHandle) -> Option<isize> {
    None
}

/// A fake biometric check for tests (here and in the command tests).
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{BiometricVerifier, VerifyError};

    /// Plays the system's biometric check.
    pub struct FakeVerifier {
        pub available: AtomicBool,
        pub outcome: Mutex<Result<(), VerifyError>>,
        pub calls: AtomicUsize,
        /// Runs inside `verify`, while the "system prompt" is open.
        pub during: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    }

    impl FakeVerifier {
        pub fn ok() -> Arc<Self> {
            Arc::new(Self {
                available: AtomicBool::new(true),
                outcome: Mutex::new(Ok(())),
                calls: AtomicUsize::new(0),
                during: Mutex::new(None),
            })
        }
        pub fn set(&self, outcome: Result<(), VerifyError>) {
            *self.outcome.lock().expect("outcome") = outcome;
        }
        pub fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl BiometricVerifier for FakeVerifier {
        fn available(&self) -> bool {
            self.available.load(Ordering::SeqCst)
        }
        fn verify(&self, _owner: Option<isize>) -> Result<(), VerifyError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(hook) = self.during.lock().expect("hook").as_ref() {
                hook();
            }
            *self.outcome.lock().expect("outcome")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeVerifier;
    use super::*;
    use std::path::Path;

    fn vault_in_scratch() -> (tempfile::TempDir, Vault) {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/task8");
        std::fs::create_dir_all(&base).expect("scratch base");
        let dir = tempfile::Builder::new()
            .prefix("quick-")
            .tempdir_in(base)
            .expect("tempdir");
        let tiny = KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        };
        let mut vault = Vault::with_kdf_params(dir.path().join("vault.bin"), tiny).expect("vault");
        vault
            .create(SecretString::from("fake-master"))
            .expect("create");
        vault.lock().expect("lock");
        (dir, vault)
    }

    fn pw(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn quick() -> (Arc<FakeVerifier>, QuickUnlock) {
        let fake = FakeVerifier::ok();
        (fake.clone(), QuickUnlock::new(fake))
    }

    #[test]
    fn nothing_is_held_at_the_start() {
        let (_, q) = quick();
        assert!(!q.ready(true));
        assert!(!q.ready(false));
    }

    #[test]
    fn a_password_unlock_with_the_option_on_makes_quick_unlock_ready() {
        let (_dir, mut vault) = vault_in_scratch();
        let (_, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        assert!(vault.is_unlocked());
        assert!(q.ready(true));
        assert!(!q.ready(false), "the option off means not ready");
    }

    #[test]
    fn with_the_option_off_nothing_is_captured_and_an_old_key_is_dropped() {
        let (_dir, mut vault) = vault_in_scratch();
        let (_, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), false)
            .expect("unlock");
        assert!(vault.is_unlocked());
        assert!(!q.is_held());

        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        assert!(q.is_held());
        q.unlock_with_password(&mut vault, pw("fake-master"), false)
            .expect("unlock");
        assert!(!q.is_held(), "an unlock with the option off drops the key");
    }

    #[test]
    fn a_wrong_password_keeps_the_state_and_captures_nothing() {
        let (_dir, mut vault) = vault_in_scratch();
        let (_, q) = quick();
        let err = q
            .unlock_with_password(&mut vault, pw("fake-wrong"), true)
            .expect_err("wrong password");
        assert!(matches!(err, VaultError::AuthFailed));
        assert!(!q.is_held());
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        vault.lock().expect("lock");
        q.unlock_with_password(&mut vault, pw("fake-wrong"), true)
            .expect_err("wrong password");
        assert!(q.is_held(), "a typo does not throw the key away");
    }

    #[test]
    fn locking_leaves_it_ready_and_quick_unlock_reopens_the_vault() {
        let (_dir, mut vault) = vault_in_scratch();
        let (fake, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        vault.lock().expect("lock");
        assert!(!vault.is_unlocked());
        assert!(q.ready(true), "Lock keeps quick unlock ready");

        q.authorize(true, None).expect("authorize");
        q.open_vault(&mut vault, true).expect("open");
        assert!(vault.is_unlocked());
        assert_eq!(fake.calls(), 1);
        assert!(q.ready(true), "and it stays ready after a quick unlock");

        // The master password stays valid.
        vault.lock().expect("lock");
        vault.unlock(pw("fake-master")).expect("password unlock");
    }

    #[test]
    fn turning_the_option_off_or_clearing_makes_it_not_ready() {
        let (_dir, mut vault) = vault_in_scratch();
        let (fake, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        vault.lock().expect("lock");
        let err = q.authorize(false, None).expect_err("option off");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);
        assert_eq!(fake.calls(), 0, "no prompt when it cannot succeed");
        q.clear();
        assert!(!q.ready(true));
        let err = q.authorize(true, None).expect_err("cleared");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);
        let err = q.open_vault(&mut vault, true).expect_err("cleared");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);
        assert!(!vault.is_unlocked(), "no key, no unlock");
    }

    #[test]
    fn the_option_turned_off_during_the_prompt_wins() {
        let (_dir, mut vault) = vault_in_scratch();
        let (_, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        vault.lock().expect("lock");
        q.authorize(true, None).expect("authorize");
        let err = q.open_vault(&mut vault, false).expect_err("now off");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);
        assert!(!vault.is_unlocked());
        assert!(!q.is_held());
    }

    #[test]
    fn changing_the_password_drops_the_key() {
        let (_dir, mut vault) = vault_in_scratch();
        let (_, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");

        // Refused before anything happened: the key stays.
        q.change_password(&mut vault, pw("fake-wrong"), pw("fake-new"))
            .expect_err("wrong current password");
        assert!(q.ready(true));
        q.change_password(&mut vault, pw("fake-master"), pw(""))
            .expect_err("empty new password");
        assert!(q.ready(true));

        q.change_password(&mut vault, pw("fake-master"), pw("fake-new"))
            .expect("change");
        assert!(!q.ready(true));
        vault.lock().expect("lock");
        let err = q.authorize(true, None).expect_err("not ready");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);

        // The next password unlock builds a new key.
        q.unlock_with_password(&mut vault, pw("fake-new"), true)
            .expect("unlock");
        assert!(q.ready(true));
    }

    #[test]
    fn a_stale_key_fails_the_unlock_and_is_dropped() {
        let (_dir, mut vault) = vault_in_scratch();
        let (_, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        // The password changes behind the back of `QuickUnlock`: the held KEK is stale.
        vault
            .change_password(pw("fake-master"), pw("fake-new"))
            .expect("change");
        vault.lock().expect("lock");

        q.authorize(true, None).expect("authorize");
        let err = q.open_vault(&mut vault, true).expect_err("stale key");
        assert_eq!(err.code, ErrorCode::QuickUnlockFailed);
        assert!(!vault.is_unlocked());
        assert!(!q.ready(true), "a failed unlock_with drops the key");
        vault.unlock(pw("fake-new")).expect("the password works");
    }

    #[test]
    fn a_cancelled_or_failed_check_keeps_the_key_for_a_retry() {
        let (_dir, mut vault) = vault_in_scratch();
        let (fake, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        vault.lock().expect("lock");

        fake.set(Err(VerifyError::Cancelled));
        let err = q.authorize(true, None).expect_err("cancelled");
        assert_eq!(err.code, ErrorCode::QuickUnlockCancelled);
        fake.set(Err(VerifyError::Failed));
        let err = q.authorize(true, None).expect_err("failed");
        assert_eq!(err.code, ErrorCode::QuickUnlockFailed);
        assert!(q.ready(true));
        assert!(!vault.is_unlocked());

        fake.set(Ok(()));
        q.authorize(true, None).expect("retry");
        q.open_vault(&mut vault, true).expect("open");
        assert!(vault.is_unlocked());
    }

    /// Calls `authorize` from the verifier itself, as a second request would while the first
    /// prompt is still open.
    struct ReentrantVerifier(Mutex<Option<Arc<QuickUnlock>>>, Mutex<Option<AppError>>);

    impl BiometricVerifier for ReentrantVerifier {
        fn available(&self) -> bool {
            true
        }
        fn verify(&self, _: Option<isize>) -> Result<(), VerifyError> {
            let inner = self.0.lock().expect("slot").clone();
            if let Some(q) = inner {
                *self.1.lock().expect("slot") = q.authorize(true, None).err();
            }
            Ok(())
        }
    }

    #[test]
    fn only_one_biometric_request_is_open_at_a_time() {
        let (_dir, mut vault) = vault_in_scratch();
        let verifier = Arc::new(ReentrantVerifier(Mutex::new(None), Mutex::new(None)));
        let q = Arc::new(QuickUnlock::new(verifier.clone()));
        *verifier.0.lock().expect("slot") = Some(q.clone());
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        q.authorize(true, None).expect("first request");
        let second = verifier.1.lock().expect("slot").take().expect("refused");
        assert_eq!(second.code, ErrorCode::QuickUnlockCancelled);
        // The flag is released afterwards.
        *verifier.0.lock().expect("slot") = None;
        q.authorize(true, None).expect("a later request works");
    }

    #[test]
    fn five_failed_checks_in_a_row_drop_the_key_and_a_success_resets_the_count() {
        let (_dir, mut vault) = vault_in_scratch();
        let (fake, q) = quick();
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        vault.lock().expect("lock");
        fake.set(Err(VerifyError::Failed));
        for _ in 0..4 {
            q.authorize(true, None).expect_err("failed");
        }
        assert!(q.ready(true), "four failures keep the key");
        // Cancellations do not count.
        fake.set(Err(VerifyError::Cancelled));
        q.authorize(true, None).expect_err("cancelled");
        // A successful unlock resets the count.
        fake.set(Ok(()));
        q.authorize(true, None).expect("ok");
        q.open_vault(&mut vault, true).expect("open");
        vault.lock().expect("lock");
        fake.set(Err(VerifyError::Failed));
        for _ in 0..4 {
            q.authorize(true, None).expect_err("failed");
        }
        assert!(q.ready(true), "the count restarted after the success");
        let err = q.authorize(true, None).expect_err("fifth failure");
        assert_eq!(err.code, ErrorCode::QuickUnlockFailed);
        assert!(!q.ready(true), "the fifth failure drops the key");
        let err = q.authorize(true, None).expect_err("needs the password");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);
    }

    #[test]
    fn a_prompt_that_never_answers_times_out_and_releases_the_request() {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), VerifyError>>();
        let mut gave_up = false;
        let out = wait_for_reply(&rx, Duration::from_millis(20), || gave_up = true);
        assert_eq!(out, Err(VerifyError::Failed));
        assert!(gave_up, "the request is invalidated on timeout");
        drop(tx);

        // A verifier that times out like that leaves the single-flight flag free.
        struct Hanging;
        impl BiometricVerifier for Hanging {
            fn available(&self) -> bool {
                true
            }
            fn verify(&self, _: Option<isize>) -> Result<(), VerifyError> {
                let (_tx, rx) = std::sync::mpsc::channel();
                wait_for_reply(&rx, Duration::from_millis(10), || {})
            }
        }
        let (_dir, mut vault) = vault_in_scratch();
        let q = QuickUnlock::new(Arc::new(Hanging));
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        let err = q.authorize(true, None).expect_err("timed out");
        assert_eq!(err.code, ErrorCode::QuickUnlockFailed);
        let err = q.authorize(true, None).expect_err("not busy afterwards");
        assert_eq!(err.code, ErrorCode::QuickUnlockFailed, "not 'already open'");
    }

    #[test]
    fn the_held_key_is_encrypted_and_round_trips() {
        let kek = Kek::init_with_mut(|b| b.copy_from_slice(&[0xAB; 32]));
        let held = Held::seal(&kek).expect("seal");
        assert_eq!(held.sealed.len(), 32 + 16, "key plus the AEAD tag");
        assert!(
            !held.sealed.windows(8).any(|w| w == [0xAB; 8]),
            "the KEK must not appear in the clear"
        );
        let back = held.open().expect("open");
        assert_eq!(back.expose_secret(), kek.expose_secret());

        let mut tampered = Held::seal(&kek).expect("seal");
        tampered.sealed[0] ^= 1;
        assert!(tampered.open().is_err());
        let a = Held::seal(&kek).expect("seal");
        let b = Held::seal(&kek).expect("seal");
        assert_ne!(a.sealed, b.sealed, "fresh key and nonce every time");
    }

    #[test]
    fn debug_prints_no_key_bytes() {
        let (_dir, mut vault) = vault_in_scratch();
        let (_, q) = quick();
        assert_eq!(format!("{q:?}"), "QuickUnlock { ready: false, .. }");
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        let text = format!("{q:?} {q:#?}");
        assert_eq!(
            text,
            "QuickUnlock { ready: true, .. } QuickUnlock {\n    ready: true,\n    ..\n}"
        );
        let held = q.held();
        let h = held.as_ref().expect("held");
        let key = h.wrap_key.expose_secret();
        for byte_text in [format!("{key:?}"), format!("{:?}", h.sealed)] {
            assert!(!text.contains(&byte_text));
        }
    }

    #[test]
    fn a_poisoned_slot_fails_closed() {
        let (_dir, mut vault) = vault_in_scratch();
        let q = Arc::new(QuickUnlock::new(FakeVerifier::ok()));
        q.unlock_with_password(&mut vault, pw("fake-master"), true)
            .expect("unlock");
        let clone = q.clone();
        let joined = std::thread::spawn(move || {
            let _guard = clone.held.lock();
            panic!("simulated panic while the key was handled");
        })
        .join();
        assert!(joined.is_err());
        assert!(!q.ready(true), "the key is dropped, not trusted");
        assert!(!q.held.is_poisoned());
    }
}
