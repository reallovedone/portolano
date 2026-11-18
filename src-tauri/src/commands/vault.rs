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
use crate::error::AppError;
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
    match state.vault.lock() {
        Ok(guard) => Ok(guard),
        Err(poison) => {
            let mut guard = poison.into_inner();
            tracing::error!("vault state was poisoned; locking the vault");
            if let Err(e) = guard.lock() {
                tracing::error!(error = %e, "could not lock a poisoned vault");
                return Err(AppError::from(e));
            }
            state.vault.clear_poison();
            Ok(guard)
        }
    }
}

/// Runs `op` on the blocking pool with the vault mutex held.
async fn with_vault<T, F>(app: AppHandle, op: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&mut Vault) -> Result<T, VaultError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut vault = lock_vault(&state)?;
        op(&mut vault).map_err(AppError::from)
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

#[tauri::command]
pub async fn vault_unlock(app: AppHandle, master_password: SecretString) -> Result<(), AppError> {
    with_vault(app, move |v| v.unlock(master_password)).await
}

#[tauri::command]
pub async fn vault_change_password(
    app: AppHandle,
    current_password: SecretString,
    new_password: SecretString,
) -> Result<(), AppError> {
    with_vault(app, move |v| {
        v.change_password(current_password, new_password)
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
    with_store(state, |store| Ok(store.clear_credential_references(id)?))
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

/// Locks the vault immediately (application exit).
pub fn lock_on_exit(state: &AppState) {
    match state.vault.lock() {
        Ok(mut vault) => {
            if let Err(err) = vault.lock() {
                tracing::warn!(error = %err, "could not lock the vault on exit");
            }
        }
        Err(poison) => {
            if let Err(err) = poison.into_inner().lock() {
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
}
