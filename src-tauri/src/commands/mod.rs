//! IPC command groups, one file per group (§3), plus `fb.rs` for the commands shared by RDP
//! and VNC. Every command is registered in the single `generate_handler!` list in `lib.rs`.
//!
//! All commands are `async` (a synchronous command runs on the main thread, and creating
//! webviews from one blocks on Windows). Until the owning work package lands, each returns
//! `AppError { code: "unsupported" }`.

pub mod appearance;
pub mod fb;
pub mod keyfile;
pub mod model;
pub mod prompt;
pub mod rdp;
pub mod shares;
pub mod term;
pub mod transfer;
pub mod update;
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

/// Not part of the §4.3 contract: the text on the system clipboard, for pasting into a terminal
/// with the right mouse button (the webview's own clipboard read may prompt or be refused).
/// `None` when the clipboard holds no text. The text is never logged.
#[tauri::command]
pub async fn clipboard_read_text() -> Result<Option<String>, AppError> {
    tauri::async_runtime::spawn_blocking(crate::clipboard::read_text_for_paste)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "clipboard read task failed");
            AppError::internal("clipboard read task failed")
        })?
        .map_err(|e| AppError::internal(e.0))
}

/// Not part of the §4.3 contract: while the keyboard belongs to a graphical session (Mac
/// shortcuts on), the native Edit menu leaves the menu bar so that ⌘C, ⌘V, ⌘X, ⌘A and ⌘Z reach
/// the webview and then the remote machine; `false` puts it back. Does nothing outside macOS.
#[tauri::command]
pub async fn menu_set_session_keys(app: tauri::AppHandle, captured: bool) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    crate::menu::set_session_keys(&app, captured);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&app, captured);
    }
    Ok(())
}

/// Where the generated test credentials are (tests only). `PORTOLANO_TEST_KEYS_DIR` moves the
/// folder (CI simulation); `PORTOLANO_TEST_SERVERS_DOWN` makes every integration test skip as
/// if the servers and their credentials did not exist.
#[cfg(test)]
pub(crate) fn test_env_path() -> std::path::PathBuf {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    if std::env::var_os("PORTOLANO_TEST_SERVERS_DOWN").is_some() {
        return repo.join(".tmp/wp12/servers-down/test.env");
    }
    std::env::var_os("PORTOLANO_TEST_KEYS_DIR")
        .map_or_else(|| repo.join("dev/keys"), std::path::PathBuf::from)
        .join("test.env")
}
