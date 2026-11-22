//! `vault_*` commands and the auto-lock ticker.
//!
//! Argon2id is slow by design, so every command runs on the blocking thread pool: the async
//! runtime's core threads (and with them the UI) never wait for key derivation or for the
//! vault mutex. Any vault operation, successful or not, counts as activity for auto-lock
//! (`vault_status` excepted: the UI polls it).

use std::sync::{Arc, MutexGuard};
use std::time::Duration;

use portolano_vault::{
    CredentialSummary, CredentialUpsert, RevealedSecret, Vault, VaultError, VaultStatus,
};
use secrecy::SecretString;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::commands::model::with_store;
use crate::error::{AppError, ErrorCode};
use crate::quick_unlock::{self, QuickUnlockInfo};
use crate::settings::{auto_lock_duration, validate_auto_lock_minutes, SettingsStore};
use crate::state::AppState;

/// Event (to the main webview, no payload) emitted whenever the vault becomes locked: the
/// `vault_lock` command and the auto-lock timer. The UI updates its state immediately.
pub const VAULT_LOCKED_EVENT: &str = "vault-locked";

fn emit_locked(app: &AppHandle) {
    if let Err(e) = app.emit_to("main", VAULT_LOCKED_EVENT, ()) {
        tracing::debug!(error = %e, "vault-locked event not delivered");
    }
}

/// How often the inactivity timer is evaluated. The lock happens at most this much after the
/// configured timeout.
const AUTO_LOCK_TICK: Duration = Duration::from_secs(5);

/// The vault mutex. A poisoned mutex means a panic happened while the vault was in use, so its
/// in-memory state cannot be trusted: fail closed. The guard is recovered, the vault is locked
/// (the key and every secret are zeroised, the user unlocks again from the file) and the
/// poison is cleared, so the vault stays usable and, above all, lockable.
fn lock_vault(state: &AppState) -> Result<MutexGuard<'_, Vault>, AppError> {
    crate::state::lock_vault_mutex(&state.vault)
}

/// Runs `op` on the blocking pool with the vault mutex held.
async fn with_vault<T, F>(app: AppHandle, op: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&mut Vault) -> Result<T, VaultError> + Send + 'static,
{
    with_vault_state(app, move |_, vault| op(vault).map_err(AppError::from)).await
}

/// `with_vault` for operations that also need the rest of the app state (quick unlock).
async fn with_vault_state<T, F>(app: AppHandle, op: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&AppState, &mut Vault) -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut vault = lock_vault(&state)?;
        op(&state, &mut vault)
    })
    .await
    .map_err(|e| {
        // A panic inside the vault must not take the app down; the message carries no data.
        tracing::error!(error = %e, "vault task failed");
        AppError::internal("vault task failed")
    })?
}

#[tauri::command]
pub async fn vault_status(app: AppHandle) -> Result<VaultStatus, AppError> {
    with_vault(app, |v| v.status()).await
}

#[tauri::command]
pub async fn vault_create(app: AppHandle, master_password: SecretString) -> Result<(), AppError> {
    with_vault(app, move |v| v.create(master_password)).await
}

/// Unlocks with the master password. With quick unlock on, the key derived here is kept in
/// memory for later Touch ID / Windows Hello unlocks. The setting is read with the vault mutex
/// held, so it cannot change between the unlock and the capture.
fn unlock_with_password(
    state: &AppState,
    vault: &mut Vault,
    settings: &SettingsStore,
    password: SecretString,
) -> Result<(), AppError> {
    // The master password always works: an unreadable setting counts as "off" (which also drops
    // any held key) instead of blocking the unlock.
    let enabled = match settings.get() {
        Ok(current) => current.quick_unlock,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the settings; unlocking without quick unlock");
            false
        }
    };
    state
        .quick_unlock
        .unlock_with_password(vault, password, enabled)
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn vault_unlock(
    app: AppHandle,
    settings: State<'_, Arc<SettingsStore>>,
    master_password: SecretString,
) -> Result<(), AppError> {
    let settings = Arc::clone(&settings);
    with_vault_state(app, move |state, vault| {
        unlock_with_password(state, vault, &settings, master_password)
    })
    .await
}

