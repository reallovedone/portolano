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

use crate::error::AppError;
use crate::shares::{read_only, DocumentView, DELETE_CONTAINS_SHARES};
use crate::state::AppState;

/// Runs `f` on the loaded store, loading it first when needed (and mounting the cached copies
/// of the shared folders). A recovery from the backup is logged at load time; file contents
/// are never logged.
pub(crate) fn with_store<T>(
    state: &AppState,
    f: impl FnOnce(&mut DocumentStore) -> Result<T, AppError>,
) -> Result<T, AppError> {
    state.shares.with_store(f)
}

/// Read access to the document as the user sees it: the local nodes plus every mounted shared
/// folder (also used by `fb.rs`, `term.rs`, ... to look connections up).
pub(crate) fn with_document<T>(
    state: &AppState,
    f: impl FnOnce(&Document) -> Result<T, AppError>,
) -> Result<T, AppError> {
    with_store(state, |store| {
        let merged = state.shares.merged(store.document())?;
        f(&merged.document)
    })
}

/// Read access to the local document only, without any shared content (export and import).
pub(crate) fn with_local_document<T>(
    state: &AppState,
    f: impl FnOnce(&Document) -> Result<T, AppError>,
) -> Result<T, AppError> {
    with_store(state, |store| f(store.document()))
}

fn get(state: &AppState) -> Result<DocumentView, AppError> {
    with_store(state, |store| state.shares.document_view(store.document()))
}

fn upsert(state: &AppState, node: Node) -> Result<Node, AppError> {
    with_store(state, |store| {
        let remote = state.shares.remote_map(store.document())?;
        if remote.contains_key(&node.id())
            || node.parent_id().is_some_and(|p| remote.contains_key(&p))
        {
            return Err(read_only());
        }
        Ok(store.upsert_node(node)?)
    })
}

fn delete(state: &AppState, id: Uuid) -> Result<(), AppError> {
    with_store(state, |store| {
        let doc = store.document();
        if state.shares.remote_map(doc)?.contains_key(&id) {
            return Err(read_only());
        }
        if !state.shares.links_under(doc, id).is_empty() {
            return Err(AppError::new(
                portolano_core_model::ErrorCode::Conflict,
                DELETE_CONTAINS_SHARES,
            ));
        }
        Ok(store.delete_node(id)?)
    })
}

fn move_to(state: &AppState, id: Uuid, new_parent_id: Option<Uuid>) -> Result<(), AppError> {
    with_store(state, |store| {
        let remote = state.shares.remote_map(store.document())?;
        if new_parent_id.is_some_and(|p| remote.contains_key(&p)) {
            return Err(read_only());
        }
        if let Some(node) = remote.get(&id) {
            return state.shares.move_mount_root(store, *node, new_parent_id);
        }
        Ok(store.move_node(id, new_parent_id)?)
    })
}

fn reorder_node(
    state: &AppState,
    id: Uuid,
    new_parent_id: Option<Uuid>,
    before: Option<Uuid>,
    visible_order: Option<Vec<Uuid>>,
) -> Result<(), AppError> {
    with_store(state, |store| {
        let remote = state.shares.remote_map(store.document())?;
        if remote.contains_key(&id) || new_parent_id.is_some_and(|p| remote.contains_key(&p)) {
            return Err(read_only());
        }
        // Mount roots come after the local siblings: dropping next to one means "at the end".
        let before = before.filter(|b| !remote.contains_key(b));
        let visible_order = visible_order.map(|order| {
            order
                .into_iter()
                .filter(|n| !remote.contains_key(n))
                .collect::<Vec<_>>()
        });
        Ok(store.reorder(id, new_parent_id, before, visible_order.as_deref())?)
    })
}

fn sort_by_name(state: &AppState, parent_id: Option<Uuid>) -> Result<(), AppError> {
    with_store(state, |store| {
        let remote = state.shares.remote_map(store.document())?;
        if parent_id.is_some_and(|p| remote.contains_key(&p)) {
            return Err(read_only());
        }
        Ok(store.sort_by_name(parent_id)?)
    })
}

