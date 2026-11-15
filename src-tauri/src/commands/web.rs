//! Web family commands: one child webview per session, embedded in the main window.
//!
//! Commands that create or move webviews are `async` (a synchronous command runs on the main
//! thread and `add_child` blocks on Windows).
//!
//! Design notes:
//! - The UI sends its placeholder rect in viewport CSS px AND its `innerHeight`
//!   (`viewport_height`, an optional extra argument of `web_open` / `web_set_bounds`). The
//!   conversion to window coordinates happens in one place (`geometry`), from live values, on
//!   every call. The UI reports its height because the backend cannot read it: Tauri's size of
//!   the main webview is the window's inner area, which on macOS with a standard title bar
//!   is taller than the viewport WebKit lays the page out in.
//! - Child pages have no IPC: the label (`web-<uuid>`) matches no capability and the page's
//!   origin is remote while the capability is local. A test pins both.
//! - Isolation: `isolated_profile` gets a store derived from the connection id (stable across
//!   tabs and restarts). Orphaned stores are removed after a connection is deleted and once on
//!   the first `web_open` of each run.

mod geometry;
mod identity;
mod session;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use portolano_core_model::web::{Bounds, WebNavAction};
use portolano_core_model::{ConnectionOptions, Document, Node};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::commands::model::with_document;
use crate::error::AppError;
use crate::state::{AppState, SessionHandle, SessionId, WebSession};

use session::OpenSpec;

/// URL and isolation flag of a web connection.
fn spec_of(doc: &Document, connection_id: Uuid) -> Result<(String, bool), AppError> {
    match doc.node(connection_id) {
        None => Err(AppError::not_found(format!(
            "connection {connection_id} not found"
        ))),
        Some(Node::Folder(_)) => Err(AppError::invalid_input(format!(
            "{connection_id} is a folder, not a connection"
        ))),
        Some(Node::Connection(c)) => match &c.options {
            ConnectionOptions::Web {
                url,
                isolated_profile,
            } => Ok((url.clone(), *isolated_profile)),
            _ => Err(AppError::invalid_input(format!(
                "connection {connection_id} is not a web connection"
            ))),
        },
    }
}

/// Connections that want an isolated profile.
fn isolated_connections(doc: &Document) -> HashSet<Uuid> {
    doc.nodes
        .iter()
        .filter_map(|n| match n {
            Node::Connection(c) => match c.options {
                ConnectionOptions::Web {
                    isolated_profile: true,
                    ..
                } => Some(c.id),
                _ => None,
            },
            Node::Folder(_) => None,
        })
        .collect()
}

#[tauri::command]
pub async fn web_open(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: Uuid,
    bounds: Bounds,
    viewport_height: Option<f64>,
) -> Result<SessionId, AppError> {
    static SWEPT: AtomicBool = AtomicBool::new(false);

    let (raw_url, isolated) = with_document(&state, |doc| spec_of(doc, connection_id))?;
    let url = identity::validate_url(&raw_url)?;
    let spec = OpenSpec {
        connection_id,
        url,
        isolated,
    };
    let (web, id_cell) = session::open(&app, &state.data_dir, spec, bounds, viewport_height)?;
    let handle = SessionHandle::Web(web.clone());
    let id = match state.sessions.insert(handle) {
        Ok(id) => id,
        Err(e) => {
            web.close();
            return Err(e);
        }
    };
    // Set after registration: page-load events carry the id. A set can only fail if it was
    // already set, which this single call site rules out.
    if id_cell.set(id).is_err() {
        tracing::warn!("web session id was already set");
    }
    if !SWEPT.swap(true, Ordering::SeqCst) {
        spawn_store_cleanup(app);
    }
    Ok(id)
}

#[tauri::command]
pub async fn web_set_bounds(
    state: State<'_, AppState>,
    session_id: SessionId,
    bounds: Bounds,
    visible: bool,
    viewport_height: Option<f64>,
) -> Result<(), AppError> {
    state
        .sessions
        .web(session_id)?
        .set_bounds(bounds, viewport_height, visible)
}

#[tauri::command]
pub async fn web_navigate(
    state: State<'_, AppState>,
    session_id: SessionId,
    action: WebNavAction,
) -> Result<(), AppError> {
    state.sessions.web(session_id)?.navigate(action)
}

/// Removes the isolated stores of connections that no longer exist, in the background.
/// Called after a connection is deleted and once per run on the first `web_open`. Stores of
/// open sessions are skipped (a tab whose connection was just deleted is closed by the UI a
/// moment later: its store is removed by the next sweep). Failures are logged, never raised.
pub fn spawn_store_cleanup(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sweep_orphan_stores(&app).await {
            tracing::warn!(error = %e.message, "cleaning up web profiles failed");
        }
    });
}