#[tauri::command]
pub async fn vault_change_password(
    app: AppHandle,
    current_password: SecretString,
    new_password: SecretString,
) -> Result<(), AppError> {
    with_vault_state(app, move |state, vault| {
        state
            .quick_unlock
            .change_password(vault, current_password, new_password)
            .map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub async fn vault_lock(app: AppHandle) -> Result<(), AppError> {
    with_vault(app.clone(), |v| v.lock()).await?;
    emit_locked(&app);
    Ok(())
}

#[tauri::command]
pub async fn vault_list(app: AppHandle) -> Result<Vec<CredentialSummary>, AppError> {
    with_vault(app, |v| v.list()).await
}

#[tauri::command]
pub async fn vault_upsert(
    app: AppHandle,
    input: CredentialUpsert,
) -> Result<CredentialSummary, AppError> {
    with_vault(app, move |v| v.upsert_update(input)).await
}

/// Deletes a credential, then turns every connection or folder reference to it into "no
/// credential". If updating the document fails the error is reported, but the credential stays
/// deleted (it is never resurrected).
#[tauri::command]
pub async fn vault_delete(app: AppHandle, id: Uuid) -> Result<(), AppError> {
    with_vault(app.clone(), move |v| v.delete(id)).await?;
    clear_references(&app.state::<AppState>(), id)
}

fn clear_references(state: &AppState, id: Uuid) -> Result<(), AppError> {
    with_store(state, |store| {
        store.clear_credential_references(id)?;
        // Local choices for shared connections that pointed at it go too.
        state.shares.forget_credential(store, id)?;
        Ok(())
    })
    .map(|_| ())
    .map_err(|e| {
        AppError::new(
            e.code,
            format!(
                "The credential was deleted, but connections that used it could not be updated: {}",
                e.message
            ),
        )
    })
}

/// The only command that returns secret material, and only on an explicit user action.
#[tauri::command]
pub async fn vault_reveal_secret(app: AppHandle, id: Uuid) -> Result<RevealedSecret, AppError> {
    let secret = with_vault(app.clone(), move |v| v.reveal_secret(id)).await?;
    // A revealed secret may end up on the clipboard: remember its hash so that a shared RDP
    // clipboard never offers it to a remote desktop (see `clipboard.rs`).
    if let Some(state) = app.try_state::<AppState>() {
        let marks = state.clipboard.secrets();
        match &secret {
            RevealedSecret::Password { password } => marks.mark(password),
            RevealedSecret::SshKey {
                private_key_pem,
                passphrase,
            } => {
                marks.mark(private_key_pem);
                if let Some(passphrase) = passphrase {
                    marks.mark(passphrase);
                }
            }
        }
    }
    Ok(secret)
}

/// The saved auto-lock time, in minutes (`0` = never).
fn get_auto_lock(settings: &SettingsStore) -> Result<u32, AppError> {
    Ok(settings.get()?.auto_lock_minutes)
}

/// Validates, saves and applies at once a new auto-lock time. Nothing changes when it is refused
/// or cannot be saved.
fn set_auto_lock(state: &AppState, settings: &SettingsStore, minutes: u32) -> Result<(), AppError> {
    let minutes = validate_auto_lock_minutes(minutes)?;
    // The vault first: if its mutex cannot be taken nothing is saved. If saving then fails, the
    // vault goes back to its previous timeout, so file and vault never disagree.
    let previous = {
        let mut vault = lock_vault(state)?;
        let previous = vault.auto_lock_timeout();
        vault.set_auto_lock_timeout(auto_lock_duration(minutes));
        previous
    };
    if let Err(e) = settings.update(|s| s.auto_lock_minutes = minutes) {
        lock_vault(state)?.set_auto_lock_timeout(previous);
        return Err(e);
    }
    Ok(())
}

/// Gives the vault the saved auto-lock time (start-up, before the first tick of the timer).
pub fn apply_saved_auto_lock(state: &AppState, settings: &SettingsStore) -> Result<(), AppError> {
    let minutes = get_auto_lock(settings)?;
    lock_vault(state)?.set_auto_lock_timeout(auto_lock_duration(minutes));
    Ok(())
}

#[tauri::command]
pub async fn vault_get_auto_lock(settings: State<'_, Arc<SettingsStore>>) -> Result<u32, AppError> {
    get_auto_lock(&settings)
}

#[tauri::command]
pub async fn vault_set_auto_lock(
    app: AppHandle,
    settings: State<'_, Arc<SettingsStore>>,
    minutes: u32,
) -> Result<(), AppError> {
    let settings = Arc::clone(&settings);
    tauri::async_runtime::spawn_blocking(move || {
        set_auto_lock(&app.state::<AppState>(), &settings, minutes)
    })
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "auto-lock task failed");
        AppError::internal("auto-lock task failed")
    })?
}

