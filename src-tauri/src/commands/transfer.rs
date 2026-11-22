//! Import and export commands (WP9). Parsing and writing live in `portolano-importers`; these
//! functions only connect them to the application state. The `*_impl` functions hold the logic
//! and are what the tests call, since a Tauri `State` cannot be built outside the app.

use std::path::Path;

use portolano_importers::{ExportFormat, ImportFormat, ImportPreview};
use tauri::State;
use uuid::Uuid;

use crate::commands::model::with_local_document;
use crate::error::{poisoned, AppError};
use crate::shares::read_only;
use crate::state::AppState;

/// Reads and parses `path`, caches the preview for `import_apply` and returns it.
fn import_preview_impl(
    state: &AppState,
    format: ImportFormat,
    path: &str,
) -> Result<ImportPreview, AppError> {
    let preview = portolano_importers::preview(format, Path::new(path))?;
    state
        .import_previews
        .lock()
        .map_err(poisoned)?
        .insert(preview.clone());
    Ok(preview)
}

/// Applies a cached preview under `target_folder_id` (`None` = root). The preview is removed only
/// when the import succeeded, so a failed apply can be retried.
fn import_apply_impl(
    state: &AppState,
    preview_id: Uuid,
    target_folder_id: Option<Uuid>,
) -> Result<u32, AppError> {
    let preview = state
        .import_previews
        .lock()
        .map_err(poisoned)?
        .get(preview_id)
        .ok_or_else(|| AppError::not_found("the import preview was not found or has expired"))?;
    // A shared folder belongs to its owner: nothing is imported into it.
    if let Some(target) = target_folder_id {
        let remote = with_local_document(state, |doc| state.shares.remote_map(doc))?;
        if remote.contains_key(&target) {
            return Err(read_only());
        }
    }
    let count = {
        let mut store = state.store.lock().map_err(poisoned)?;
        portolano_importers::apply(&mut store, target_folder_id, &preview)?
    };
    state
        .import_previews
        .lock()
        .map_err(poisoned)?
        .remove(preview_id);
    Ok(count)
}

/// Writes the selected nodes (with their descendants; `None` = the whole document) to `path`.
fn export_document_impl(
    state: &AppState,
    format: ExportFormat,
    path: &str,
    node_ids: Option<&[Uuid]>,
) -> Result<(), AppError> {
    // The local document only: what a shared folder holds belongs to its owner and is never
    // written to a file from here. Ids of shared nodes in the selection are ignored.
    let nodes = with_local_document(state, |doc| {
        // Ids that belong to a shared folder are ignored; unknown ids still fail.
        let remote = state.shares.remote_map(doc)?;
        let selected: Option<Vec<Uuid>> = node_ids.map(|ids| {
            ids.iter()
                .copied()
                .filter(|id| !remote.contains_key(id))
                .collect()
        });
        Ok(portolano_importers::select(doc, selected.as_deref())?)
    })?;
    portolano_importers::export(format, Path::new(path), &nodes)?;
    Ok(())
}

#[tauri::command]
pub async fn import_preview(
    state: State<'_, AppState>,
    format: ImportFormat,
    path: String,
) -> Result<ImportPreview, AppError> {
    import_preview_impl(&state, format, &path)
}

/// Returns the number of imported nodes. `target_folder_id: None` imports at the root.
#[tauri::command]
pub async fn import_apply(
    state: State<'_, AppState>,
    preview_id: Uuid,
    target_folder_id: Option<Uuid>,
) -> Result<u32, AppError> {
    import_apply_impl(&state, preview_id, target_folder_id)
}

