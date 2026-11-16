//! `DocumentStore`: load, mutate and atomically save the connection document.
//!
//! # On-disk layout (all in the same directory)
//!
//! - `connections.json`: the document.
//! - `connections.json.bak`: the last valid version that was replaced by a save.
//! - `connections.json.tmp` / `.bak.tmp`: transient files of an in-progress save.
//! - `connections.json.corrupt-<unix secs>`: a corrupt main file set aside during a recovery,
//!   kept so that nothing is ever silently lost.
//!
//! # Load rules
//!
//! - Missing main file (and no backup): empty document, no error.
//! - Corrupt or truncated main file (invalid JSON, wrong shape, or broken invariants): the
//!   backup is used if it is valid and the event is reported through [`DocumentStore::recovery`];
//!   if there is no valid backup the load fails and the file is left untouched.
//! - A file with a `schemaVersion` newer than [`SCHEMA_VERSION`] is refused with
//!   [`ModelError::NewerSchema`] and never modified (the backup is not consulted either).
//! - Older versions go through [`migrate`] and are saved back (the old file becomes the backup).
//!
//! The store is loaded lazily: [`DocumentStore::new`] does no I/O and every mutation calls
//! [`DocumentStore::ensure_loaded`] first, so a store whose file could not be read can never
//! overwrite it with an empty document.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::error::ModelError;
use crate::model::{Connection, ConnectionOptions, CredentialRef, Document, Node, SCHEMA_VERSION};

/// Report of a load that had to use the backup instead of the main file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// Why the main file was not usable (never contains file contents).
    pub reason: String,
    /// The file the document was restored from.
    pub restored_from: PathBuf,
    /// Where the unusable main file was set aside, if it existed.
    pub corrupt_copy: Option<PathBuf>,
}

#[derive(Debug)]
pub struct DocumentStore {
    path: PathBuf,
    document: Document,
    loaded: bool,
    recovery: Option<Recovery>,
}

/// Outcome of reading one file.
enum FileRead {
    Missing,
    Valid { document: Document, migrated: bool },
    Invalid(String),
}

