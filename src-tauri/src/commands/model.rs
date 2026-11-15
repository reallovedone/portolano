//! `doc_*` commands: the connection document.
//!
//! The document is loaded lazily, on the first command that needs it, instead of in
//! `AppState::new`. Reason: `AppState::new` is infallible and I/O-free (it is built in the
//! Tauri `setup` hook and in tests), while loading can fail (newer schema, unrecoverable
//! corruption). A failed load leaves the store unloaded, so the app still starts, every `doc_*`
//! command reports the error, the next call retries (for example after the user restores a
//! file), and no mutation can ever overwrite a file that could not be read.
//!
//! Each mutation persists atomically (`DocumentStore`) before it returns.

use portolano_core_model::{Document, DocumentStore, Node};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::{poisoned, AppError};
use crate::state::AppState;

/// Runs `f` on the loaded store, loading it first when needed. A recovery from the backup is
/// logged here (once, at load time); file contents are never logged.
pub(crate) fn with_store<T>(
    state: &AppState,
    f: impl FnOnce(&mut DocumentStore) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut store = state.store.lock().map_err(poisoned)?;
    if !store.is_loaded() {
        store.ensure_loaded()?;
        if let Some(recovery) = store.recovery() {
            tracing::warn!(
                reason = %recovery.reason,
                restored_from = %recovery.restored_from.display(),
                corrupt_copy = ?recovery.corrupt_copy,
                "connections file was unusable, restored from backup"
            );
        }
    }
    f(&mut store)
}

/// Read access to the loaded document (also used by `fb.rs` to look connections up).
pub(crate) fn with_document<T>(
    state: &AppState,
    f: impl FnOnce(&Document) -> Result<T, AppError>,
) -> Result<T, AppError> {
    with_store(state, |store| f(store.document()))
}

fn get(state: &AppState) -> Result<Document, AppError> {
    with_document(state, |doc| Ok(doc.clone()))
}

fn upsert(state: &AppState, node: Node) -> Result<Node, AppError> {
    with_store(state, |store| Ok(store.upsert_node(node)?))
}

fn delete(state: &AppState, id: Uuid) -> Result<(), AppError> {
    with_store(state, |store| Ok(store.delete_node(id)?))
}

fn move_to(state: &AppState, id: Uuid, new_parent_id: Option<Uuid>) -> Result<(), AppError> {
    with_store(state, |store| Ok(store.move_node(id, new_parent_id)?))
}

#[tauri::command]
pub async fn doc_get(state: State<'_, AppState>) -> Result<Document, AppError> {
    get(&state)
}

#[tauri::command]
pub async fn doc_upsert_node(state: State<'_, AppState>, node: Node) -> Result<Node, AppError> {
    upsert(&state, node)
}

#[tauri::command]
pub async fn doc_delete_node(
    app: AppHandle,
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<(), AppError> {
    delete(&state, id)?;
    // Isolated web profiles of the deleted connection(s) are removed in the background.
    crate::commands::web::spawn_store_cleanup(app);
    Ok(())
}

#[tauri::command]
pub async fn doc_move_node(
    state: State<'_, AppState>,
    id: Uuid,
    new_parent_id: Option<Uuid>,
) -> Result<(), AppError> {
    move_to(&state, id, new_parent_id)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use portolano_core_model::{
        connections_path, Connection, ConnectionOptions, CredentialRef, ErrorCode, Folder,
    };

    use super::*;

    /// Temp directory under `<repo>/.tmp/wp2/` (git-ignored), removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp2");
            let dir = base.join(format!(
                "cmd-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if let Err(e) = fs::remove_dir_all(&self.0) {
                eprintln!("cleanup failed: {e}");
            }
        }
    }

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(id: u128, parent: Option<u128>) -> Node {
        Node::Folder(Folder {
            id: u(id),
            parent_id: parent.map(u),
            name: "folder".into(),
            credential: CredentialRef::Inherit,
        })
    }

    fn conn(id: u128, parent: Option<u128>) -> Node {
        Node::Connection(Connection {
            id: u(id),
            parent_id: parent.map(u),
            name: "conn".into(),
            host: "h".into(),
            port: None,
            credential: CredentialRef::Inherit,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Vnc { view_only: false },
        })
    }

    #[test]
    fn first_access_loads_a_missing_file_as_empty() {
        let dir = TempDir::new();
        let state = AppState::new(dir.0.clone());
        assert!(get(&state).expect("get").nodes.is_empty());
        assert!(!connections_path(&dir.0).exists());
    }

    #[test]
    fn mutations_are_persisted_before_returning() {
        let dir = TempDir::new();
        let state = AppState::new(dir.0.clone());
        upsert(&state, folder(1, None)).expect("folder");
        upsert(&state, conn(2, Some(1))).expect("conn");
        upsert(&state, folder(3, None)).expect("folder 3");
        move_to(&state, u(2), Some(u(3))).expect("move");
        delete(&state, u(1)).expect("delete");

        // A brand-new state on the same directory sees the persisted result.
        let fresh = AppState::new(dir.0.clone());
        let doc = get(&fresh).expect("get");
        let ids: Vec<_> = doc.nodes.iter().map(Node::id).collect();
        // Moving keeps the array position.
        assert_eq!(ids, vec![u(2), u(3)]);
        assert_eq!(doc, get(&state).expect("get"));
    }

    #[test]
    fn errors_map_to_stable_codes() {
        let dir = TempDir::new();
        let state = AppState::new(dir.0.clone());
        assert_eq!(
            delete(&state, u(9)).expect_err("nf").code,
            ErrorCode::NotFound
        );
        assert_eq!(
            move_to(&state, u(9), None).expect_err("nf").code,
            ErrorCode::NotFound
        );
        assert_eq!(
            upsert(&state, conn(1, Some(5))).expect_err("parent").code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn unreadable_file_is_reported_and_never_overwritten() {
        let dir = TempDir::new();
        let content = br#"{"schemaVersion":7,"nodes":[]}"#;
        fs::write(connections_path(&dir.0), content).expect("write");
        let state = AppState::new(dir.0.clone());
        assert_eq!(get(&state).expect_err("newer").code, ErrorCode::Unsupported);
        assert_eq!(
            upsert(&state, folder(1, None)).expect_err("newer").code,
            ErrorCode::Unsupported
        );
        assert_eq!(fs::read(connections_path(&dir.0)).expect("read"), content);
    }

    #[test]
    fn corrupt_file_is_recovered_from_backup_on_first_access() {
        let dir = TempDir::new();
        {
            let state = AppState::new(dir.0.clone());
            upsert(&state, folder(1, None)).expect("first");
            upsert(&state, folder(2, None)).expect("second");
        }
        fs::write(connections_path(&dir.0), b"{ truncated").expect("corrupt");
        let state = AppState::new(dir.0.clone());
        assert_eq!(get(&state).expect("recovered").nodes.len(), 1);
    }

    #[test]
    fn with_document_gives_read_access_to_the_loaded_document() {
        let dir = TempDir::new();
        let state = AppState::new(dir.0.clone());
        upsert(&state, conn(1, None)).expect("conn");
        let found = with_document(&state, |doc| Ok(doc.node(u(1)).is_some())).expect("read");
        assert!(found);
    }
}
