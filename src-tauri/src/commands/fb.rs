//! Framebuffer commands shared by RDP and VNC (`fb_*`). Neither protocol package edits this
//! file's command bodies:
//!
//! - `fb_pointer`, `fb_key`, `fb_resize`, `fb_set_visible`, `fb_cert_decision` look the session
//!   up in the `SessionManager` and call the `FbSession` trait object. RDP implements the
//!   trait in `rdp.rs`, VNC in `vnc.rs`.
//! - `fb_open` picks the protocol from the connection's `ConnectionOptions` and calls
//!   `rdp::open` or `vnc::open`: one match arm per protocol, one line each.
//!
//! `on_event` carries raw binary frames (`FbEvent::encode`), not JSON. `ask_credentials` skips
//! the vault for this session ("enter manually"): the session prompts for its credential.

use portolano_core_model::{ConnectionOptions, ErrorCode, Node};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::commands::{rdp, vnc};
use crate::error::AppError;
use crate::state::{AppState, SessionId};

/// Framebuffer protocol of a connection (constructed once `protocol_of` reads the document).
#[allow(dead_code)]
pub enum FbProtocol {
    Rdp,
    Vnc,
}

/// Reads the connection's options from the document and picks the framebuffer protocol.
/// `not_found` for an unknown id, `invalid_input` when the node is a folder or its options are
/// not RDP/VNC.
fn protocol_of(state: &AppState, connection_id: Uuid) -> Result<FbProtocol, AppError> {
    crate::commands::model::with_document(state, |doc| match doc.node(connection_id) {
        None => Err(AppError::not_found(format!(
            "connection {connection_id} not found"
        ))),
        Some(Node::Folder(_)) => Err(AppError::invalid_input(format!(
            "{connection_id} is a folder, not a connection"
        ))),
        Some(Node::Connection(c)) => match c.options {
            ConnectionOptions::Rdp { .. } => Ok(FbProtocol::Rdp),
            ConnectionOptions::Vnc { .. } => Ok(FbProtocol::Vnc),
            _ => Err(AppError::invalid_input(format!(
                "connection {connection_id} is not an RDP or VNC connection"
            ))),
        },
    })
}

#[tauri::command]
pub async fn fb_open(
    state: State<'_, AppState>,
    connection_id: Uuid,
    width: u16,
    height: u16,
    on_event: Channel<InvokeResponseBody>,
    ask_credentials: Option<bool>,
) -> Result<SessionId, AppError> {
    match protocol_of(&state, connection_id)? {
        FbProtocol::Rdp => {
            let ask = ask_credentials.unwrap_or(false);
            rdp::open(&state, connection_id, width, height, on_event, ask).await
        }
        FbProtocol::Vnc => {
            let ask = ask_credentials.unwrap_or(false);
            vnc::open(&state, connection_id, width, height, on_event, ask).await
        }
    }
}

#[tauri::command]
pub async fn fb_pointer(
    state: State<'_, AppState>,
    session_id: SessionId,
    x: u16,
    y: u16,
    buttons: u8,
    wheel: i16,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.pointer(x, y, buttons, wheel)
}

#[tauri::command]
pub async fn fb_key(
    state: State<'_, AppState>,
    session_id: SessionId,
    code: String,
    down: bool,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.key(&code, down)
}

#[tauri::command]
pub async fn fb_resize(
    state: State<'_, AppState>,
    session_id: SessionId,
    width: u16,
    height: u16,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.resize(width, height)
}

#[tauri::command]
pub async fn fb_set_visible(
    state: State<'_, AppState>,
    session_id: SessionId,
    visible: bool,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.set_visible(visible)
}

#[tauri::command]
pub async fn fb_cert_decision(
    state: State<'_, AppState>,
    session_id: SessionId,
    accept: bool,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.cert_decision(accept)
}

/// Answer to an `UnencryptedPrompt` (VNC): `allow` agrees to send the password without
/// encryption for this host; `false` closes the session.
#[tauri::command]
pub async fn fb_unencrypted_decision(
    state: State<'_, AppState>,
    session_id: SessionId,
    allow: bool,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.unencrypted_decision(allow)
}

/// Removes the stored certificate of the session's host. The UI offers it only after a changed
/// certificate (or a refused downgrade), as a deliberate step before connecting again.
#[tauri::command]
pub async fn fb_forget_certificate(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.forget_certificate()
}

/// Saves the files on the remote clipboard (RDP with a shared clipboard). The folder comes
/// from a native dialog opened here, in the backend: the destination is always something the
/// user just picked, never a path the webview supplies. `Ok(false)`: the user dismissed the
/// dialog and nothing happened. Progress and the outcome arrive as clipboard events.
#[tauri::command]
pub async fn fb_clipboard_save(
    state: State<'_, AppState>,
    window: WebviewWindow,
    session_id: SessionId,
) -> Result<bool, AppError> {
    let session = state.sessions.fb(session_id)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_title("Save the files from the remote clipboard")
        .set_can_create_directories(true)
        .set_parent(&window)
        .pick_folder(move |picked| {
            if tx.send(picked).is_err() {
                tracing::debug!("the folder dialog was answered after the command ended");
            }
        });
    let picked = rx
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal, "the folder dialog was closed"))?;
    let Some(picked) = picked else {
        return Ok(false);
    };
    let dir = picked.into_path().map_err(|_| {
        AppError::invalid_input("the chosen location is not a folder on this computer")
    })?;
    session.clipboard_save(dir)?;
    Ok(true)
}

/// Stops the save started by `fb_clipboard_save`.
#[tauri::command]
pub async fn fb_clipboard_cancel(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), AppError> {
    state.sessions.fb(session_id)?.clipboard_cancel()
}