impl DocumentStore {
    /// An empty, not yet loaded store bound to `path`. Performs no I/O.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            document: Document::default(),
            loaded: false,
            recovery: None,
        }
    }

    /// Creates and loads a store (see the module docs for the rules).
    pub fn load(path: PathBuf) -> Result<Self, ModelError> {
        let mut store = Self::new(path);
        store.ensure_loaded()?;
        Ok(store)
    }

    /// Loads from disk if that has not happened yet. On failure the store stays unloaded (and
    /// the next call retries), so mutations are refused until the file is readable.
    pub fn ensure_loaded(&mut self) -> Result<(), ModelError> {
        if self.loaded {
            return Ok(());
        }
        let (document, recovery) = load_from_disk(&self.path)?;
        self.document = document;
        self.recovery = recovery;
        self.loaded = true;
        Ok(())
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The in-memory document. Empty until [`DocumentStore::ensure_loaded`] succeeded.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Set when the last load restored the document from the backup.
    pub fn recovery(&self) -> Option<&Recovery> {
        self.recovery.as_ref()
    }

    /// Inserts or replaces a node (by id) and persists. Returns the stored node. A new node is
    /// appended; a replaced node keeps its position.
    pub fn upsert_node(&mut self, node: Node) -> Result<Node, ModelError> {
        self.mutate(|doc| {
            if node.name().trim().is_empty() {
                return Err(ModelError::InvalidInput(
                    "the name must not be empty".into(),
                ));
            }
            if let Node::Connection(c) = &node {
                // The host ends up in line-based stores (known hosts, pinned certificates).
                if !crate::model::host_is_storable(c.host.trim()) {
                    return Err(ModelError::InvalidInput(
                        "the host must not contain spaces or control characters".into(),
                    ));
                }
            }
            let id = node.id();
            check_parent(doc, id, node.parent_id())?;
            match doc.nodes.iter().position(|n| n.id() == id) {
                Some(pos) => {
                    if doc.nodes[pos].is_folder()
                        && !node.is_folder()
                        && doc.nodes.iter().any(|n| n.parent_id() == Some(id))
                    {
                        return Err(ModelError::InvalidInput(
                            "a folder that has children cannot become a connection".into(),
                        ));
                    }
                    doc.nodes[pos] = node.clone();
                }
                None => doc.nodes.push(node.clone()),
            }
            Ok(node)
        })
    }

    /// Deletes a node and, for folders, all its descendants, then persists. In the same atomic
    /// change, every SSH `jump_host` that pointed at a deleted connection is cleared, so no
    /// reference is left dangling.
    pub fn delete_node(&mut self, id: Uuid) -> Result<(), ModelError> {
        self.apply_batch(|doc| {
            if doc.node(id).is_none() {
                return Err(ModelError::NotFound(id.to_string()));
            }
            let mut doomed: HashSet<Uuid> = HashSet::from([id]);
            // Fixed point over the parent links; terminates because the set only grows.
            loop {
                let before = doomed.len();
                for node in &doc.nodes {
                    if node.parent_id().is_some_and(|p| doomed.contains(&p)) {
                        doomed.insert(node.id());
                    }
                }
                if doomed.len() == before {
                    break;
                }
            }
            doc.nodes.retain(|n| !doomed.contains(&n.id()));
            for node in &mut doc.nodes {
                if let Node::Connection(Connection {
                    options: ConnectionOptions::Ssh { jump_host, .. },
                    ..
                }) = node
                {
                    if jump_host.is_some_and(|j| doomed.contains(&j)) {
                        *jump_host = None;
                    }
                }
            }
            Ok(())
        })
    }

    /// Turns every `CredentialRef::Id` pointing at `credential_id` (folders and connections)
    /// into `CredentialRef::None` and persists once. Returns how many nodes changed; nothing is
    /// written when that is zero.
    pub fn clear_credential_references(
        &mut self,
        credential_id: Uuid,
    ) -> Result<usize, ModelError> {
        self.ensure_loaded()?;
        let points_here = |reference: &CredentialRef| matches!(reference, CredentialRef::Id { id } if *id == credential_id);
        let affected = self
            .document
            .nodes
            .iter()
            .filter(|n| match n {
                Node::Folder(f) => points_here(&f.credential),
                Node::Connection(c) => points_here(&c.credential),
            })
            .count();
        if affected == 0 {
            return Ok(0);
        }
        self.apply_batch(|doc| {
            for node in &mut doc.nodes {
                let reference = match node {
                    Node::Folder(f) => &mut f.credential,
                    Node::Connection(c) => &mut c.credential,
                };
                if points_here(reference) {
                    *reference = CredentialRef::None;
                }
            }
            Ok(affected)
        })
    }

    /// Re-parents a node (`None` = root), rejecting unknown or non-folder parents and cycles,
    /// then persists.
    pub fn move_node(&mut self, id: Uuid, new_parent_id: Option<Uuid>) -> Result<(), ModelError> {
        self.mutate(|doc| {
            if doc.node(id).is_none() {
                return Err(ModelError::NotFound(id.to_string()));
            }
            check_parent(doc, id, new_parent_id)?;
            for node in doc.nodes.iter_mut().filter(|n| n.id() == id) {
                node.set_parent_id(new_parent_id);
            }
            Ok(())
        })
    }

    /// Runs `f` once on a working copy of the document, then validates and saves the result
    /// once. The in-memory document is replaced only when everything succeeded: on any error
    /// (the closure's, validation or the save) memory and disk are left exactly as they were.
    ///
    /// Use it for multi-node changes (imports, cascades) where per-node saves would be slow and
    /// a partial result would be wrong.
    pub fn apply_batch<T>(
        &mut self,
        f: impl FnOnce(&mut Document) -> Result<T, ModelError>,
    ) -> Result<T, ModelError> {
        self.ensure_loaded()?;
        let mut working = self.document.clone();
        let value = f(&mut working)?;
        validate_document(&working)?;
        write_document(&self.path, &working)?;
        self.document = working;
        Ok(value)
    }

    /// Atomic save (temporary file in the same directory, fsync, rename) keeping a backup of
    /// the last valid version.
    pub fn save(&self) -> Result<(), ModelError> {
        write_document(&self.path, &self.document)
    }

    /// Runs `f` on the document; on any error (including a failed save) the in-memory document
    /// is rolled back so memory and disk never diverge.
    fn mutate<T>(
        &mut self,
        f: impl FnOnce(&mut Document) -> Result<T, ModelError>,
    ) -> Result<T, ModelError> {
        self.ensure_loaded()?;
        let snapshot = self.document.clone();
        let result = f(&mut self.document)
            .and_then(|value| validate_document(&self.document).map(|()| value))
            .and_then(|value| self.save().map(|()| value));
        if result.is_err() {
            self.document = snapshot;
        }
        result
    }
}

/// Migrates a raw document from `from` (< [`SCHEMA_VERSION`]) to the current schema by applying
/// [`migrate_step`] once per version.
fn migrate(mut value: serde_json::Value, from: u32) -> Result<serde_json::Value, ModelError> {
    let mut version = from;
    while version < SCHEMA_VERSION {
        value = migrate_step(value, version)?;
        version += 1;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("schemaVersion".into(), version.into());
        }
    }
    Ok(value)
}

/// Hook for future format changes: transforms a document from version `from` to `from + 1`.
/// Add one `match` arm per step. Version 1 is the first and current version, so for now every
/// input is unknown.
#[allow(clippy::match_single_binding)] // grows one arm per schema version
fn migrate_step(value: serde_json::Value, from: u32) -> Result<serde_json::Value, ModelError> {
    let _ = value;
    match from {
        // 1 => Ok(upgrade_v1_to_v2(value)),
        other => Err(ModelError::Corrupt(format!(
            "no migration from schema version {other}"
        ))),
    }
}