/// What the UI needs for quick unlock: the system has a biometric method, the option is on, a
/// key is held. `available` may ask the system (Windows), so this runs off the async threads.
fn quick_unlock_info(
    state: &AppState,
    settings: &SettingsStore,
) -> Result<QuickUnlockInfo, AppError> {
    let enabled = settings.get()?.quick_unlock;
    Ok(QuickUnlockInfo {
        available: state.quick_unlock.available(),
        enabled,
        ready: state.quick_unlock.ready(enabled),
    })
}

/// Turns quick unlock on or off. Turning it off drops the held key first. Turning it on does not
/// create a key (there is no password at hand): the next unlock with the master password does.
/// Holding the vault mutex orders this against a password unlock that is capturing the key.
fn set_quick_unlock(
    state: &AppState,
    settings: &SettingsStore,
    enabled: bool,
) -> Result<(), AppError> {
    if enabled && !state.quick_unlock.available() {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Touch ID / Windows Hello is not available on this device",
        ));
    }
    let _vault = lock_vault(state)?;
    if !enabled {
        state.quick_unlock.clear();
    }
    settings.update(|s| s.quick_unlock = enabled)?;
    Ok(())
}

/// The quick unlock sequence. The biometric prompt runs with NO lock held (it can stay open for
/// as long as the user likes); only the final `unlock_with` takes the vault mutex, and the option
/// is read again there, so turning it off meanwhile wins.
fn quick_unlock_flow(
    state: &AppState,
    settings: &SettingsStore,
    owner: Option<isize>,
) -> Result<(), AppError> {
    let enabled = settings.get()?.quick_unlock;
    state.quick_unlock.authorize(enabled, owner)?;
    let mut vault = lock_vault(state)?;
    let enabled = settings.get()?.quick_unlock;
    state.quick_unlock.open_vault(&mut vault, enabled)
}

async fn blocking<T, F>(what: &'static str, op: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(op)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, task = what, "task failed");
            AppError::internal(format!("{what} task failed"))
        })?
}

#[tauri::command]
pub async fn vault_quick_unlock_info(
    app: AppHandle,
    settings: State<'_, Arc<SettingsStore>>,
) -> Result<QuickUnlockInfo, AppError> {
    let settings = Arc::clone(&settings);
    blocking("quick unlock", move || {
        quick_unlock_info(&app.state::<AppState>(), &settings)
    })
    .await
}

#[tauri::command]
pub async fn vault_set_quick_unlock(
    app: AppHandle,
    settings: State<'_, Arc<SettingsStore>>,
    enabled: bool,
) -> Result<(), AppError> {
    let settings = Arc::clone(&settings);
    blocking("quick unlock", move || {
        set_quick_unlock(&app.state::<AppState>(), &settings, enabled)
    })
    .await
}

/// Asks for Touch ID / Windows Hello, then reopens the vault with the held key. Errors:
/// `quick_unlock_cancelled` (the user dismissed the prompt), `quick_unlock_failed` (the check or
/// the unlock failed: use the master password), `quick_unlock_not_ready` (no key held).
#[tauri::command]
pub async fn vault_quick_unlock(
    app: AppHandle,
    settings: State<'_, Arc<SettingsStore>>,
) -> Result<(), AppError> {
    let settings = Arc::clone(&settings);
    let owner = quick_unlock::owner_handle(&app);
    blocking("quick unlock", move || {
        quick_unlock_flow(&app.state::<AppState>(), &settings, owner)
    })
    .await
}

/// One evaluation of the inactivity timer. Never blocks: when an operation holds the vault
/// (an unlock in progress, for example) that operation is itself activity, so skipping is
/// correct.
/// Returns whether the vault was locked by this tick.
pub fn auto_lock_tick(state: &AppState) -> bool {
    match state.vault.try_lock() {
        Ok(mut vault) => vault.tick(),
        Err(std::sync::TryLockError::WouldBlock) => false,
        // Fail closed: a poisoned vault is locked now, not left open.
        Err(std::sync::TryLockError::Poisoned(poison)) => {
            let mut vault = poison.into_inner();
            tracing::error!("vault state was poisoned; locking the vault");
            let was_unlocked = vault.is_unlocked();
            if let Err(e) = vault.lock() {
                tracing::error!(error = %e, "could not lock a poisoned vault");
            }
            drop(vault);
            state.vault.clear_poison();
            was_unlocked
        }
    }
}

