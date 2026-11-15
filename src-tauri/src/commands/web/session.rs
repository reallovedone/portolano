//! The Tauri-touching layer of a web session: one child webview per tab. Everything that can
//! be decided without Tauri (geometry, validation, identifiers) lives in the sibling modules
//! and is unit-tested there; this file only calls the Tauri API.
//!
//! API shape: `Window::add_child` with a `WebviewBuilder`,
//! `set_bounds`, `show`/`hide`, `navigate`, `reload`, `close`; `data_store_identifier` for
//! isolation on macOS.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use portolano_core_model::web::{Bounds, WebNavAction};
use serde::Serialize;
use tauri::webview::{
    DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse,
    WebviewBuilder,
};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Url,
    WebviewUrl, Window, Wry,
};
use uuid::Uuid;

use super::geometry::{self, PlacedRect};
use super::identity;
use crate::error::AppError;
use crate::state::{SessionId, WebSession};

/// Label of the main UI window and webview.
const MAIN_LABEL: &str = "main";

/// Event the UI listens to for the address bar: the child page started or finished a load.
pub const URL_EVENT: &str = "web-url-changed";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UrlEvent {
    session_id: SessionId,
    url: String,
    /// "started" or "finished".
    phase: &'static str,
}

/// Connections whose isolated store is used by an open session (one entry per session).
/// Cleanup of orphaned stores must never touch these.
static STORES_IN_USE: Mutex<Vec<Uuid>> = Mutex::new(Vec::new());

pub fn stores_in_use() -> HashSet<Uuid> {
    STORES_IN_USE
        .lock()
        .map(|v| v.iter().copied().collect())
        .unwrap_or_default()
}

/// Releases the "in use" mark of an isolated store when the session is dropped.
struct StoreGuard(Uuid);

impl StoreGuard {
    fn acquire(connection_id: Uuid) -> Self {
        if let Ok(mut v) = STORES_IN_USE.lock() {
            v.push(connection_id);
        }
        Self(connection_id)
    }
}

impl Drop for StoreGuard {
    fn drop(&mut self) {
        if let Ok(mut v) = STORES_IN_USE.lock() {
            if let Some(pos) = v.iter().position(|id| *id == self.0) {
                v.swap_remove(pos);
            }
        }
    }
}

fn tauri_err(what: &str, e: tauri::Error) -> AppError {
    AppError::internal(format!("{what} failed: {e}"))
}

#[cfg(windows)]
fn io_err(what: &str, e: std::io::Error) -> AppError {
    AppError::new(
        portolano_core_model::ErrorCode::Io,
        format!("{what} failed: {e}"),
    )
}

fn main_window(app: &AppHandle) -> Result<Window, AppError> {
    app.get_window(MAIN_LABEL)
        .ok_or_else(|| AppError::internal("main window not found"))
}

/// Logical height of the window's inner area as Tauri reports it right now.
fn window_inner_logical_height(window: &Window) -> Result<f64, AppError> {
    let scale = window
        .scale_factor()
        .map_err(|e| tauri_err("scale_factor", e))?;
    let inner = window
        .inner_size()
        .map_err(|e| tauri_err("inner_size", e))?;
    Ok(geometry::logical_height(inner.height, scale))
}

/// Converts the UI bounds to window coordinates using live window values.
fn placed(
    window: &Window,
    bounds: Bounds,
    ui_viewport_height: Option<f64>,
) -> Result<PlacedRect, AppError> {
    let offset = geometry::chrome_offset(window_inner_logical_height(window)?, ui_viewport_height);
    geometry::place(bounds, offset)
}

fn rect_of(p: PlacedRect) -> Rect {
    Rect {
        position: Position::Logical(LogicalPosition::new(f64::from(p.x), f64::from(p.y))),
        size: Size::Logical(LogicalSize::new(f64::from(p.width), f64::from(p.height))),
    }
}

pub struct WebviewSession {
    app: AppHandle,
    webview: tauri::Webview<Wry>,
    /// Visibility as last requested; Tauri has no getter on `Webview`.
    visible: Mutex<bool>,
    _store: Option<StoreGuard>,
}

