//! IPC commands of the update feature (WP13a). Not part of the §4.3 table: they are additive.
//! The updater itself lives in `crate::update`; these only translate between it and the UI.

use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::update::{
    release_notes as all_release_notes, ReleaseNotesList, UpdateCheck, UpdateEvent, UpdateService,
    UpdateSettings, WhatsNew,
};

#[tauri::command]
pub async fn update_settings(
    service: State<'_, UpdateService>,
) -> Result<UpdateSettings, AppError> {
    service.settings()
}

#[tauri::command]
pub async fn update_set_auto_check(
    service: State<'_, UpdateService>,
    enabled: bool,
) -> Result<(), AppError> {
    service.set_auto_check(enabled)
}

/// Looks for a newer version. `manual` is true for "Check for updates…": it ignores the
/// automatic-check setting and the skipped version and always answers.
#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    service: State<'_, UpdateService>,
    manual: bool,
) -> Result<UpdateCheck, AppError> {
    service.check(&app, manual).await
}

/// "Skip this version": automatic checks stop proposing it.
#[tauri::command]
pub async fn update_skip(
    service: State<'_, UpdateService>,
    version: String,
) -> Result<(), AppError> {
    service.skip(&version)
}

/// "Update now": downloads the update found by the last `update_check`, verifies it, installs it
/// and restarts the application. Progress arrives on `on_event`.
#[tauri::command]
pub async fn update_install(
    app: AppHandle,
    service: State<'_, UpdateService>,
    on_event: Channel<UpdateEvent>,
) -> Result<(), AppError> {
    service
        .install(&app, move |event| {
            if let Err(e) = on_event.send(event) {
                tracing::debug!(error = %e, "update progress not delivered");
            }
        })
        .await
}

/// The "What's new" dialog due at this start-up, if any.
#[tauri::command]
pub async fn whats_new(service: State<'_, UpdateService>) -> Result<Option<WhatsNew>, AppError> {
    service.whats_new().await
}

/// The release notes of every version, newest first (the "Release notes" window).
#[tauri::command]
pub fn release_notes() -> ReleaseNotesList {
    all_release_notes()
}

/// Opens the GitHub release page of a version in the browser.
#[tauri::command]
pub async fn update_open_release_page(
    service: State<'_, UpdateService>,
    version: String,
) -> Result<(), AppError> {
    service.open_release_page(&version)
}

/// Opens the full changelog on GitHub in the browser (the address is fixed in the backend).
#[tauri::command]
pub async fn update_open_changelog(service: State<'_, UpdateService>) -> Result<(), AppError> {
    service.open_changelog()
}
