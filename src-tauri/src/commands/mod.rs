//! IPC command groups, one file per group (§3), plus `fb.rs` for the commands shared by RDP
//! and VNC. Every command is registered in the single `generate_handler!` list in `lib.rs`.
//!
//! All commands are `async` (a synchronous command runs on the main thread, and creating
//! webviews from one blocks on Windows). Until the owning work package lands, each returns
//! `AppError { code: "unsupported" }`.

pub mod fb;
pub mod model;
pub mod prompt;
pub mod rdp;
pub mod term;
pub mod transfer;
pub mod vault;
pub mod vnc;
pub mod web;

use portolano_core_model::prompt::CredentialAnswer;
use tauri::State;

use crate::error::AppError;
use crate::state::{AppState, SessionId};

/// Closes a session of any family. The UI calls this whenever a tab is closed.
#[tauri::command]
pub async fn session_close(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), AppError> {
    state.sessions.close(session_id)
}

/// Answer to a credential prompt of a terminal or framebuffer session (WP11). The secrets
/// travel UI -> backend only, are used for that session and are never logged or echoed.
#[tauri::command]
pub async fn session_credential(
    state: State<'_, AppState>,
    session_id: SessionId,
    answer: CredentialAnswer,
    save_to_vault: bool,
) -> Result<(), AppError> {
    state.sessions.credential(session_id, answer, save_to_vault)
}

/// The user dismissed the credential prompt: the session ends as `cancelled`.
#[tauri::command]
pub async fn session_credential_cancel(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), AppError> {
    state.sessions.credential_cancel(session_id)
}

/// Not part of the §4.3 contract: lets the UI report lifecycle facts (mounted, which IPC
/// results it got) to the backend log so they can be checked without looking at a screen.
/// The UI must never pass secrets here; the message is truncated.
#[tauri::command]
pub async fn ui_log(level: String, message: String) -> Result<(), AppError> {
    const MAX_CHARS: usize = 500;
    let message: String = message.chars().take(MAX_CHARS).collect();
    match level.as_str() {
        "error" => tracing::error!(target: "ui", "{message}"),
        "warn" => tracing::warn!(target: "ui", "{message}"),
        _ => tracing::info!(target: "ui", "{message}"),
    }
    Ok(())
}