/// Starts the background task that locks the vault after the inactivity period.
pub fn spawn_auto_lock(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(AUTO_LOCK_TICK);
        loop {
            interval.tick().await;
            if auto_lock_tick(&app.state::<AppState>()) {
                emit_locked(&app);
            }
        }
    });
}

/// Locks the vault immediately and drops the quick unlock key (application exit).
pub fn lock_on_exit(state: &AppState) {
    // The key is dropped with the vault mutex held, so a password unlock that is racing the
    // exit cannot capture a new key after the clear.
    match state.vault.lock() {
        Ok(mut vault) => {
            state.quick_unlock.clear();
            if let Err(err) = vault.lock() {
                tracing::warn!(error = %err, "could not lock the vault on exit");
            }
        }
        Err(poison) => {
            let mut vault = poison.into_inner();
            state.quick_unlock.clear();
            if let Err(err) = vault.lock() {
                tracing::warn!(error = %err, "could not lock the vault on exit");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use portolano_core_model::{Connection, ConnectionOptions, CredentialRef, Node};
    use portolano_vault::KdfParams;

    use super::*;

    fn state_in_scratch() -> (tempfile::TempDir, AppState) {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp3");
        std::fs::create_dir_all(&base).expect("create scratch base");
        let dir = tempfile::Builder::new()
            .prefix("vault-cmd-")
            .tempdir_in(base)
            .expect("tempdir");
        let mut state = AppState::new(dir.path().to_path_buf());
        let tiny = KdfParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        };
        let path: PathBuf = state.data_dir.join("vault.bin");
        state.vault = std::sync::Arc::new(std::sync::Mutex::new(
            Vault::with_kdf_params(path, tiny).expect("vault"),
        ));
        (dir, state)
    }

    #[test]
    fn tick_locks_an_idle_vault_and_skips_a_busy_one() {
        let (_dir, state) = state_in_scratch();
        {
            let mut v = state.vault.lock().expect("lock");
            v.create(SecretString::from("fake-master")).expect("create");
            v.set_auto_lock_timeout(Duration::from_nanos(1));
        }
        std::thread::sleep(Duration::from_millis(5));

        // While another operation holds the vault, the tick does nothing and does not block.
        {
            let _busy = state.vault.lock().expect("lock");
            assert!(!auto_lock_tick(&state), "a busy vault is not locked");
        }
        assert!(state.vault.lock().expect("lock").is_unlocked());

        assert!(auto_lock_tick(&state), "the tick reports that it locked");
        assert!(!state.vault.lock().expect("lock").is_unlocked());
        assert!(!auto_lock_tick(&state), "already locked: nothing to report");
    }

    fn settings_in(dir: &tempfile::TempDir) -> SettingsStore {
        SettingsStore::load(dir.path())
    }

    #[test]
    fn setting_the_auto_lock_time_applies_it_at_once_and_saves_it() {
        let (dir, state) = state_in_scratch();
        let settings = settings_in(&dir);
        set_auto_lock(&state, &settings, 30).expect("set");
        assert_eq!(
            state.vault.lock().expect("lock").auto_lock_timeout(),
            Duration::from_secs(30 * 60)
        );
        assert_eq!(settings.get().expect("get").auto_lock_minutes, 30);
        set_auto_lock(&state, &settings, 0).expect("never");
        assert_eq!(
            state.vault.lock().expect("lock").auto_lock_timeout(),
            Duration::ZERO
        );
        assert_eq!(get_auto_lock(&settings).expect("get"), 0);
        assert_eq!(
            SettingsStore::load(dir.path())
                .get()
                .expect("get")
                .auto_lock_minutes,
            0
        );
    }

    #[test]
    fn a_failed_save_leaves_the_vault_timeout_unchanged() {
        let (dir, state) = state_in_scratch();
        let settings = settings_in(&dir);
        // A directory where the file should be makes the save fail.
        std::fs::create_dir_all(crate::settings::settings_path(dir.path())).expect("block");
        set_auto_lock(&state, &settings, 5).expect_err("save fails");
        assert_eq!(
            state.vault.lock().expect("lock").auto_lock_timeout(),
            portolano_vault::DEFAULT_AUTO_LOCK
        );
    }

    #[test]
    fn an_unsupported_auto_lock_time_is_refused_and_changes_nothing() {
        let (dir, state) = state_in_scratch();
        let settings = settings_in(&dir);
        let err = set_auto_lock(&state, &settings, 7).expect_err("7 is refused");
        assert_eq!(err.code, crate::error::ErrorCode::InvalidInput);
        assert_eq!(get_auto_lock(&settings).expect("get"), 15);
        assert_eq!(
            state.vault.lock().expect("lock").auto_lock_timeout(),
            portolano_vault::DEFAULT_AUTO_LOCK
        );
    }

    #[test]
    fn the_saved_auto_lock_time_reaches_the_vault_before_the_first_tick() {
        let (dir, state) = state_in_scratch();
        SettingsStore::load(dir.path())
            .update(|s| s.auto_lock_minutes = 5)
            .expect("save");
        let settings = settings_in(&dir);
        apply_saved_auto_lock(&state, &settings).expect("apply");
        assert_eq!(
            state.vault.lock().expect("lock").auto_lock_timeout(),
            Duration::from_secs(300)
        );
    }

    fn connection_with(credential: CredentialRef) -> Node {
        Node::Connection(Connection {
            sort_index: None,
            icon: None,
            color: None,
            id: Uuid::new_v4(),
            parent_id: None,
            name: "srv".into(),
            host: "h".into(),
            port: None,
            credential,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Vnc { view_only: false },
        })
    }

    #[test]
    fn deleting_a_credential_clears_the_references_to_it() {
        let (_dir, state) = state_in_scratch();
        let credential_id = {
            let mut v = state.vault.lock().expect("lock");
            v.create(SecretString::from("fake-master")).expect("create");
            v.upsert(portolano_vault::CredentialInput {
                id: None,
                name: "c".into(),
                username: "u".into(),
                domain: None,
                secret: portolano_vault::SecretInput::Password {
                    password: SecretString::from("fake-pw"),
                },
            })
            .expect("upsert")
            .id
        };
        let (pointing, other) = {
            let mut store = state.store.lock().expect("store");
            let a = store
                .upsert_node(connection_with(CredentialRef::Id { id: credential_id }))
                .expect("a");
            let b = store
                .upsert_node(connection_with(CredentialRef::Id { id: Uuid::new_v4() }))
                .expect("b");
            (a.id(), b.id())
        };

        state
            .vault
            .lock()
            .expect("lock")
            .delete(credential_id)
            .expect("delete");
        clear_references(&state, credential_id).expect("clear");

        let store = state.store.lock().expect("store");
        let credential_of = |id: Uuid| match store.document().node(id) {
            Some(Node::Connection(c)) => c.credential,
            _ => panic!("missing connection"),
        };
        assert_eq!(credential_of(pointing), CredentialRef::None);
        assert!(matches!(credential_of(other), CredentialRef::Id { .. }));
    }

    #[test]
    fn a_document_failure_is_reported_and_names_the_deleted_credential() {
        let (dir, state) = state_in_scratch();
        std::fs::write(
            portolano_core_model::connections_path(dir.path()),
            br#"{"schemaVersion":99,"nodes":[]}"#,
        )
        .expect("write");
        let err = clear_references(&state, Uuid::new_v4()).expect_err("unreadable document");
        assert!(err.message.starts_with("The credential was deleted"));
    }

    /// Poisons the vault mutex the way a panic inside a vault operation would.
    fn poison(state: &AppState) {
        let vault = state.vault.clone();
        let joined = std::thread::spawn(move || {
            let _guard = vault.lock();
            panic!("simulated panic inside a vault operation");
        })
        .join();
        assert!(joined.is_err());
        assert!(state.vault.is_poisoned());
    }

    fn unlocked_state() -> (tempfile::TempDir, AppState) {
        let (dir, state) = state_in_scratch();
        state
            .vault
            .lock()
            .expect("lock")
            .create(SecretString::from("fake-master"))
            .expect("create");
        (dir, state)
    }

    #[test]
    fn a_poisoned_vault_is_locked_by_the_auto_lock_tick() {
        let (_dir, state) = unlocked_state();
        poison(&state);
        assert!(auto_lock_tick(&state), "the tick must lock and say so");
        assert!(!state.vault.is_poisoned(), "the poison is cleared");
        assert!(!state.vault.lock().expect("lock").is_unlocked());
    }

    #[test]
    fn a_poisoned_vault_can_still_be_locked_and_is_locked_by_any_command() {
        let (_dir, state) = unlocked_state();
        poison(&state);
        // What `vault_lock` and every other command go through.
        let guard = lock_vault(&state).expect("a poisoned vault is recovered, not an error");
        assert!(!guard.is_unlocked(), "recovery locks the vault");
        drop(guard);
        assert!(!state.vault.is_poisoned());
        // The user unlocks again from the file, as after any lock.
        let mut vault = state.vault.lock().expect("lock");
        vault
            .unlock(SecretString::from("fake-master"))
            .expect("unlock again");
        assert!(vault.is_unlocked());
    }

    #[test]
    fn lock_on_exit_locks() {
        let (_dir, state) = state_in_scratch();
        state
            .vault
            .lock()
            .expect("lock")
            .create(SecretString::from("fake-master"))
            .expect("create");
        lock_on_exit(&state);
        assert!(!state.vault.lock().expect("lock").is_unlocked());
    }

    /// A locked vault with quick unlock switched on and a fake biometric check.
    fn quick_state() -> (
        tempfile::TempDir,
        AppState,
        SettingsStore,
        Arc<crate::quick_unlock::testing::FakeVerifier>,
    ) {
        let (dir, mut state) = state_in_scratch();
        let fake = crate::quick_unlock::testing::FakeVerifier::ok();
        state.quick_unlock = crate::quick_unlock::QuickUnlock::new(fake.clone());
        let settings = settings_in(&dir);
        state
            .vault
            .lock()
            .expect("lock")
            .create(SecretString::from("fake-master"))
            .expect("create");
        lock_vault(&state).expect("vault").lock().expect("lock");
        set_quick_unlock(&state, &settings, true).expect("turn on");
        (dir, state, settings, fake)
    }

    fn password_unlock(state: &AppState, settings: &SettingsStore, password: &str) {
        let mut vault = lock_vault(state).expect("vault");
        unlock_with_password(state, &mut vault, settings, SecretString::from(password))
            .expect("unlock");
    }

    fn info(state: &AppState, settings: &SettingsStore) -> QuickUnlockInfo {
        quick_unlock_info(state, settings).expect("info")
    }

    #[test]
    fn quick_unlock_becomes_ready_after_a_password_unlock_and_survives_lock() {
        let (_dir, state, settings, fake) = quick_state();
        assert_eq!(
            info(&state, &settings),
            QuickUnlockInfo {
                available: true,
                enabled: true,
                ready: false
            },
            "turning it on holds no key yet"
        );
        password_unlock(&state, &settings, "fake-master");
        assert!(info(&state, &settings).ready);

        lock_vault(&state).expect("vault").lock().expect("lock");
        assert!(info(&state, &settings).ready, "Lock keeps it ready");
        quick_unlock_flow(&state, &settings, None).expect("quick unlock");
        assert!(lock_vault(&state).expect("vault").is_unlocked());
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn the_biometric_prompt_never_runs_with_the_vault_mutex_held() {
        let (_dir, state, settings, fake) = quick_state();
        password_unlock(&state, &settings, "fake-master");
        lock_vault(&state).expect("vault").lock().expect("lock");
        let vault = state.vault.clone();
        let free = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = free.clone();
        *fake.during.lock().expect("hook") = Some(Box::new(move || {
            seen.store(
                vault.try_lock().is_ok(),
                std::sync::atomic::Ordering::SeqCst,
            );
        }));
        quick_unlock_flow(&state, &settings, None).expect("quick unlock");
        assert!(
            free.load(std::sync::atomic::Ordering::SeqCst),
            "the vault mutex must be free while the prompt is open"
        );
    }

    #[test]
    fn with_the_option_off_a_password_unlock_captures_nothing() {
        let (_dir, state, settings, _) = quick_state();
        set_quick_unlock(&state, &settings, false).expect("turn off");
        password_unlock(&state, &settings, "fake-master");
        assert_eq!(
            info(&state, &settings),
            QuickUnlockInfo {
                available: true,
                enabled: false,
                ready: false
            }
        );
        lock_vault(&state).expect("vault").lock().expect("lock");
        let err = quick_unlock_flow(&state, &settings, None).expect_err("off");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);
    }

    #[test]
    fn turning_the_option_off_drops_the_key_and_it_needs_a_password_unlock_to_return() {
        let (_dir, state, settings, _) = quick_state();
        password_unlock(&state, &settings, "fake-master");
        lock_vault(&state).expect("vault").lock().expect("lock");
        set_quick_unlock(&state, &settings, false).expect("turn off");
        assert!(
            !SettingsStore::load(_dir.path())
                .get()
                .expect("get")
                .quick_unlock
        );
        assert!(!info(&state, &settings).ready);

        // Turned on again: still no key until the next unlock with the password.
        set_quick_unlock(&state, &settings, true).expect("turn on");
        assert!(!info(&state, &settings).ready);
        let err = quick_unlock_flow(&state, &settings, None).expect_err("no key");
        assert_eq!(err.code, ErrorCode::QuickUnlockNotReady);
        password_unlock(&state, &settings, "fake-master");
        assert!(info(&state, &settings).ready);
    }

    #[test]
    fn the_option_cannot_be_turned_on_without_a_biometric_method() {
        let (_dir, state, settings, fake) = quick_state();
        set_quick_unlock(&state, &settings, false).expect("turn off");
        fake.available
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let err = set_quick_unlock(&state, &settings, true).expect_err("unavailable");
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert!(!settings.get().expect("get").quick_unlock);
        assert!(!info(&state, &settings).available);
        // Turning it off never needs the hardware.
        set_quick_unlock(&state, &settings, false).expect("off");
    }

    #[test]
    fn a_failed_save_of_the_option_is_reported() {
        let (dir, state, settings, _) = quick_state();
        password_unlock(&state, &settings, "fake-master");
        std::fs::remove_file(crate::settings::settings_path(dir.path())).expect("remove");
        std::fs::create_dir_all(crate::settings::settings_path(dir.path())).expect("block");
        set_quick_unlock(&state, &settings, false).expect_err("save fails");
        // The key was dropped first: failing closed.
        assert!(!state.quick_unlock.ready(true));
    }

    #[test]
    fn a_cancelled_prompt_and_a_failed_unlock_report_their_own_errors() {
        let (_dir, state, settings, fake) = quick_state();
        password_unlock(&state, &settings, "fake-master");
        lock_vault(&state).expect("vault").lock().expect("lock");

        fake.set(Err(crate::quick_unlock::VerifyError::Cancelled));
        let err = quick_unlock_flow(&state, &settings, None).expect_err("cancelled");
        assert_eq!(err.code, ErrorCode::QuickUnlockCancelled);
        assert!(info(&state, &settings).ready);
        assert!(!lock_vault(&state).expect("vault").is_unlocked());

        // The password changes without going through the commands: the held key is stale.
        {
            let mut vault = lock_vault(&state).expect("vault");
            vault
                .unlock(SecretString::from("fake-master"))
                .expect("unlock");
            vault
                .change_password(
                    SecretString::from("fake-master"),
                    SecretString::from("fake-new"),
                )
                .expect("change");
            vault.lock().expect("lock");
        }
        fake.set(Ok(()));
        let err = quick_unlock_flow(&state, &settings, None).expect_err("stale key");
        assert_eq!(err.code, ErrorCode::QuickUnlockFailed);
        assert!(
            !info(&state, &settings).ready,
            "ready = false after a failure"
        );
        assert!(!lock_vault(&state).expect("vault").is_unlocked());
    }

    #[test]
    fn an_unreadable_setting_never_blocks_the_password_unlock() {
        let (_dir, state, settings, _) = quick_state();
        password_unlock(&state, &settings, "fake-master");
        assert!(state.quick_unlock.ready(true));
        lock_vault(&state).expect("vault").lock().expect("lock");

        settings.poison_for_test();
        assert!(settings.get().is_err());
        {
            let mut vault = lock_vault(&state).expect("vault");
            unlock_with_password(
                &state,
                &mut vault,
                &settings,
                SecretString::from("fake-master"),
            )
            .expect("the master password still unlocks");
            assert!(vault.is_unlocked());
        }
        assert!(
            !state.quick_unlock.ready(true),
            "treated as off: key dropped"
        );
    }

    #[test]
    fn exit_drops_the_quick_unlock_key() {
        let (_dir, state, settings, _) = quick_state();
        password_unlock(&state, &settings, "fake-master");
        assert!(info(&state, &settings).ready);
        lock_on_exit(&state);
        assert!(!info(&state, &settings).ready);
    }
}
