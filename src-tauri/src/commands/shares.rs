//! `identity_*` and `share_*` commands: the signing identity and the shared folders.
//!
//! The work is done by [`crate::shares::ShareService`]; this file only adapts it to IPC. The
//! secret key of the identity never leaves the vault except inside an encrypted backup file the
//! user asked for, and is never part of an answer.

use std::path::Path;

use ed25519_dalek::VerifyingKey;
use portolano_core_model::private_fs::write_private_atomic;
use portolano_core_model::{CredentialRef, ErrorCode, ShareLocation};
use portolano_shares::identity::{encode_public_code, fingerprint, PublicIdentity};
use portolano_vault::identity_backup;
use secrecy::SecretString;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::AppError;
use crate::shares::{SharePreview, ShareService, ShareSummary};
use crate::state::AppState;

/// An identity backup is a few hundred bytes; anything bigger is not one.
const MAX_BACKUP_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySummary {
    pub id: Uuid,
    pub name: String,
    pub fingerprint: String,
    /// What the user sends to others (public name and key only).
    pub public_code: String,
}

fn summary_of(record: &portolano_vault::IdentityRecord) -> Result<IdentitySummary, AppError> {
    let key = VerifyingKey::from_bytes(&record.public_key)
        .map_err(|_| AppError::internal("the identity key is not valid"))?;
    Ok(IdentitySummary {
        id: record.id,
        name: record.name.clone(),
        fingerprint: fingerprint(&key),
        public_code: encode_public_code(&PublicIdentity {
            name: record.name.clone(),
            key,
        }),
    })
}

fn identity_get_impl(state: &AppState) -> Result<Option<IdentitySummary>, AppError> {
    let vault = crate::state::lock_vault_mutex(&state.vault)?;
    vault.identities()?.first().map(summary_of).transpose()
}

fn already_have_one() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "This vault already has a signing identity.",
    )
}

fn clean_identity_name(name: &str) -> Result<String, AppError> {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::invalid_input(
            "The name must have between 1 and 100 characters.",
        ));
    }
    Ok(name)
}

fn identity_create_impl(state: &AppState, name: &str) -> Result<IdentitySummary, AppError> {
    let name = clean_identity_name(name)?;
    let mut vault = crate::state::lock_vault_mutex(&state.vault)?;
    if !vault.identities()?.is_empty() {
        return Err(already_have_one());
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut *secret)
        .map_err(|_| AppError::internal("no source of randomness is available"))?;
    let record = vault.add_identity(Uuid::new_v4(), &name, &secret)?;
    summary_of(&record)
}

fn identity_export_impl(
    state: &AppState,
    path: &str,
    password: &SecretString,
) -> Result<(), AppError> {
    let (name, secret) = {
        let vault = crate::state::lock_vault_mutex(&state.vault)?;
        let record = vault
            .identities()?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::not_found("There is no signing identity to export."))?;
        (record.name.clone(), vault.identity_secret(record.id)?)
    };
    let bytes = identity_backup::encrypt(&name, &secret, password)?;
    drop(secret);
    write_private_atomic(Path::new(path), &bytes)
        .map_err(|_| AppError::new(ErrorCode::Io, "The backup file could not be written."))
}

fn identity_import_impl(
    state: &AppState,
    path: &str,
    password: &SecretString,
) -> Result<IdentitySummary, AppError> {
    let file = std::fs::File::open(path)
        .map_err(|_| AppError::new(ErrorCode::Io, "The backup file could not be read."))?;
    let len = file
        .metadata()
        .map_err(|_| AppError::new(ErrorCode::Io, "The backup file could not be read."))?
        .len();
    if len > MAX_BACKUP_BYTES {
        return Err(AppError::invalid_input(
            "This is not an identity backup file.",
        ));
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(file, MAX_BACKUP_BYTES), &mut bytes)
        .map_err(|_| AppError::new(ErrorCode::Io, "The backup file could not be read."))?;
    let (name, secret) = identity_backup::decrypt(&bytes, password)?;
    let mut vault = crate::state::lock_vault_mutex(&state.vault)?;
    if !vault.identities()?.is_empty() {
        return Err(already_have_one());
    }
    let record = vault.add_identity(Uuid::new_v4(), &name, &secret)?;
    summary_of(&record)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        tracing::error!(error = %e, "share task failed");
        AppError::internal("the operation failed")
    })?
}

#[tauri::command]
pub async fn identity_get(state: State<'_, AppState>) -> Result<Option<IdentitySummary>, AppError> {
    identity_get_impl(&state)
}

#[tauri::command]
pub async fn identity_create(
    app: tauri::AppHandle,
    name: String,
) -> Result<IdentitySummary, AppError> {
    use tauri::Manager;
    blocking(move || identity_create_impl(&app.state::<AppState>(), &name)).await
}