/// A parent reference is valid when it is `None`, or an existing folder that is neither the
/// node itself nor one of its descendants.
fn check_parent(doc: &Document, id: Uuid, parent: Option<Uuid>) -> Result<(), ModelError> {
    let Some(parent_id) = parent else {
        return Ok(());
    };
    if parent_id == id {
        return Err(ModelError::InvalidInput(
            "a node cannot be its own parent".into(),
        ));
    }
    match doc.node(parent_id) {
        None => {
            return Err(ModelError::InvalidInput(format!(
                "unknown parent {parent_id}"
            )))
        }
        Some(p) if !p.is_folder() => {
            return Err(ModelError::InvalidInput(format!(
                "parent {parent_id} is not a folder"
            )))
        }
        Some(_) => {}
    }
    // Walk up from the new parent: reaching `id` means the move would create a cycle.
    let index: HashMap<Uuid, &Node> = doc.nodes.iter().map(|n| (n.id(), n)).collect();
    let mut seen = HashSet::new();
    let mut cursor = Some(parent_id);
    while let Some(current) = cursor {
        if current == id {
            return Err(ModelError::InvalidInput(
                "the move would create a cycle".into(),
            ));
        }
        if !seen.insert(current) {
            break;
        }
        cursor = index.get(&current).and_then(|n| n.parent_id());
    }
    Ok(())
}

/// Whole-document invariants: unique ids, non-empty names, parents exist and are folders, no
/// parent cycles.
fn validate_document(doc: &Document) -> Result<(), ModelError> {
    let mut index: HashMap<Uuid, &Node> = HashMap::with_capacity(doc.nodes.len());
    for node in &doc.nodes {
        if index.insert(node.id(), node).is_some() {
            return Err(ModelError::InvalidInput(format!(
                "duplicate node id {}",
                node.id()
            )));
        }
        if node.name().trim().is_empty() {
            return Err(ModelError::InvalidInput(format!(
                "node {} has an empty name",
                node.id()
            )));
        }
    }
    for node in &doc.nodes {
        if let Some(parent_id) = node.parent_id() {
            match index.get(&parent_id) {
                None => {
                    return Err(ModelError::InvalidInput(format!(
                        "node {} has unknown parent {parent_id}",
                        node.id()
                    )))
                }
                Some(p) if !p.is_folder() => {
                    return Err(ModelError::InvalidInput(format!(
                        "parent {parent_id} of node {} is not a folder",
                        node.id()
                    )))
                }
                Some(_) => {}
            }
        }
        // Every chain must end at a root within `len` steps, otherwise it loops.
        let mut cursor = node.parent_id();
        let mut steps = 0usize;
        while let Some(current) = cursor {
            steps += 1;
            if steps > doc.nodes.len() {
                return Err(ModelError::InvalidInput(format!(
                    "cycle in the parents of node {}",
                    node.id()
                )));
            }
            cursor = index.get(&current).and_then(|n| n.parent_id());
        }
    }
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    sibling_with_suffix(path, ".bak")
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Reads and checks one file. I/O errors other than "not found" and a too-new schema version
/// are hard errors; everything else about the content is reported as `Invalid`.
fn read_file(path: &Path) -> Result<FileRead, ModelError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileRead::Missing),
        Err(e) => return Err(ModelError::Io(e)),
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            // Only position and category: serde messages may quote file content.
            return Ok(FileRead::Invalid(format!(
                "not valid JSON ({:?} at line {}, column {})",
                e.classify(),
                e.line(),
                e.column()
            )));
        }
    };
    let version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());
    let version = match version {
        Some(v) if v >= 1 => v,
        _ => return Ok(FileRead::Invalid("missing or invalid schemaVersion".into())),
    };
    if version > SCHEMA_VERSION {
        return Err(ModelError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    let migrated = version < SCHEMA_VERSION;
    let value = match migrate(value, version) {
        Ok(v) => v,
        Err(ModelError::Corrupt(reason)) => return Ok(FileRead::Invalid(reason)),
        Err(e) => return Err(e),
    };
    let document: Document = match serde_json::from_value(value) {
        Ok(d) => d,
        Err(e) => {
            return Ok(FileRead::Invalid(format!(
                "unexpected document structure ({:?})",
                e.classify()
            )))
        }
    };
    if let Err(e) = validate_document(&document) {
        return Ok(FileRead::Invalid(e.to_string()));
    }
    Ok(FileRead::Valid { document, migrated })
}

fn load_from_disk(path: &Path) -> Result<(Document, Option<Recovery>), ModelError> {
    let bak = backup_path(path);
    match read_file(path)? {
        FileRead::Valid { document, migrated } => {
            if migrated {
                // The pre-migration file becomes the backup (see `write_document`).
                write_document(path, &document)?;
            }
            Ok((document, None))
        }
        FileRead::Missing => match read_file(&bak)? {
            FileRead::Valid { document, .. } => {
                write_atomic(path, &document)?;
                let recovery = Recovery {
                    reason: "the connections file is missing".into(),
                    restored_from: bak,
                    corrupt_copy: None,
                };
                Ok((document, Some(recovery)))
            }
            FileRead::Missing | FileRead::Invalid(_) => Ok((Document::default(), None)),
        },
        FileRead::Invalid(reason) => match read_file(&bak)? {
            FileRead::Valid { document, .. } => {
                let corrupt_copy = set_aside(path)?;
                write_atomic(path, &document)?;
                let recovery = Recovery {
                    reason,
                    restored_from: bak,
                    corrupt_copy: Some(corrupt_copy),
                };
                Ok((document, Some(recovery)))
            }
            FileRead::Missing => Err(ModelError::Corrupt(format!(
                "{reason}; no backup is available"
            ))),
            FileRead::Invalid(bak_reason) => Err(ModelError::Corrupt(format!(
                "{reason}; the backup is unusable too ({bak_reason})"
            ))),
        },
    }
}

/// Renames the unusable main file to `<name>.corrupt-<unix secs>` (never overwriting).
fn set_aside(path: &Path) -> Result<PathBuf, ModelError> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut target = sibling_with_suffix(path, &format!(".corrupt-{secs}"));
    let mut n = 1u32;
    while target.exists() {
        target = sibling_with_suffix(path, &format!(".corrupt-{secs}-{n}"));
        n += 1;
    }
    fs::rename(path, &target)?;
    Ok(target)
}