/// What `web_open` needs to know about the connection.
pub struct OpenSpec {
    pub connection_id: Uuid,
    pub url: Url,
    pub isolated: bool,
}

#[cfg(target_os = "macos")]
fn isolate(
    builder: WebviewBuilder<Wry>,
    spec: &OpenSpec,
    _data_dir: &Path,
) -> Result<WebviewBuilder<Wry>, AppError> {
    Ok(builder.data_store_identifier(identity::store_identifier(spec.connection_id)))
}

#[cfg(windows)]
fn isolate(
    builder: WebviewBuilder<Wry>,
    spec: &OpenSpec,
    data_dir: &Path,
) -> Result<WebviewBuilder<Wry>, AppError> {
    let dir = identity::profile_dir(data_dir, spec.connection_id);
    std::fs::create_dir_all(&dir).map_err(|e| io_err("creating the web profile folder", e))?;
    Ok(builder.data_directory(dir))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn isolate(
    _builder: WebviewBuilder<Wry>,
    _spec: &OpenSpec,
    _data_dir: &Path,
) -> Result<WebviewBuilder<Wry>, AppError> {
    Err(AppError::unsupported(
        "isolated web profiles on this platform",
    ))
}

/// Creates the child webview, hidden, at the converted bounds. The returned cell must be set
/// with the session id once the session is registered (page-load events carry it).
pub fn open(
    app: &AppHandle,
    data_dir: &Path,
    spec: OpenSpec,
    bounds: Bounds,
    ui_viewport_height: Option<f64>,
) -> Result<(Arc<WebviewSession>, Arc<OnceLock<SessionId>>), AppError> {
    let window = main_window(app)?;
    let rect = placed(&window, bounds, ui_viewport_height)?;
    let session_id: Arc<OnceLock<SessionId>> = Arc::new(OnceLock::new());

    let event_id = session_id.clone();
    let mut builder = WebviewBuilder::new(
        identity::new_label(),
        WebviewUrl::External(spec.url.clone()),
    )
    // Only http(s) (and about:blank) may load: no file:, tauri: or custom schemes.
    .on_navigation(identity::navigation_allowed)
    // Popups and window.open are blocked.
    .on_new_window(|_url, _features| NewWindowResponse::Deny)
    // Downloads are out of scope: nothing is written to disk.
    .on_download(|_webview, event| !matches!(event, DownloadEvent::Requested { .. }))
    // Camera, microphone, location and the like are never granted to a web page: without a
    // handler the WebKit default for media capture is to grant it (the OS still asks).
    .on_permission_request(|_webview, kind| permission_response(kind))
    .on_page_load(move |webview, payload| {
        let Some(id) = event_id.get() else { return };
        let phase = match payload.event() {
            PageLoadEvent::Started => "started",
            PageLoadEvent::Finished => "finished",
        };
        let event = UrlEvent {
            session_id: *id,
            url: payload.url().to_string(),
            phase,
        };
        if let Err(e) = webview.app_handle().emit_to(MAIN_LABEL, URL_EVENT, event) {
            tracing::debug!(error = %e, "web url event not delivered");
        }
    });

    let store = if spec.isolated {
        builder = isolate(builder, &spec, data_dir)?;
        Some(StoreGuard::acquire(spec.connection_id))
    } else {
        None
    };

    let webview = window
        .add_child(
            builder,
            LogicalPosition::new(f64::from(rect.x), f64::from(rect.y)),
            LogicalSize::new(f64::from(rect.width), f64::from(rect.height)),
        )
        .map_err(|e| tauri_err("add_child", e))?;
    // Hidden until the UI's first sync says the tab is on screen: no flash at a stale place.
    webview.hide().map_err(|e| tauri_err("hide", e))?;

    Ok((
        Arc::new(WebviewSession {
            app: app.clone(),
            webview,
            visible: Mutex::new(false),
            _store: store,
        }),
        session_id,
    ))
}

/// Every permission a page asks for (camera, microphone, geolocation, notifications...) is
/// denied.
fn permission_response(_kind: PermissionKind) -> PermissionResponse {
    PermissionResponse::Deny
}

impl WebSession for WebviewSession {
    fn set_bounds(
        &self,
        bounds: Bounds,
        ui_viewport_height: Option<f64>,
        visible: bool,
    ) -> Result<(), AppError> {
        let window = main_window(&self.app)?;
        let rect = placed(&window, bounds, ui_viewport_height)?;
        self.webview
            .set_bounds(rect_of(rect))
            .map_err(|e| tauri_err("set_bounds", e))?;
        let mut current = self
            .visible
            .lock()
            .map_err(|_| AppError::internal("web session state is poisoned"))?;
        if visible != *current {
            if visible {
                self.webview.show().map_err(|e| tauri_err("show", e))?;
            } else {
                self.webview.hide().map_err(|e| tauri_err("hide", e))?;
            }
            *current = visible;
        }
        Ok(())
    }

    fn navigate(&self, action: WebNavAction) -> Result<(), AppError> {
        match action {
            WebNavAction::Back => self
                .webview
                .eval("history.back()")
                .map_err(|e| tauri_err("back", e)),
            WebNavAction::Forward => self
                .webview
                .eval("history.forward()")
                .map_err(|e| tauri_err("forward", e)),
            WebNavAction::Reload => self.webview.reload().map_err(|e| tauri_err("reload", e)),
            WebNavAction::Url { url } => {
                let target = identity::validate_url(&url)?;
                self.webview
                    .navigate(target)
                    .map_err(|e| tauri_err("navigate", e))
            }
        }
    }

    fn close(&self) {
        if let Err(e) = self.webview.close() {
            tracing::warn!(error = %e, "closing a web session webview failed");
        }
    }
}

/// Isolated stores that exist (macOS: known to WebKit for this app; Windows: profile folders).
#[cfg(target_os = "macos")]
pub async fn existing_stores(app: &AppHandle, _data_dir: &Path) -> Result<Vec<Uuid>, AppError> {
    let ids = app
        .fetch_data_store_identifiers()
        .await
        .map_err(|e| tauri_err("fetch_data_store_identifiers", e))?;
    Ok(ids
        .into_iter()
        .map(identity::connection_of_identifier)
        .collect())
}

#[cfg(windows)]
pub async fn existing_stores(_app: &AppHandle, data_dir: &Path) -> Result<Vec<Uuid>, AppError> {
    let root = data_dir.join(identity::PROFILES_DIR);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err("listing the web profiles", e)),
    };
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|e| Uuid::parse_str(&e.file_name().to_string_lossy()).ok())
        .collect())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub async fn existing_stores(_app: &AppHandle, _data_dir: &Path) -> Result<Vec<Uuid>, AppError> {
    Ok(Vec::new())
}

/// Removes one isolated store (see [`existing_stores`]).
#[cfg(target_os = "macos")]
pub async fn remove_store(
    app: &AppHandle,
    _data_dir: &Path,
    connection_id: Uuid,
) -> Result<(), AppError> {
    app.remove_data_store(identity::store_identifier(connection_id))
        .await
        .map_err(|e| tauri_err("remove_data_store", e))
}

#[cfg(windows)]
pub async fn remove_store(
    _app: &AppHandle,
    data_dir: &Path,
    connection_id: Uuid,
) -> Result<(), AppError> {
    std::fs::remove_dir_all(identity::profile_dir(data_dir, connection_id))
        .map_err(|e| io_err("removing the web profile folder", e))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub async fn remove_store(
    _app: &AppHandle,
    _data_dir: &Path,
    _connection_id: Uuid,
) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_permissions_are_denied() {
        for kind in [
            PermissionKind::Microphone,
            PermissionKind::Camera,
            PermissionKind::Geolocation,
            PermissionKind::Notifications,
        ] {
            assert!(matches!(
                permission_response(kind),
                PermissionResponse::Deny
            ));
        }
    }
}