#[tauri::command]
pub async fn export_document(
    state: State<'_, AppState>,
    format: ExportFormat,
    path: String,
    node_ids: Option<Vec<Uuid>>,
) -> Result<(), AppError> {
    export_document_impl(&state, format, &path, node_ids.as_deref())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use portolano_core_model::{
        Connection, ConnectionOptions, CredentialRef, Document, ErrorCode, Folder, Node,
    };

    use super::*;

    /// Temp directory under `<repo>/.tmp/wp9/` (git-ignored), removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp9");
            let dir = base.join(format!(
                "app-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
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

    fn csv_file(dir: &TempDir) -> PathBuf {
        let path = dir.path("list.csv");
        fs::write(
            &path,
            "name,protocol,host,port,folder\r\n\
             db01,ssh,db01.example.org,22,Prod/DB\r\n\
             desk01,rdp,10.0.0.5,3390,Prod\r\n",
        )
        .expect("write csv");
        path
    }

    fn state_in(dir: &TempDir) -> AppState {
        AppState::new(dir.0.clone())
    }

    #[test]
    fn preview_is_cached_and_apply_puts_nodes_under_target() {
        let dir = TempDir::new();
        let state = state_in(&dir);
        let folder = Node::Folder(Folder {
            sort_index: None,
            icon: None,
            color: None,
            id: u(1),
            parent_id: None,
            name: "Target".into(),
            credential: CredentialRef::Inherit,
        });
        state
            .store
            .lock()
            .expect("lock")
            .upsert_node(folder)
            .expect("folder");

        let preview = import_preview_impl(&state, ImportFormat::Csv, path_str(&csv_file(&dir)))
            .expect("preview");
        assert_eq!(preview.nodes.len(), 4, "two folders and two connections");

        let count = import_apply_impl(&state, preview.preview_id, Some(u(1))).expect("apply");
        assert_eq!(count, 4);
        let store = state.store.lock().expect("lock");
        let doc = store.document();
        assert_eq!(doc.nodes.len(), 5);
        let top: Vec<_> = doc
            .nodes
            .iter()
            .filter(|n| n.parent_id() == Some(u(1)))
            .map(Node::name)
            .collect();
        assert_eq!(top, vec!["Prod"]);
        drop(store);
        // Applied previews are removed.
        let again = import_apply_impl(&state, preview.preview_id, None).expect_err("consumed");
        assert_eq!(again.code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn import_into_a_shared_folder_is_read_only() {
        use crate::shares::testkit::{subscribe, world};
        let dir = TempDir::new();
        let w = world();
        subscribe(&w).await;
        let team = with_local_document(&w.sub, |doc| state_remote_root(&w.sub, doc))
            .expect("mounted root");
        let preview = import_preview_impl(&w.sub, ImportFormat::Csv, path_str(&csv_file(&dir)))
            .expect("preview");
        let err = import_apply_impl(&w.sub, preview.preview_id, Some(team)).expect_err("remote");
        assert_eq!(err.code, ErrorCode::ReadOnly);
        // Nothing was imported and the preview is still there for another target.
        assert_eq!(
            import_apply_impl(&w.sub, preview.preview_id, None).expect("elsewhere"),
            4
        );
    }

    fn state_remote_root(state: &AppState, doc: &Document) -> Result<Uuid, AppError> {
        state
            .shares
            .remote_map(doc)?
            .iter()
            .find(|(_, info)| info.is_root)
            .map(|(id, _)| *id)
            .ok_or_else(|| AppError::not_found("no mounted root"))
    }

    #[test]
    fn unknown_preview_is_not_found() {
        let dir = TempDir::new();
        let state = state_in(&dir);
        let err = import_apply_impl(&state, u(9), None).expect_err("unknown");
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn failed_apply_leaves_document_and_preview_untouched() {
        let dir = TempDir::new();
        let state = state_in(&dir);
        let preview = import_preview_impl(&state, ImportFormat::Csv, path_str(&csv_file(&dir)))
            .expect("preview");
        let before: Document = state.store.lock().expect("lock").document().clone();

        // The target folder does not exist: refused before anything is written.
        let err = import_apply_impl(&state, preview.preview_id, Some(u(77))).expect_err("missing");
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(state.store.lock().expect("lock").document(), &before);
        // The preview survives a failed apply, so it can be applied again.
        assert_eq!(
            import_apply_impl(&state, preview.preview_id, None).expect("retry"),
            4
        );
    }

    #[test]
    fn export_json_and_rdp_through_the_application_state() {
        let dir = TempDir::new();
        let state = state_in(&dir);
        {
            let mut store = state.store.lock().expect("lock");
            store
                .upsert_node(Node::Connection(Connection {
                    sort_index: None,
                    icon: None,
                    color: None,
                    id: u(2),
                    parent_id: None,
                    name: "desk01".into(),
                    host: "desk01.example.org".into(),
                    port: Some(3390),
                    credential: CredentialRef::Id { id: u(9) },
                    username: Some("ops".into()),
                    tags: vec![],
                    notes: String::new(),
                    options: ConnectionOptions::Rdp {
                        width: Some(1920),
                        height: Some(1080),
                        domain: None,
                        ignore_cert: false,
                        allow_without_nla: false,
                        kerberos_kdc: None,
                        share_clipboard: false,
                        mac_shortcuts: false,
                    },
                }))
                .expect("rdp connection");
            store
                .upsert_node(Node::Connection(Connection {
                    sort_index: None,
                    icon: None,
                    color: None,
                    id: u(3),
                    parent_id: None,
                    name: "ssh01".into(),
                    host: "ssh01.example.org".into(),
                    port: None,
                    credential: CredentialRef::None,
                    username: None,
                    tags: vec![],
                    notes: String::new(),
                    options: ConnectionOptions::Ssh {
                        jump_host: None,
                        keepalive_secs: None,
                    },
                }))
                .expect("ssh connection");
        }

        let json_path = dir.path("export.json");
        export_document_impl(&state, ExportFormat::Json, path_str(&json_path), None)
            .expect("json export");
        let json = fs::read_to_string(&json_path).expect("read json");
        assert!(json.contains("portolano-export"));
        assert!(!json.contains("\"password\""));

        let rdp_path = dir.path("desk01.rdp");
        export_document_impl(
            &state,
            ExportFormat::Rdp,
            path_str(&rdp_path),
            Some(&[u(2)]),
        )
        .expect("rdp export");
        assert!(rdp_path.exists());

        let err = export_document_impl(
            &state,
            ExportFormat::Rdp,
            path_str(&dir.path("x.rdp")),
            Some(&[u(3)]),
        )
        .expect_err("ssh is not rdp");
        assert_eq!(err.code, ErrorCode::InvalidInput);

        let missing = export_document_impl(
            &state,
            ExportFormat::Json,
            path_str(&dir.path("y.json")),
            Some(&[u(42)]),
        )
        .expect_err("unknown node");
        assert_eq!(missing.code, ErrorCode::NotFound);
    }

    fn path_str(path: &Path) -> &str {
        path.to_str().expect("utf-8 temp path")
    }

    #[tokio::test]
    async fn export_skips_remote_nodes() {
        let w = crate::shares::testkit::world();
        crate::shares::testkit::subscribe(&w).await;
        let remote = w
            .sub
            .shares
            .with_store(|store| {
                let merged = w.sub.shares.merged(store.document())?;
                Ok(merged
                    .document
                    .nodes
                    .iter()
                    .map(Node::id)
                    .filter(|id| store.document().node(*id).is_none())
                    .collect::<Vec<_>>())
            })
            .expect("remote ids");
        assert_eq!(remote.len(), 2);
        let local = Node::Folder(Folder {
            sort_index: None,
            icon: None,
            color: None,
            id: u(1),
            parent_id: None,
            name: "mine".into(),
            credential: CredentialRef::Inherit,
        });
        crate::commands::model::with_store(&w.sub, |s| Ok(s.upsert_node(local)?)).expect("local");
        let out = w._sub_dir.path().join("out.json");
        // The whole document, and a selection that names remote ids: neither carries them.
        for ids in [None, Some(vec![u(1), remote[0], remote[1]])] {
            export_document_impl(&w.sub, ExportFormat::Json, path_str(&out), ids.as_deref())
                .expect("export");
            let text = fs::read_to_string(&out).expect("read");
            assert!(text.contains("mine"), "{text}");
            assert!(
                !text.contains("web.example.net") && !text.contains("Team"),
                "{text}"
            );
        }
    }
}