#[tauri::command]
pub async fn doc_get(state: State<'_, AppState>) -> Result<DocumentView, AppError> {
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

/// Moves a node under `new_parent_id`, right before the sibling `before` (end of its group
/// when `None`), and renumbers the group. `visible_order` is the UI's order of the group's
/// siblings, used only while the group has no index yet.
#[tauri::command]
pub async fn node_reorder(
    state: State<'_, AppState>,
    id: Uuid,
    new_parent_id: Option<Uuid>,
    before: Option<Uuid>,
    visible_order: Option<Vec<Uuid>>,
) -> Result<(), AppError> {
    reorder_node(&state, id, new_parent_id, before, visible_order)
}

/// Puts the children of `parent_id` (`None` = root) back in alphabetical order.
#[tauri::command]
pub async fn node_sort_by_name(
    state: State<'_, AppState>,
    parent_id: Option<Uuid>,
) -> Result<(), AppError> {
    sort_by_name(&state, parent_id)
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
            sort_index: None,
            icon: None,
            color: None,
            id: u(id),
            parent_id: parent.map(u),
            name: "folder".into(),
            credential: CredentialRef::Inherit,
        })
    }

    fn conn(id: u128, parent: Option<u128>) -> Node {
        Node::Connection(Connection {
            sort_index: None,
            icon: None,
            color: None,
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

    #[test]
    fn reorder_and_sort_by_name_persist_and_map_errors() {
        let dir = TempDir::new();
        let state = AppState::new(dir.0.clone());
        upsert(&state, conn(1, None)).expect("one");
        upsert(&state, conn(2, None)).expect("two");
        reorder_node(&state, u(2), None, Some(u(1)), None).expect("reorder");

        let fresh = AppState::new(dir.0.clone());
        let doc = get(&fresh).expect("get");
        assert_eq!(doc.node(u(2)).and_then(Node::sort_index), Some(0));
        assert_eq!(doc.node(u(1)).and_then(Node::sort_index), Some(1));

        assert_eq!(
            reorder_node(&state, u(9), None, None, None)
                .expect_err("nf")
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            reorder_node(&state, u(1), None, Some(u(9)), None)
                .expect_err("before")
                .code,
            ErrorCode::NotFound
        );
        sort_by_name(&state, None).expect("sort");
        let doc = get(&state).expect("get");
        assert!(doc.nodes.iter().all(|n| n.sort_index().is_none()));
        assert_eq!(
            sort_by_name(&state, Some(u(1)))
                .expect_err("not a folder")
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn unknown_icon_or_color_is_rejected_when_saving() {
        let dir = TempDir::new();
        let state = AppState::new(dir.0.clone());
        let Node::Folder(mut f) = folder(1, None) else {
            panic!("folder");
        };
        f.icon = Some("not-an-icon".into());
        assert_eq!(
            upsert(&state, Node::Folder(f.clone()))
                .expect_err("icon")
                .code,
            ErrorCode::InvalidInput
        );
        f.icon = Some("server".into());
        f.color = Some("blue".into());
        upsert(&state, Node::Folder(f)).expect("valid names");
    }

    // ---- shared folders ----

    use crate::shares::testkit::{subscribe, world};

    fn id_of(state: &AppState, name: &str) -> Uuid {
        with_document(state, |doc| {
            Ok(doc.nodes.iter().find(|n| n.name() == name).map(Node::id))
        })
        .expect("document")
        .unwrap_or_else(|| panic!("{name} not found"))
    }

    #[tokio::test]
    async fn sessions_resolve_remote_connections_through_merged_view() {
        let w = world();
        subscribe(&w).await;
        let web = id_of(&w.sub, "web");
        // What term/rdp/vnc/web/fb use to find a connection sees the mounted one.
        let host = with_document(&w.sub, |doc| match doc.node(web) {
            Some(Node::Connection(c)) => Ok(c.host.clone()),
            _ => Err(AppError::not_found("missing")),
        })
        .expect("merged lookup");
        assert_eq!(host, "web.example.net");
        // The raw store does not hold it.
        assert!(with_local_document(&w.sub, |doc| Ok(doc.node(web).is_none())).expect("local"));
        let view = get(&w.sub).expect("view");
        assert!(view.remote_nodes.contains_key(&web));
        assert!(view.remote_nodes[&id_of(&w.sub, "Team")].is_root);
        assert_eq!(view.shares.len(), 1);
    }

    #[tokio::test]
    async fn upsert_refuses_remote_node_and_remote_parent() {
        let w = world();
        subscribe(&w).await;
        let team = id_of(&w.sub, "Team");
        let web = id_of(&w.sub, "web");
        let Some(node) = with_document(&w.sub, |d| Ok(d.node(web).cloned())).expect("doc") else {
            panic!("web");
        };
        assert_eq!(
            upsert(&w.sub, node).expect_err("remote node").code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            upsert(&w.sub, conn(77, None).clone()).map(|_| ()).err(),
            None,
            "local nodes still work"
        );
        let Node::Connection(mut local) = conn(78, None) else {
            panic!("connection");
        };
        local.parent_id = Some(team);
        assert_eq!(
            upsert(&w.sub, Node::Connection(local))
                .expect_err("remote parent")
                .code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            delete(&w.sub, web).expect_err("delete").code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            reorder_node(&w.sub, web, None, None, None)
                .expect_err("reorder")
                .code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            reorder_node(&w.sub, u(77), Some(team), None, None)
                .expect_err("into remote")
                .code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            sort_by_name(&w.sub, Some(team)).expect_err("sort").code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            move_to(&w.sub, web, None).expect_err("move out").code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            move_to(&w.sub, u(77), Some(team))
                .expect_err("move in")
                .code,
            ErrorCode::ReadOnly
        );
        // A local node dropped before a mounted root goes to the end of its group.
        reorder_node(&w.sub, u(77), None, Some(team), None).expect("before a root");
    }

    #[tokio::test]
    async fn delete_local_folder_with_mount_is_conflict() {
        let w = world();
        upsert(&w.sub, folder(5, None)).expect("folder");
        upsert(&w.sub, folder(6, Some(5))).expect("inner");
        let preview = w
            .sub
            .shares
            .preview(crate::shares::testkit::at(&w.file))
            .await
            .expect("preview");
        w.sub
            .shares
            .subscribe(preview.preview_id, Some(u(6)))
            .expect("subscribe");
        for id in [5, 6] {
            let err = delete(&w.sub, u(id)).expect_err("contains a share");
            assert_eq!(err.code, ErrorCode::Conflict);
            assert_eq!(err.message, "share.deleteContainsShares");
        }
        // After unsubscribing the folder can go.
        let link = w
            .sub
            .shares
            .with_store(|s| Ok(s.document().shares[0].id))
            .expect("link");
        w.sub.shares.unsubscribe(link).expect("unsubscribe");
        delete(&w.sub, u(5)).expect("delete");
    }

    #[tokio::test]
    async fn moving_a_mount_root_into_a_remote_folder_is_read_only() {
        let w = world();
        subscribe(&w).await;
        upsert(&w.sub, folder(5, None)).expect("folder");
        let team = id_of(&w.sub, "Team");
        let remote_child = with_document(&w.sub, |d| {
            Ok(d.nodes.iter().find(|n| n.name() == "web").map(Node::id))
        })
        .expect("doc")
        .expect("web");
        upsert(&w.sub, folder(6, None)).expect("second folder");
        // Another mount root cannot go inside a remote node either; try the mount itself.
        assert_eq!(
            move_to(&w.sub, team, Some(team))
                .expect_err("into itself")
                .code,
            ErrorCode::ReadOnly
        );
        assert_eq!(
            move_to(&w.sub, u(6), Some(remote_child))
                .expect_err("into a remote")
                .code,
            ErrorCode::ReadOnly
        );
    }

    #[tokio::test]
    async fn move_mount_root_updates_link() {
        let w = world();
        subscribe(&w).await;
        upsert(&w.sub, folder(5, None)).expect("folder");
        let team = id_of(&w.sub, "Team");
        move_to(&w.sub, team, Some(u(5))).expect("move");
        let parent = w
            .sub
            .shares
            .with_store(|s| Ok(s.document().shares[0].parent_id))
            .expect("link");
        assert_eq!(parent, Some(u(5)));
        assert_eq!(
            with_document(&w.sub, |d| Ok(d.node(team).and_then(Node::parent_id))).expect("doc"),
            Some(u(5))
        );
        // A connection is not a folder.
        upsert(&w.sub, conn(9, None)).expect("conn");
        assert_eq!(
            move_to(&w.sub, team, Some(u(9)))
                .expect_err("not a folder")
                .code,
            ErrorCode::InvalidInput
        );
        move_to(&w.sub, team, None).expect("back to the root");
        assert_eq!(
            w.sub
                .shares
                .with_store(|s| Ok(s.document().shares[0].parent_id))
                .expect("link"),
            None
        );
    }
}