/// Saves `document` to `path`, first copying the current main file to the backup when (and
/// only when) it is itself a valid document, so a corrupt file never replaces a good backup.
fn write_document(path: &Path, document: &Document) -> Result<(), ModelError> {
    if matches!(read_file(path), Ok(FileRead::Valid { .. })) {
        let tmp = sibling_with_suffix(path, ".bak.tmp");
        fs::copy(path, &tmp)?;
        crate::private_fs::restrict_file(&tmp)?;
        // Flushing needs write access on Windows (FlushFileBuffers): a read-only handle from
        // `File::open` fails there with "Access is denied". The handle is closed (dropped at
        // the end of the statement) before the rename.
        OpenOptions::new().write(true).open(&tmp)?.sync_all()?;
        fs::rename(&tmp, backup_path(path))?;
    }
    write_atomic(path, document)
}

/// Temporary file in the same directory, fsync, rename, fsync of the directory.
fn write_atomic(path: &Path, document: &Document) -> Result<(), ModelError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    // The document lists hosts and user names: readable by the user only.
    let json = serde_json::to_vec_pretty(document).map_err(|e| {
        ModelError::Corrupt(format!(
            "cannot serialize the document ({:?})",
            e.classify()
        ))
    })?;
    let tmp = sibling_with_suffix(path, ".tmp");
    let written = (|| -> std::io::Result<()> {
        let mut file = crate::private_fs::private_create_options().open(&tmp)?;
        crate::private_fs::restrict_file(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        // Windows cannot rename a file that is still open here: close it first.
        drop(file);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        if let Err(cleanup) = fs::remove_file(&tmp) {
            if cleanup.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %cleanup, "could not remove the temporary connections file");
            }
        }
        return Err(ModelError::Io(e));
    }
    #[cfg(unix)]
    // Directories can be opened and flushed on unix only (opening one fails on Windows).
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::model::{Connection, ConnectionOptions, CredentialRef, Folder};

    /// Temp directory under `<repo>/.tmp/wp2/` (git-ignored), removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/wp2");
            let dir = base.join(format!(
                "store-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("connections.json")
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

    fn folder(id: u128, parent: Option<u128>, name: &str) -> Node {
        Node::Folder(Folder {
            id: u(id),
            parent_id: parent.map(u),
            name: name.into(),
            credential: CredentialRef::Inherit,
        })
    }

    fn conn(id: u128, parent: Option<u128>, name: &str) -> Node {
        Node::Connection(Connection {
            id: u(id),
            parent_id: parent.map(u),
            name: name.into(),
            host: "example.test".into(),
            port: Some(22),
            credential: CredentialRef::Inherit,
            username: None,
            tags: vec!["t".into()],
            notes: String::new(),
            options: ConnectionOptions::Vnc { view_only: true },
        })
    }

    fn code(r: Result<impl std::fmt::Debug, ModelError>) -> crate::ErrorCode {
        r.expect_err("expected an error").code()
    }

    #[test]
    fn new_store_is_empty_and_does_no_io() {
        let store = DocumentStore::new(PathBuf::from("/nonexistent/connections.json"));
        assert!(store.document().nodes.is_empty());
        assert!(!store.is_loaded());
        assert_eq!(store.path(), Path::new("/nonexistent/connections.json"));
    }

    #[test]
    fn a_host_with_spaces_or_control_characters_is_refused() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        for host in ["a b", "a\nb", "a\tb", "a\u{0}b"] {
            let mut node = conn(1, None, "c");
            if let Node::Connection(c) = &mut node {
                c.host = host.into();
            }
            assert_eq!(
                code(store.upsert_node(node)),
                crate::ErrorCode::InvalidInput
            );
        }
        assert!(store.document().nodes.is_empty());
        let mut ok = conn(1, None, "c");
        if let Node::Connection(c) = &mut ok {
            c.host = "  host.example  ".into();
        }
        store
            .upsert_node(ok)
            .expect("surrounding spaces are trimmed away");
    }

    /// The first save has nothing to back up; every later one copies the previous file over
    /// the backup (an existing destination) and replaces the main file. Windows refuses both
    /// when a handle is still open or was opened without write access, so this runs everywhere.
    #[test]
    fn consecutive_saves_replace_the_file_and_the_backup_and_reload_cleanly() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        for n in 1..=4u128 {
            store
                .upsert_node(folder(n, None, &format!("F{n}")))
                .unwrap_or_else(|e| panic!("save {n}: {e:?}"));
        }
        assert!(backup_path(&dir.file()).exists());
        // Save right after a load, then load again.
        let mut again = DocumentStore::load(dir.file()).expect("reload");
        assert_eq!(again.document().nodes.len(), 4);
        again
            .upsert_node(folder(5, None, "F5"))
            .expect("save after load");
        assert_eq!(
            DocumentStore::load(dir.file())
                .expect("final")
                .document()
                .nodes
                .len(),
            5
        );
        // No temporary file is left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.file().parent().expect("parent"))
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn the_document_and_its_backup_are_readable_by_the_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("first save");
        store
            .upsert_node(folder(2, None, "B"))
            .expect("second save");
        let mode = |p: &Path| fs::metadata(p).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode(&dir.file()), 0o600);
        assert_eq!(mode(&backup_path(&dir.file())), 0o600);
    }

    #[test]
    fn missing_file_is_an_empty_document() {
        let dir = TempDir::new();
        let store = DocumentStore::load(dir.file()).expect("load");
        assert!(store.document().nodes.is_empty());
        assert_eq!(store.document().schema_version, SCHEMA_VERSION);
        assert!(store.recovery().is_none());
        assert!(!dir.file().exists(), "loading must not create the file");
    }

    #[test]
    fn save_and_reload_round_trip() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "F")).expect("folder");
        store.upsert_node(conn(2, Some(1), "C")).expect("conn");
        let reloaded = DocumentStore::load(dir.file()).expect("reload");
        assert_eq!(reloaded.document(), store.document());
        assert_eq!(reloaded.document().nodes.len(), 2);
        assert!(
            !sibling_with_suffix(&dir.file(), ".tmp").exists(),
            "no temp file left behind"
        );
    }

    #[test]
    fn save_keeps_backup_of_previous_version() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("first");
        store.upsert_node(folder(2, None, "B")).expect("second");
        let bak = DocumentStore::load(backup_path(&dir.file())).expect("bak");
        assert_eq!(bak.document().nodes.len(), 1);
    }

    #[test]
    fn corrupt_main_is_recovered_from_backup() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("first");
        store.upsert_node(folder(2, None, "B")).expect("second");
        fs::write(dir.file(), b"{ this is not json").expect("corrupt");

        let recovered = DocumentStore::load(dir.file()).expect("recover");
        let info = recovered.recovery().expect("recovery is reported");
        assert_eq!(info.restored_from, backup_path(&dir.file()));
        let copy = info.corrupt_copy.as_ref().expect("corrupt copy kept");
        assert_eq!(fs::read(copy).expect("read copy"), b"{ this is not json");
        // Backup had only the first node (the version before the last save).
        assert_eq!(recovered.document().nodes.len(), 1);
        // The main file is valid again.
        let again = DocumentStore::load(dir.file()).expect("reload");
        assert!(again.recovery().is_none());
        assert_eq!(again.document(), recovered.document());
    }

    #[test]
    fn truncated_main_is_recovered_from_backup() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("first");
        store.upsert_node(conn(2, Some(1), "B")).expect("second");
        let full = fs::read(dir.file()).expect("read");
        fs::write(dir.file(), &full[..full.len() / 2]).expect("truncate");
        let recovered = DocumentStore::load(dir.file()).expect("recover");
        assert!(recovered.recovery().is_some());
        assert_eq!(recovered.document().nodes.len(), 1);
    }

    #[test]
    fn empty_main_without_backup_is_an_error_and_untouched() {
        let dir = TempDir::new();
        fs::write(dir.file(), b"").expect("write");
        let err = DocumentStore::load(dir.file()).expect_err("corrupt");
        assert!(matches!(err, ModelError::Corrupt(_)));
        assert_eq!(err.code(), crate::ErrorCode::Io);
        assert_eq!(fs::read(dir.file()).expect("read"), b"");
    }

    #[test]
    fn corrupt_main_and_corrupt_backup_is_an_error_and_both_untouched() {
        let dir = TempDir::new();
        fs::write(dir.file(), b"{\"schemaVersion\":1,\"nodes\":[").expect("write");
        fs::write(backup_path(&dir.file()), b"garbage").expect("write bak");
        let err = DocumentStore::load(dir.file()).expect_err("corrupt");
        assert!(matches!(err, ModelError::Corrupt(_)));
        assert_eq!(
            fs::read(dir.file()).expect("read"),
            b"{\"schemaVersion\":1,\"nodes\":["
        );
        assert_eq!(
            fs::read(backup_path(&dir.file())).expect("read"),
            b"garbage"
        );
    }

    #[test]
    fn failed_load_never_lets_a_mutation_overwrite_the_file() {
        let dir = TempDir::new();
        fs::write(dir.file(), b"garbage").expect("write");
        let mut store = DocumentStore::new(dir.file());
        assert_eq!(
            code(store.upsert_node(folder(1, None, "A"))),
            crate::ErrorCode::Io
        );
        assert_eq!(fs::read(dir.file()).expect("read"), b"garbage");
    }

    #[test]
    fn missing_main_is_restored_from_backup() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("first");
        store.upsert_node(folder(2, None, "B")).expect("second");
        fs::remove_file(dir.file()).expect("remove");
        let recovered = DocumentStore::load(dir.file()).expect("recover");
        assert!(recovered.recovery().is_some());
        assert_eq!(recovered.document().nodes.len(), 1);
    }

    #[test]
    fn newer_schema_is_refused_and_untouched() {
        let dir = TempDir::new();
        let content = br#"{"schemaVersion":99,"nodes":[],"futureField":true}"#;
        fs::write(dir.file(), content).expect("write");
        // A valid backup must not be used as a fallback for a newer file.
        fs::write(
            backup_path(&dir.file()),
            br#"{"schemaVersion":1,"nodes":[]}"#,
        )
        .expect("bak");
        let err = DocumentStore::load(dir.file()).expect_err("newer");
        assert!(matches!(
            err,
            ModelError::NewerSchema {
                found: 99,
                supported: 1
            }
        ));
        assert_eq!(err.code(), crate::ErrorCode::Unsupported);
        assert_eq!(fs::read(dir.file()).expect("read"), content);

        let mut store = DocumentStore::new(dir.file());
        assert!(store.upsert_node(folder(1, None, "A")).is_err());
        assert_eq!(fs::read(dir.file()).expect("read"), content);
    }

    #[test]
    fn invalid_document_invariants_count_as_corrupt() {
        let dir = TempDir::new();
        // Valid JSON and shape, but the parent does not exist.
        let json = serde_json::to_vec(&Document {
            schema_version: 1,
            nodes: vec![folder(1, Some(42), "A")],
        })
        .expect("json");
        fs::write(dir.file(), json).expect("write");
        assert!(matches!(
            DocumentStore::load(dir.file()),
            Err(ModelError::Corrupt(_))
        ));
    }

    #[test]
    fn unique_ids_are_enforced_across_kinds_only_by_replacement() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("insert");
        store
            .upsert_node(folder(1, None, "A renamed"))
            .expect("replace");
        assert_eq!(store.document().nodes.len(), 1);
        assert_eq!(store.document().nodes[0].name(), "A renamed");
    }

    #[test]
    fn replacing_keeps_position() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        for n in 1..=3 {
            store.upsert_node(folder(n, None, "x")).expect("insert");
        }
        store.upsert_node(folder(2, None, "y")).expect("replace");
        let ids: Vec<_> = store.document().nodes.iter().map(Node::id).collect();
        assert_eq!(ids, vec![u(1), u(2), u(3)]);
    }

    #[test]
    fn duplicate_id_in_a_loaded_file_is_corrupt() {
        let dir = TempDir::new();
        let json = serde_json::to_vec(&Document {
            schema_version: 1,
            nodes: vec![folder(1, None, "A"), conn(1, None, "B")],
        })
        .expect("json");
        fs::write(dir.file(), json).expect("write");
        assert!(matches!(
            DocumentStore::load(dir.file()),
            Err(ModelError::Corrupt(_))
        ));
    }

    #[test]
    fn upsert_validates_names_and_parents() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "F")).expect("folder");
        store.upsert_node(conn(2, Some(1), "C")).expect("conn");

        assert_eq!(
            code(store.upsert_node(folder(3, None, ""))),
            crate::ErrorCode::InvalidInput
        );
        assert_eq!(
            code(store.upsert_node(folder(3, None, "   "))),
            crate::ErrorCode::InvalidInput
        );
        // Unknown parent.
        assert_eq!(
            code(store.upsert_node(conn(3, Some(99), "x"))),
            crate::ErrorCode::InvalidInput
        );
        // Parent is a connection.
        assert_eq!(
            code(store.upsert_node(conn(3, Some(2), "x"))),
            crate::ErrorCode::InvalidInput
        );
        // Own parent.
        assert_eq!(
            code(store.upsert_node(folder(1, Some(1), "F"))),
            crate::ErrorCode::InvalidInput
        );
        // Folder with children turned into a connection.
        assert_eq!(
            code(store.upsert_node(conn(1, None, "F"))),
            crate::ErrorCode::InvalidInput
        );
        // Nothing of the above changed the document.
        assert_eq!(store.document().nodes.len(), 2);
    }

    #[test]
    fn upsert_rejects_cycles() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        store.upsert_node(folder(2, Some(1), "B")).expect("b");
        store.upsert_node(folder(3, Some(2), "C")).expect("c");
        // Re-parent A under its own grandchild through an upsert.
        assert_eq!(
            code(store.upsert_node(folder(1, Some(3), "A"))),
            crate::ErrorCode::InvalidInput
        );
        assert_eq!(store.document().node(u(1)).and_then(Node::parent_id), None);
    }

    #[test]
    fn move_node_works_and_persists() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        store.upsert_node(folder(2, None, "B")).expect("b");
        store.upsert_node(conn(3, Some(1), "C")).expect("c");
        store.move_node(u(3), Some(u(2))).expect("move");
        assert_eq!(
            store.document().node(u(3)).and_then(Node::parent_id),
            Some(u(2))
        );
        store.move_node(u(3), None).expect("to root");
        let reloaded = DocumentStore::load(dir.file()).expect("reload");
        assert_eq!(
            reloaded.document().node(u(3)).and_then(Node::parent_id),
            None
        );
    }

    #[test]
    fn move_node_rejects_bad_targets() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        store.upsert_node(folder(2, Some(1), "B")).expect("b");
        store.upsert_node(conn(3, Some(2), "C")).expect("c");
        // Into itself, into a descendant.
        assert_eq!(
            code(store.move_node(u(1), Some(u(1)))),
            crate::ErrorCode::InvalidInput
        );
        assert_eq!(
            code(store.move_node(u(1), Some(u(2)))),
            crate::ErrorCode::InvalidInput
        );
        // Under a connection, under an unknown parent.
        assert_eq!(
            code(store.move_node(u(2), Some(u(3)))),
            crate::ErrorCode::InvalidInput
        );
        assert_eq!(
            code(store.move_node(u(2), Some(u(99)))),
            crate::ErrorCode::InvalidInput
        );
        // Unknown node.
        assert_eq!(
            code(store.move_node(u(99), None)),
            crate::ErrorCode::NotFound
        );
        assert_eq!(store.document().node(u(1)).and_then(Node::parent_id), None);
    }

    #[test]
    fn delete_folder_deletes_the_whole_subtree() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        store.upsert_node(folder(2, Some(1), "B")).expect("b");
        store.upsert_node(conn(3, Some(2), "C")).expect("c");
        store.upsert_node(conn(4, Some(1), "D")).expect("d");
        store.upsert_node(folder(5, None, "Other")).expect("e");
        store.upsert_node(conn(6, Some(5), "Keep")).expect("f");
        store.delete_node(u(1)).expect("delete");
        let ids: Vec<_> = store.document().nodes.iter().map(Node::id).collect();
        assert_eq!(ids, vec![u(5), u(6)]);
        let reloaded = DocumentStore::load(dir.file()).expect("reload");
        assert_eq!(reloaded.document().nodes.len(), 2);
    }

    #[test]
    fn delete_connection_and_unknown_id() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        store.upsert_node(conn(2, Some(1), "C")).expect("c");
        store.delete_node(u(2)).expect("delete");
        assert_eq!(store.document().nodes.len(), 1);
        assert_eq!(code(store.delete_node(u(2))), crate::ErrorCode::NotFound);
    }

    #[test]
    fn failed_save_rolls_back_memory() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        // Make the destination a directory so the final rename fails.
        fs::remove_file(dir.file()).expect("remove");
        fs::create_dir(dir.file()).expect("dir in the way");
        assert_eq!(
            code(store.upsert_node(folder(2, None, "B"))),
            crate::ErrorCode::Io
        );
        assert_eq!(store.document().nodes.len(), 1, "rolled back");
        assert!(!sibling_with_suffix(&dir.file(), ".tmp").exists());
    }

    fn ssh_via(id: u128, parent: Option<u128>, jump: Option<u128>) -> Node {
        Node::Connection(Connection {
            id: u(id),
            parent_id: parent.map(u),
            name: format!("ssh{id}"),
            host: "example.test".into(),
            port: None,
            credential: CredentialRef::Inherit,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Ssh {
                jump_host: jump.map(u),
                keepalive_secs: Some(30),
            },
        })
    }

    fn jump_of(store: &DocumentStore, id: u128) -> Option<Uuid> {
        match store.document().node(u(id)) {
            Some(Node::Connection(Connection {
                options: ConnectionOptions::Ssh { jump_host, .. },
                ..
            })) => *jump_host,
            other => panic!("not an ssh connection: {other:?}"),
        }
    }

    #[test]
    fn deleting_a_connection_clears_jump_hosts_that_pointed_at_it() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(ssh_via(1, None, None)).expect("bastion");
        store.upsert_node(ssh_via(2, None, Some(1))).expect("a");
        store.upsert_node(ssh_via(3, None, Some(1))).expect("b");
        store.upsert_node(ssh_via(4, None, Some(2))).expect("c");
        store.delete_node(u(1)).expect("delete");
        assert_eq!(jump_of(&store, 2), None);
        assert_eq!(jump_of(&store, 3), None);
        assert_eq!(jump_of(&store, 4), Some(u(2)), "unrelated references stay");
        let reloaded = DocumentStore::load(dir.file()).expect("reload");
        assert_eq!(reloaded.document(), store.document());
    }

    #[test]
    fn deleting_a_folder_clears_jump_hosts_to_its_descendants() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "Hosts")).expect("folder");
        store
            .upsert_node(ssh_via(2, Some(1), None))
            .expect("inside");
        store
            .upsert_node(ssh_via(3, None, Some(2)))
            .expect("outside");
        store.delete_node(u(1)).expect("delete");
        assert_eq!(store.document().nodes.len(), 1);
        assert_eq!(jump_of(&store, 3), None);
    }

    #[test]
    fn clearing_credential_references_touches_only_that_id() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        let with = |node: Node, reference: CredentialRef| match node {
            Node::Folder(mut f) => {
                f.credential = reference;
                Node::Folder(f)
            }
            Node::Connection(mut c) => {
                c.credential = reference;
                Node::Connection(c)
            }
        };
        let gone = CredentialRef::Id { id: u(900) };
        let other = CredentialRef::Id { id: u(901) };
        store
            .upsert_node(with(folder(1, None, "F"), gone))
            .expect("folder");
        store
            .upsert_node(with(conn(2, Some(1), "a"), gone))
            .expect("a");
        store
            .upsert_node(with(conn(3, Some(1), "b"), other))
            .expect("b");
        store.upsert_node(conn(4, None, "c")).expect("c");

        assert_eq!(store.clear_credential_references(u(900)).expect("clear"), 2);
        let refs: Vec<CredentialRef> = store
            .document()
            .nodes
            .iter()
            .map(|n| match n {
                Node::Folder(f) => f.credential,
                Node::Connection(c) => c.credential,
            })
            .collect();
        assert_eq!(
            refs,
            vec![
                CredentialRef::None,
                CredentialRef::None,
                other,
                CredentialRef::Inherit
            ]
        );
        let reloaded = DocumentStore::load(dir.file()).expect("reload");
        assert_eq!(reloaded.document(), store.document());

        // Nothing to clear: no write happens.
        let before = fs::read(dir.file()).expect("read");
        assert_eq!(store.clear_credential_references(u(900)).expect("noop"), 0);
        assert_eq!(fs::read(dir.file()).expect("read"), before);
    }

    #[test]
    fn apply_batch_saves_once_and_swaps_memory() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        let n = store
            .apply_batch(|doc| {
                doc.nodes.push(folder(1, None, "A"));
                doc.nodes.push(conn(2, Some(1), "c"));
                Ok(doc.nodes.len())
            })
            .expect("batch");
        assert_eq!(n, 2);
        assert_eq!(store.document().nodes.len(), 2);
        let reloaded = DocumentStore::load(dir.file()).expect("reload");
        assert_eq!(reloaded.document(), store.document());
    }

    #[test]
    fn apply_batch_changes_nothing_on_closure_or_validation_failure() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        let before_doc = store.document().clone();
        let before_bytes = fs::read(dir.file()).expect("read");

        let closure_err = store.apply_batch(|doc| {
            doc.nodes.push(folder(2, None, "B"));
            Err::<(), _>(ModelError::InvalidInput("nope".into()))
        });
        assert_eq!(code(closure_err), crate::ErrorCode::InvalidInput);

        let validation_err = store.apply_batch(|doc| {
            doc.nodes.push(folder(3, None, "C"));
            doc.nodes.push(folder(3, None, "dup"));
            Ok(())
        });
        assert_eq!(code(validation_err), crate::ErrorCode::InvalidInput);

        assert_eq!(store.document(), &before_doc);
        assert_eq!(fs::read(dir.file()).expect("read"), before_bytes);
    }

    #[test]
    fn apply_batch_save_failure_keeps_memory() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(folder(1, None, "A")).expect("a");
        fs::remove_file(dir.file()).expect("remove");
        fs::create_dir(dir.file()).expect("dir in the way");
        let result = store.apply_batch(|doc| {
            doc.nodes.push(folder(2, None, "B"));
            Ok(())
        });
        assert_eq!(code(result), crate::ErrorCode::Io);
        assert_eq!(store.document().nodes.len(), 1);
    }

    #[test]
    fn a_file_without_username_loads_and_one_with_it_round_trips() {
        let dir = TempDir::new();
        // Written before the field existed: no `username` key on the connection.
        let old = r#"{"schemaVersion":1,"nodes":[{"type":"connection",
            "id":"00000000-0000-0000-0000-000000000002","parentId":null,"name":"old",
            "host":"h","port":null,"credential":{"type":"inherit"},"tags":[],"notes":"",
            "options":{"type":"ssh","jumpHost":null,"keepaliveSecs":null}}]}"#;
        fs::write(dir.file(), old).expect("write");
        let mut store = DocumentStore::load(dir.file()).expect("load");
        let Some(Node::Connection(c)) = store.document().node(u(2)).cloned() else {
            panic!("connection missing");
        };
        assert_eq!(c.username, None);

        let mut with_name = c;
        with_name.username = Some("alice".into());
        store
            .upsert_node(Node::Connection(with_name))
            .expect("upsert");
        let reloaded = DocumentStore::load(dir.file()).expect("reload");
        let Some(Node::Connection(c)) = reloaded.document().node(u(2)) else {
            panic!("connection missing");
        };
        assert_eq!(c.username.as_deref(), Some("alice"));
    }

    #[test]
    fn saved_file_matches_the_json_convention() {
        let dir = TempDir::new();
        let mut store = DocumentStore::load(dir.file()).expect("load");
        store.upsert_node(conn(2, None, "C")).expect("c");
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.file()).expect("read")).expect("json");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["nodes"][0]["type"], "connection");
        assert_eq!(value["nodes"][0]["parentId"], serde_json::Value::Null);
        assert_eq!(value["nodes"][0]["credential"]["type"], "inherit");
    }
}