async fn sweep_orphan_stores(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    // If the document cannot be read nothing is removed: an empty "wanted" set would wipe
    // every profile.
    let wanted = with_document(&state, |doc| Ok(isolated_connections(doc)))?;
    let found = session::existing_stores(app, &state.data_dir).await?;
    let orphans = identity::orphaned(found, &wanted, &session::stores_in_use());
    for connection_id in orphans {
        match session::remove_store(app, &state.data_dir, connection_id).await {
            Ok(()) => tracing::info!(%connection_id, "removed an orphaned web profile"),
            Err(e) => {
                tracing::warn!(%connection_id, error = %e.message, "removing a web profile failed");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use portolano_core_model::{Connection, CredentialRef, ErrorCode, Folder};

    use super::*;

    fn web_conn(n: u128, url: &str, isolated: bool) -> Node {
        Node::Connection(Connection {
            id: Uuid::from_u128(n),
            parent_id: None,
            name: format!("c{n}"),
            host: String::new(),
            port: None,
            credential: CredentialRef::None,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Web {
                url: url.into(),
                isolated_profile: isolated,
            },
        })
    }

    fn doc(nodes: Vec<Node>) -> Document {
        Document {
            nodes,
            ..Document::default()
        }
    }

    #[test]
    fn spec_of_reads_url_and_isolation() {
        let d = doc(vec![web_conn(1, "https://a.b", true)]);
        assert_eq!(
            spec_of(&d, Uuid::from_u128(1)).expect("spec"),
            ("https://a.b".to_string(), true)
        );
    }

    #[test]
    fn spec_of_rejects_unknown_folder_and_other_protocols() {
        let folder = Node::Folder(Folder {
            id: Uuid::from_u128(2),
            parent_id: None,
            name: "f".into(),
            credential: CredentialRef::None,
        });
        let ssh = serde_json::from_value::<Node>(serde_json::json!({
            "type": "connection", "id": Uuid::from_u128(3), "name": "s", "host": "h",
            "credential": {"type": "none"}, "options": {"type": "vnc"}
        }))
        .expect("node");
        let d = doc(vec![folder, ssh]);
        assert_eq!(
            spec_of(&d, Uuid::from_u128(9)).expect_err("none").code,
            ErrorCode::NotFound
        );
        assert_eq!(
            spec_of(&d, Uuid::from_u128(2)).expect_err("folder").code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            spec_of(&d, Uuid::from_u128(3)).expect_err("shell").code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn only_isolated_web_connections_want_a_store() {
        let d = doc(vec![
            web_conn(1, "https://a.b", true),
            web_conn(2, "https://a.b", false),
        ]);
        assert_eq!(
            isolated_connections(&d),
            HashSet::from([Uuid::from_u128(1)])
        );
    }

    /// `*` and `?` glob match, the subset of Tauri's capability patterns that matters here.
    fn glob_matches(pattern: &str, text: &str) -> bool {
        fn go(p: &[char], t: &[char]) -> bool {
            match p.split_first() {
                None => t.is_empty(),
                Some(('*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
                Some(('?', rest)) => !t.is_empty() && go(rest, &t[1..]),
                Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
            }
        }
        let p: Vec<char> = pattern.chars().collect();
        let t: Vec<char> = text.chars().collect();
        go(&p, &t)
    }

    #[test]
    fn glob_helper_behaves() {
        assert!(glob_matches("main", "main"));
        assert!(glob_matches("web-*", "web-123"));
        assert!(glob_matches("*", "anything"));
        assert!(!glob_matches("main", "web-1"));
        assert!(glob_matches("w?b-*", "web-1"));
    }

    /// Child pages must never get Tauri IPC. Tauri grants a capability to a webview when its
    /// label matches `webviews` OR its window label matches `windows`, and only for the
    /// origin the capability declares (local unless a `remote` block exists). So: no pattern
    /// may match a `web-<uuid>` label, and no capability (or config) may open IPC to remote
    /// origins.
    #[test]
    fn child_webview_labels_match_no_capability() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("capabilities dir") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).expect("read"))
                    .expect("capability json");
            let labels: Vec<String> = (0..20).map(|_| identity::new_label()).collect();
            for key in ["windows", "webviews"] {
                for pattern in value[key].as_array().into_iter().flatten() {
                    let pattern = pattern.as_str().expect("pattern string");
                    for label in &labels {
                        assert!(
                            !glob_matches(pattern, label),
                            "{} pattern {pattern:?} matches child label {label}",
                            path.display()
                        );
                    }
                }
            }
            assert!(
                value.get("remote").is_none(),
                "{} grants IPC to remote origins",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "no capability files found");
        let conf =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
                .expect("tauri.conf.json");
        assert!(!conf.contains("dangerousRemoteDomainIpcAccess"));
    }
}