#[tauri::command]
pub async fn identity_export(
    app: tauri::AppHandle,
    path: String,
    password: String,
) -> Result<(), AppError> {
    use tauri::Manager;
    let password = SecretString::from(password);
    blocking(move || identity_export_impl(&app.state::<AppState>(), &path, &password)).await
}

#[tauri::command]
pub async fn identity_import(
    app: tauri::AppHandle,
    path: String,
    password: String,
) -> Result<IdentitySummary, AppError> {
    use tauri::Manager;
    let password = SecretString::from(password);
    blocking(move || identity_import_impl(&app.state::<AppState>(), &path, &password)).await
}

#[tauri::command]
pub async fn share_publish(
    app: tauri::AppHandle,
    folder_id: Uuid,
    path: String,
) -> Result<ShareSummary, AppError> {
    use tauri::Manager;
    blocking(move || {
        let state = app.state::<AppState>();
        state.shares.publish(folder_id, &path)
    })
    .await
}

#[tauri::command]
pub async fn share_republish(
    app: tauri::AppHandle,
    link_id: Uuid,
) -> Result<ShareSummary, AppError> {
    use tauri::Manager;
    blocking(move || {
        let state = app.state::<AppState>();
        state.shares.republish(link_id)
    })
    .await
}

#[tauri::command]
pub async fn share_preview(
    state: State<'_, AppState>,
    location: ShareLocation,
) -> Result<SharePreview, AppError> {
    let shares: std::sync::Arc<ShareService> = state.shares.clone();
    shares.preview(location).await
}

#[tauri::command]
pub async fn share_subscribe(
    app: tauri::AppHandle,
    preview_id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<ShareSummary, AppError> {
    use tauri::Manager;
    blocking(move || {
        let state = app.state::<AppState>();
        state.shares.subscribe(preview_id, parent_id)
    })
    .await
}

#[tauri::command]
pub async fn share_sync(
    state: State<'_, AppState>,
    link_id: Option<Uuid>,
) -> Result<Vec<ShareSummary>, AppError> {
    let shares = state.shares.clone();
    shares.sync(link_id).await
}

#[tauri::command]
pub async fn share_unsubscribe(app: tauri::AppHandle, link_id: Uuid) -> Result<(), AppError> {
    use tauri::Manager;
    blocking(move || {
        let state = app.state::<AppState>();
        state.shares.unsubscribe(link_id)
    })
    .await
}

/// `credential`: `Inherit` removes the local choice. `node_id` is an id of the merged tree.
#[tauri::command]
pub async fn share_set_credential(
    app: tauri::AppHandle,
    node_id: Uuid,
    credential: CredentialRef,
) -> Result<(), AppError> {
    use tauri::Manager;
    blocking(move || {
        let state = app.state::<AppState>();
        state.shares.set_credential(node_id, credential)
    })
    .await
}

#[tauri::command]
pub async fn share_confirm_targets(
    app: tauri::AppHandle,
    link_id: Uuid,
    revision: u64,
) -> Result<(), AppError> {
    use tauri::Manager;
    blocking(move || {
        let state = app.state::<AppState>();
        state.shares.confirm_targets(link_id, revision)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shares::testkit;

    #[test]
    fn identity_create_export_import_round_trip() {
        let dir = testkit::scratch();
        let state = testkit::state_with_vault(dir.path(), false);
        assert!(identity_get_impl(&state).expect("get").is_none());
        let made = identity_create_impl(&state, "  Ada  ").expect("create");
        assert_eq!(made.name, "Ada");
        assert!(made.public_code.starts_with("portolano-id:1:"));
        assert_eq!(
            identity_create_impl(&state, "Other")
                .expect_err("one only")
                .code,
            ErrorCode::Conflict
        );
        let backup = dir.path().join("ada.backup");
        let password = SecretString::from("a backup password".to_owned());
        identity_export_impl(&state, &backup.to_string_lossy(), &password).expect("export");

        let other_dir = testkit::scratch();
        let other = testkit::state_with_vault(other_dir.path(), false);
        let wrong = SecretString::from("nope".to_owned());
        assert!(identity_import_impl(&other, &backup.to_string_lossy(), &wrong).is_err());
        let imported =
            identity_import_impl(&other, &backup.to_string_lossy(), &password).expect("import");
        assert_eq!(imported.fingerprint, made.fingerprint);
        assert_eq!(
            identity_import_impl(&other, &backup.to_string_lossy(), &password)
                .expect_err("one only")
                .code,
            ErrorCode::Conflict
        );
    }

    #[test]
    fn identity_commands_need_the_unlocked_vault() {
        let dir = testkit::scratch();
        let state = testkit::state_with_vault(dir.path(), true);
        state.vault.lock().expect("vault").lock().expect("lock");
        assert_eq!(
            identity_create_impl(&state, "Ada")
                .expect_err("locked")
                .code,
            ErrorCode::VaultLocked
        );
    }
}
