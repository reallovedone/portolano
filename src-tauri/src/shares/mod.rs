//! Shared folders (phase A): the service that publishes, subscribes to and syncs share files and
//! builds the merged tree every other command works on.
//!
//! What a share file may contain, who may sign it and how the merged tree is built is decided by
//! `portolano-shares` (pure code). This module does the I/O around it: reading and writing the
//! files, the offline cache, the vault (signing) and the document (the `shares` list).
//!
//! Rules that hold everywhere here:
//! - remote content is sanitised before it is kept, signed or not;
//! - a failed sync never touches the cache or the mounted tree, it only records `last_error`;
//! - error texts are shown to the user as they are, so they carry no paths and no file content;
//! - private keys leave the vault only to sign, and never reach a log, an error or the document.
//!
//! Lock order: document store, then `inner`. The vault is never locked while the store is.

pub mod transport;

#[cfg(test)]
pub(crate) mod testkit;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use ed25519_dalek::VerifyingKey;
use portolano_core_model::private_fs::write_private_atomic;
use portolano_core_model::{
    AppError, CredentialRef, Document, DocumentStore, ErrorCode, ModelError, Node, OwnedShare,
    ShareLink, ShareLocation, ShareRole,
};
use portolano_shares::accept::{accept, first_pin, AcceptMode, Pinned, Rejection};
use portolano_shares::format::{
    decode, encode, sign_editors, EditorList, KeyEntry, SharePayload, SignedShare,
};
use portolano_shares::identity::{decode_key, fingerprint, Identity};
use portolano_shares::sanitize::{prepare_for_publish, sanitize, SanitizeOutcome, SanitizedNodes};
use portolano_shares::view::{
    derive_id, merge, pending_host, target_of, MergedView, Mount, PendingHost, RemoteNode,
};
use portolano_shares::ShareError;
use portolano_vault::{IdentityRecord, Vault};
use serde::Serialize;
use uuid::Uuid;

use crate::error::poisoned;
use crate::state::{lock_vault_mutex, Notifier};
use transport::{Fetched, TransportError, Validator};

/// Event (to the main webview, no payload) after a sync changed a mount or an error.
pub const CHANGED_EVENT: &str = "shares-changed";
/// Extension of a share file (and of its cached copy).
pub const FILE_EXTENSION: &str = "portolano-share";
/// Folder of the cached copies, inside the data directory.
const CACHE_DIR: &str = "shares";
/// How often every share is synced while the app runs.
const SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Previews waiting for `share_subscribe`; the oldest is dropped beyond this.
const MAX_PREVIEWS: usize = 8;
const MAX_NAME_CHARS: usize = 200;
const FOLDER_DELETED: &str = "folder deleted";

/// Message key the UI recognises when a folder that holds shared folders is deleted.
pub const DELETE_CONTAINS_SHARES: &str = "share.deleteContainsShares";

pub fn read_only() -> AppError {
    AppError::new(
        ErrorCode::ReadOnly,
        "This item belongs to a shared folder and cannot be changed here.",
    )
}

/// Why a share needs attention, for the icon next to it in the tree. Host confirmations are
/// counted apart (`pending_targets`): they are not a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShareStatus {
    Ok,
    /// The file could not be reached or read: the last good copy stays in use.
    Offline,
    /// The file was read but refused by a check (signature, owner, revision, content).
    Rejected,
    /// Anything else that went wrong (the copy on this computer, writing the file).
    Error,
    /// The owner's local folder was deleted.
    FolderDeleted,
}

/// What the UI shows about one share.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareSummary {
    /// The link id (not the share id).
    pub id: Uuid,
    pub name: String,
    pub role: ShareRole,
    /// The mounted root (subscriber) or the published folder (owner), as the merged tree names it.
    pub root_node_id: Option<Uuid>,
    pub owner_name: String,
    pub fingerprint: String,
    pub revision: u64,
    pub signer_name: Option<String>,
    pub published_at: Option<String>,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
    pub status: ShareStatus,
    /// Connections whose target is new or changed and not confirmed yet.
    pub pending_targets: u32,
    /// Those connections, one entry each (what the user is asked to confirm).
    pub pending_hosts: Vec<PendingHost>,
    pub location: Option<ShareLocation>,
    pub publish_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteNodeInfo {
    pub link_id: Uuid,
    pub is_root: bool,
}

/// `doc_get`: the merged document plus what the UI needs to treat shared nodes differently.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    pub schema_version: u32,
    pub nodes: Vec<Node>,
    pub shares: Vec<ShareSummary>,
    pub remote_nodes: BTreeMap<Uuid, RemoteNodeInfo>,
}

impl DocumentView {
    #[cfg(test)]
    pub fn node(&self, id: Uuid) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id() == id)
    }
}

/// What `share_preview` shows before the user subscribes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePreview {
    pub preview_id: Uuid,
    pub name: String,
    pub owner_name: String,
    pub fingerprint: String,
    pub revision: u64,
    /// Folders in the share, the root included.
    pub folders: u32,
    pub connections: u32,
    /// Names of the connections that were left out (local shells and the like).
    pub skipped: Vec<String>,
    /// Names of the connections whose unsafe settings were switched off (certificate check,
    /// NLA, clipboard sharing, Kerberos KDC).
    pub adjusted: Vec<String>,
    /// Every connection the share holds, with its target: subscribing confirms all of them, so
    /// the user must have seen them. `node_id` is the id inside the share.
    pub hosts: Vec<PendingHost>,
}

/// A sanitised share kept in memory, with what the summary needs.
struct Loaded {
    nodes: SanitizedNodes,
    signer_name: String,
    published_at: String,
}

struct Preview {
    id: Uuid,
    location: ShareLocation,
    validator: Validator,
    share: SignedShare,
    outcome: SanitizeOutcome,
}

#[derive(Default)]
struct Inner {
    caches_loaded: bool,
    /// Subscribed shares by link id.
    mounts: BTreeMap<Uuid, Loaded>,
    /// What identified the last accepted copy of a link (not kept across restarts).
    validators: BTreeMap<Uuid, Validator>,
    /// When a link was last checked without a change (shown as its last sync).
    checked: BTreeMap<Uuid, String>,
    /// What kind of failure `last_error` is (not kept across restarts: a remembered error
    /// counts as [`ShareStatus::Error`] until the next sync says more).
    error_kinds: BTreeMap<Uuid, ShareStatus>,
    /// One gate per link, so a share is never synced twice at once while a slow or dead
    /// source does not hold up the others.
    sync_gates: BTreeMap<Uuid, Arc<tokio::sync::Mutex<()>>>,
    previews: VecDeque<Preview>,
}

/// A file that passed the checks, ready to be mounted.
struct Update {
    share: SignedShare,
    outcome: SanitizeOutcome,
}

enum Checked {
    /// The same revision that is already mounted.
    Same,
    New(Box<Update>),
}

pub struct ShareService {
    data_dir: PathBuf,
    store: Arc<Mutex<DocumentStore>>,
    vault: Arc<Mutex<Vault>>,
    notifier: Arc<Notifier>,
    inner: Mutex<Inner>,
    /// One sync at a time: the periodic one and the ones asked for by the user.
    /// One publication at a time (signing needs the vault, which is never locked under the
    /// store): together with the re-check in the final critical section it keeps a folder from
    /// getting two owner links.
    publish_gate: Mutex<()>,
    stop: tokio::sync::Notify,
}

fn clean_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect()
}

fn named(text: &str) -> String {
    let cleaned = clean_text(text);
    if cleaned.trim().is_empty() {
        "Unknown".to_owned()
    } else {
        cleaned
    }
}

fn root_name(outcome: &SanitizeOutcome) -> String {
    outcome
        .nodes
        .iter()
        .find(|n| n.id() == outcome.root)
        .map(|n| n.name().to_owned())
        .unwrap_or_default()
}

fn transport_error(e: &TransportError) -> AppError {
    let code = match e {
        TransportError::NotHttps
        | TransportError::InvalidAddress
        | TransportError::TooLarge
        | TransportError::InsecureRedirect => ErrorCode::InvalidInput,
        TransportError::FileUnavailable => ErrorCode::NotFound,
        _ => ErrorCode::HostUnreachable,
    };
    AppError::new(code, e.to_string())
}

fn share_error(e: &ShareError) -> AppError {
    let code = match e {
        ShareError::NoRandomSource => ErrorCode::Internal,
        _ => ErrorCode::InvalidInput,
    };
    AppError::new(code, e.to_string())
}

fn pinned_of(link: &ShareLink) -> Option<Pinned> {
    Some(Pinned {
        share_id: link.share_id,
        owner_key: decode_key(&link.owner_key)?,
        last_revision: link.last_revision,
        editors_version: link.editors_version,
    })
}

/// Decodes, checks against what the link trusts and sanitises. The error is the text for
/// `last_error`.
///
/// `cached_hash` is the payload hash of the copy kept for the mount, when there is one: the same
/// revision with other content is refused.
fn check_update(
    link: &ShareLink,
    bytes: &[u8],
    has_mount: bool,
    cached_hash: Option<[u8; 32]>,
) -> Result<Checked, String> {
    let share = decode(bytes).map_err(|e| e.to_string())?;
    let pinned = pinned_of(link)
        .ok_or_else(|| "The owner of this shared folder cannot be verified.".to_owned())?;
    // Without a mounted copy (cache lost) the same revision is fine, like at startup.
    let mode = if has_mount {
        AcceptMode::Update
    } else {
        AcceptMode::Cached
    };
    match accept(&pinned, &share, mode) {
        Ok(()) => {}
        // The same file again (for example after a restart, when nothing remembers its
        // validator) is not a rollback.
        Err(Rejection::OldRevision) if share.payload.revision == link.last_revision => {
            return match cached_hash {
                Some(hash) if hash != share.payload_sha256 => Err(CONTENT_CHANGED.to_owned()),
                _ => Ok(Checked::Same),
            };
        }
        Err(e) => return Err(e.to_string()),
    }
    // Cached mode (nothing mounted) has no cache to compare with, so only the branch above
    // can see the same revision carrying other content.
    let outcome = sanitize(share.payload.nodes.clone()).map_err(|e| e.to_string())?;
    Ok(Checked::New(Box::new(Update { share, outcome })))
}

const CONTENT_CHANGED: &str = "The shared folder changed without a new revision. Update refused.";

fn count_kinds(nodes: &[Node]) -> (u32, u32) {
    let folders = nodes.iter().filter(|n| n.is_folder()).count();
    let connections = nodes.len() - folders;
    (
        u32::try_from(folders).unwrap_or(u32::MAX),
        u32::try_from(connections).unwrap_or(u32::MAX),
    )
}

/// `2026-11-20T09:30:00Z` for the current time.
fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (i64::try_from(secs / 86_400).unwrap_or(0), secs % 86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

impl ShareService {
    pub fn new(
        data_dir: PathBuf,
        store: Arc<Mutex<DocumentStore>>,
        vault: Arc<Mutex<Vault>>,
        notifier: Arc<Notifier>,
    ) -> Self {
        Self {
            data_dir,
            store,
            vault,
            notifier,
            inner: Mutex::new(Inner::default()),
            publish_gate: Mutex::new(()),
            stop: tokio::sync::Notify::new(),
        }
    }

    fn inner(&self) -> Result<MutexGuard<'_, Inner>, AppError> {
        self.inner.lock().map_err(poisoned)
    }

    fn cache_path(&self, link_id: Uuid) -> PathBuf {
        self.data_dir
            .join(CACHE_DIR)
            .join(format!("{link_id}.{FILE_EXTENSION}"))
    }

    /// The payload hash of the cached copy of a link, when there is a readable one.
    fn cached_payload_hash(&self, link_id: Uuid) -> Option<[u8; 32]> {
        let bytes = transport::read_limited(&self.cache_path(link_id)).ok()?;
        decode(&bytes).ok().map(|s| s.payload_sha256)
    }

    fn remove_cache(&self, link_id: Uuid) {
        if let Err(e) = std::fs::remove_file(self.cache_path(link_id)) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %e, "could not remove the cached copy of a share");
            }
        }
    }

    fn emit_changed(&self) {
        self.notifier.emit(CHANGED_EVENT, serde_json::Value::Null);
    }

    // ---- the store, with the cached copies loaded ----

    /// Runs `f` on the loaded store. The document is loaded on first use (a recovery from the
    /// backup is logged here, never file contents), then the cached copies of the subscribed
    /// shares are mounted once.
    pub(crate) fn with_store<T>(
        &self,
        f: impl FnOnce(&mut DocumentStore) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut store = self.store.lock().map_err(poisoned)?;
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
        self.load_caches(&mut store);
        f(&mut store)
    }

    /// Mounts the cached copy of every subscribed share (once). A copy that cannot be used is
    /// dropped and the link keeps a `last_error`; the next sync brings the share back.
    fn load_caches(&self, store: &mut DocumentStore) {
        let Ok(mut inner) = self.inner() else {
            return;
        };
        if inner.caches_loaded {
            return;
        }
        inner.caches_loaded = true;
        let mut failed: Vec<(Uuid, String)> = Vec::new();
        for link in store
            .document()
            .shares
            .iter()
            .filter(|l| l.role == ShareRole::Subscriber)
        {
            match self.load_cache(link) {
                Ok(loaded) => {
                    inner.mounts.insert(link.id, loaded);
                }
                Err(text) => failed.push((link.id, text)),
            }
        }
        for (id, _) in &failed {
            inner.error_kinds.insert(*id, ShareStatus::Error);
        }
        drop(inner);
        if failed.is_empty() {
            return;
        }
        let result = store.apply_batch(|doc| {
            for (id, text) in &failed {
                if let Some(link) = doc.shares.iter_mut().find(|l| l.id == *id) {
                    link.last_error = Some(text.clone());
                }
            }
            Ok(())
        });
        if let Err(e) = result {
            tracing::warn!(error = %e, "could not record unusable cached shares");
        }
    }

    fn load_cache(&self, link: &ShareLink) -> Result<Loaded, String> {
        let path = self.cache_path(link.id);
        if !path.exists() {
            return Err("The saved copy of this shared folder is missing.".to_owned());
        }
        let unusable =
            || "The saved copy of this shared folder could not be used and was removed.".to_owned();
        let result = (|| -> Result<Loaded, String> {
            let bytes = transport::read_limited(&path).map_err(|_| unusable())?;
            let share = decode(&bytes).map_err(|_| unusable())?;
            let pinned = pinned_of(link).ok_or_else(unusable)?;
            accept(&pinned, &share, AcceptMode::Cached).map_err(|_| unusable())?;
            let outcome = sanitize(share.payload.nodes.clone()).map_err(|_| unusable())?;
            Ok(Loaded {
                nodes: outcome.nodes,
                signer_name: named(&share.payload.signer_name),
                published_at: clean_text(&share.payload.published_at),
            })
        })();
        if result.is_err() {
            self.remove_cache(link.id);
        }
        result
    }

    // ---- the merged tree ----

    fn mounts_of(inner: &Inner, doc: &Document) -> Vec<Mount> {
        doc.shares
            .iter()
            .filter(|l| l.role == ShareRole::Subscriber)
            .filter_map(|link| {
                inner.mounts.get(&link.id).map(|loaded| Mount {
                    link: link.clone(),
                    nodes: loaded.nodes.clone(),
                })
            })
            .collect()
    }

    fn merged_with(inner: &Inner, doc: &Document) -> MergedView {
        let mut view = merge(doc, &Self::mounts_of(inner, doc));
        // Mount roots are shown after the local siblings and are not reorderable: whatever
        // order the owner had must not interleave them.
        for node in &mut view.document.nodes {
            if view.remote.get(&node.id()).is_some_and(|r| r.is_root) {
                node.set_sort_index(None);
            }
        }
        view
    }

    /// The local document plus every mounted share (see `portolano_shares::view::merge`).
    pub fn merged(&self, doc: &Document) -> Result<MergedView, AppError> {
        Ok(Self::merged_with(&*self.inner()?, doc))
    }

    /// The remote nodes of `doc`'s mounted shares by their ids in the merged tree.
    pub fn remote_map(&self, doc: &Document) -> Result<BTreeMap<Uuid, RemoteNode>, AppError> {
        let inner = self.inner()?;
        let mut map = BTreeMap::new();
        for link in doc
            .shares
            .iter()
            .filter(|l| l.role == ShareRole::Subscriber)
        {
            let Some(loaded) = inner.mounts.get(&link.id) else {
                continue;
            };
            for node in &loaded.nodes {
                map.insert(
                    derive_id(link.id, node.id()),
                    RemoteNode {
                        link_id: link.id,
                        source_id: node.id(),
                        is_root: node.parent_id().is_none(),
                    },
                );
            }
        }
        Ok(map)
    }

    fn summaries_with(inner: &Inner, doc: &Document, merged: &MergedView) -> Vec<ShareSummary> {
        doc.shares
            .iter()
            .map(|link| {
                let loaded = inner.mounts.get(&link.id);
                let root_node_id = match link.role {
                    ShareRole::Subscriber => loaded.and_then(|l| {
                        l.nodes
                            .iter()
                            .find(|n| n.parent_id().is_none())
                            .map(|n| derive_id(link.id, n.id()))
                    }),
                    ShareRole::Owner => link
                        .owned
                        .and_then(|o| doc.node(o.folder_id).map(|_| o.folder_id)),
                };
                let folder_gone = link.role == ShareRole::Owner
                    && link.owned.is_some_and(|o| doc.node(o.folder_id).is_none());
                let last_error = if folder_gone {
                    Some(FOLDER_DELETED.to_owned())
                } else {
                    link.last_error.clone()
                };
                let status = if folder_gone {
                    ShareStatus::FolderDeleted
                } else if last_error.is_some() {
                    inner
                        .error_kinds
                        .get(&link.id)
                        .copied()
                        .unwrap_or(ShareStatus::Error)
                } else {
                    ShareStatus::Ok
                };
                let (signer_name, published_at) = match (link.role, loaded) {
                    (ShareRole::Owner, _) => {
                        (Some(link.owner_name.clone()), link.last_sync.clone())
                    }
                    (_, Some(l)) => (Some(l.signer_name.clone()), Some(l.published_at.clone())),
                    _ => (None, None),
                };
                let last_sync = match (&link.last_sync, inner.checked.get(&link.id)) {
                    (Some(synced), Some(checked)) if checked > synced => Some(checked.clone()),
                    (None, Some(checked)) => Some(checked.clone()),
                    (synced, _) => synced.clone(),
                };
                ShareSummary {
                    id: link.id,
                    name: link.name.clone(),
                    role: link.role,
                    root_node_id,
                    owner_name: link.owner_name.clone(),
                    fingerprint: decode_key(&link.owner_key)
                        .map(|k| fingerprint(&k))
                        .unwrap_or_default(),
                    revision: link.last_revision,
                    signer_name,
                    published_at,
                    last_sync,
                    last_error,
                    status,
                    pending_targets: merged.pending.get(&link.id).copied().unwrap_or(0),
                    pending_hosts: merged
                        .pending_hosts
                        .get(&link.id)
                        .cloned()
                        .unwrap_or_default(),
                    location: link.location.clone(),
                    publish_path: link.publish_path.clone(),
                }
            })
            .collect()
    }

    pub fn summaries(&self, doc: &Document) -> Result<Vec<ShareSummary>, AppError> {
        let inner = self.inner()?;
        let merged = Self::merged_with(&inner, doc);
        Ok(Self::summaries_with(&inner, doc, &merged))
    }

    fn summary_of(&self, doc: &Document, link_id: Uuid) -> Result<ShareSummary, AppError> {
        self.summaries(doc)?
            .into_iter()
            .find(|s| s.id == link_id)
            .ok_or_else(|| AppError::not_found("This shared folder is not in the document."))
    }

    /// `doc_get`.
    pub fn document_view(&self, doc: &Document) -> Result<DocumentView, AppError> {
        let inner = self.inner()?;
        let merged = Self::merged_with(&inner, doc);
        let shares = Self::summaries_with(&inner, doc, &merged);
        Ok(DocumentView {
            schema_version: merged.document.schema_version,
            remote_nodes: merged
                .remote
                .iter()
                .map(|(id, r)| {
                    (
                        *id,
                        RemoteNodeInfo {
                            link_id: r.link_id,
                            is_root: r.is_root,
                        },
                    )
                })
                .collect(),
            nodes: merged.document.nodes,
            shares,
        })
    }

    // ---- guards ----

    /// Subscribed links whose mount point is `folder_id` or a local folder below it.
    pub fn links_under(&self, doc: &Document, folder_id: Uuid) -> Vec<Uuid> {
        let mut folders: BTreeSet<Uuid> = BTreeSet::from([folder_id]);
        loop {
            let before = folders.len();
            for node in &doc.nodes {
                if node.is_folder() && node.parent_id().is_some_and(|p| folders.contains(&p)) {
                    folders.insert(node.id());
                }
            }
            if folders.len() == before {
                break;
            }
        }
        doc.shares
            .iter()
            .filter(|l| l.role == ShareRole::Subscriber)
            .filter(|l| l.parent_id.is_some_and(|p| folders.contains(&p)))
            .map(|l| l.id)
            .collect()
    }

    /// Moves a mount root under another local folder (`None`: the root of the tree).
    pub fn move_mount_root(
        &self,
        store: &mut DocumentStore,
        root: RemoteNode,
        new_parent: Option<Uuid>,
    ) -> Result<(), AppError> {
        if !root.is_root {
            return Err(read_only());
        }
        if let Some(parent) = new_parent {
            match store.document().node(parent) {
                Some(node) if node.is_folder() => {}
                Some(_) => {
                    return Err(AppError::invalid_input(
                        "A shared folder can only be moved into a folder.",
                    ))
                }
                None => return Err(AppError::not_found("The destination folder was not found.")),
            }
        }
        store.apply_batch(|doc| {
            let link = doc
                .shares
                .iter_mut()
                .find(|l| l.id == root.link_id)
                .ok_or_else(|| ModelError::NotFound(root.link_id.to_string()))?;
            link.parent_id = new_parent;
            Ok(())
        })?;
        Ok(())
    }

    /// After a credential was deleted from the vault: local choices that pointed at it go.
    pub fn forget_credential(
        &self,
        store: &mut DocumentStore,
        credential_id: Uuid,
    ) -> Result<(), ModelError> {
        let gone = CredentialRef::Id { id: credential_id };
        let used = store
            .document()
            .shares
            .iter()
            .any(|l| l.overrides.values().any(|c| *c == gone));
        if !used {
            return Ok(());
        }
        store.apply_batch(|doc| {
            for link in &mut doc.shares {
                link.overrides.retain(|_, c| *c != gone);
            }
            Ok(())
        })
    }

    // ---- credentials and targets ----

    /// The user's own credential choice for a remote node (stored by its source id).
    pub fn set_credential(&self, node_id: Uuid, credential: CredentialRef) -> Result<(), AppError> {
        self.with_store(|store| {
            let remote = self.remote_map(store.document())?;
            let node = remote.get(&node_id).ok_or_else(|| {
                AppError::invalid_input("Only items of a shared folder have a local credential.")
            })?;
            store.apply_batch(|doc| {
                let link = doc
                    .shares
                    .iter_mut()
                    .find(|l| l.id == node.link_id)
                    .ok_or_else(|| ModelError::NotFound(node.link_id.to_string()))?;
                if credential == CredentialRef::Inherit {
                    link.overrides.remove(&node.source_id);
                } else {
                    link.overrides.insert(node.source_id, credential);
                }
                Ok(())
            })?;
            Ok(())
        })
    }

    /// Whether the connection `node_id` (an id of the merged tree) waits for its host to be
    /// confirmed, so no credential reaches it yet.
    pub fn is_pending_target(&self, node_id: Uuid) -> Result<bool, AppError> {
        self.with_store(|store| {
            Ok(self
                .summaries(store.document())?
                .iter()
                .any(|s| s.pending_hosts.iter().any(|h| h.node_id == node_id)))
        })
    }

    /// Accepts the current target of every connection of a share: from now on the credential
    /// choices (and inheritance) reach them.
    pub fn confirm_targets(&self, link_id: Uuid, revision: u64) -> Result<(), AppError> {
        self.with_store(|store| {
            // The user confirms what they saw: a newer version (a sync in between) is not it.
            let seen = store
                .document()
                .shares
                .iter()
                .find(|l| l.id == link_id)
                .map(|l| l.last_revision);
            if seen.is_some_and(|r| r != revision) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "The shared folder changed since you looked at it: review it again.",
                ));
            }
            let targets: BTreeMap<Uuid, _> = {
                let inner = self.inner()?;
                let loaded = inner
                    .mounts
                    .get(&link_id)
                    .ok_or_else(|| AppError::not_found("This shared folder is not available."))?;
                loaded
                    .nodes
                    .iter()
                    .filter_map(|n| match n {
                        Node::Connection(c) => target_of(c).map(|t| (c.id, t)),
                        Node::Folder(_) => None,
                    })
                    .collect()
            };
            store.apply_batch(|doc| {
                let link = doc
                    .shares
                    .iter_mut()
                    .find(|l| l.id == link_id)
                    .ok_or_else(|| ModelError::NotFound(link_id.to_string()))?;
                link.confirmed_targets = targets;
                Ok(())
            })?;
            Ok(())
        })
    }

    // ---- previews and subscriptions ----

    pub async fn preview(&self, location: ShareLocation) -> Result<SharePreview, AppError> {
        let fetched = transport::fetch(&location, &Validator::None)
            .await
            .map_err(|e| transport_error(&e))?;
        let Fetched::Bytes { data, validator } = fetched else {
            return Err(AppError::internal("the share was not read"));
        };
        // Decoding (signature) and sanitising cost CPU on a file up to 5 MiB: not on the runtime.
        let (share, outcome) = tauri::async_runtime::spawn_blocking(move || {
            let share = decode(&data).map_err(|e| share_error(&e))?;
            first_pin(&share).map_err(|e| AppError::invalid_input(e.to_string()))?;
            let outcome = sanitize(share.payload.nodes.clone()).map_err(|e| share_error(&e))?;
            Ok::<_, AppError>((share, outcome))
        })
        .await
        .map_err(|_| AppError::internal("the share could not be read"))??;
        let owner = decode_key(&share.payload.owner.key).ok_or_else(|| {
            AppError::invalid_input("The owner of this shared folder is invalid.")
        })?;
        let (folders, connections) = count_kinds(&outcome.nodes);
        let preview = SharePreview {
            preview_id: Uuid::new_v4(),
            name: root_name(&outcome),
            owner_name: named(&share.payload.owner.name),
            fingerprint: fingerprint(&owner),
            revision: share.payload.revision,
            folders,
            connections,
            skipped: outcome.skipped.clone(),
            adjusted: outcome.adjusted.clone(),
            hosts: outcome
                .nodes
                .iter()
                .filter_map(|n| match n {
                    Node::Connection(c) => Some(pending_host(c.id, c)),
                    Node::Folder(_) => None,
                })
                .collect(),
        };
        let mut inner = self.inner()?;
        inner.previews.push_back(Preview {
            id: preview.preview_id,
            location,
            validator,
            share,
            outcome,
        });
        while inner.previews.len() > MAX_PREVIEWS {
            inner.previews.pop_front();
        }
        Ok(preview)
    }

    pub fn subscribe(
        &self,
        preview_id: Uuid,
        parent_id: Option<Uuid>,
    ) -> Result<ShareSummary, AppError> {
        let (share, outcome, location, validator) = {
            let inner = self.inner()?;
            let preview = inner
                .previews
                .iter()
                .find(|p| p.id == preview_id)
                .ok_or_else(|| {
                    AppError::not_found("The preview was not found or has expired: open it again.")
                })?;
            (
                preview.share.clone(),
                preview.outcome.clone(),
                preview.location.clone(),
                preview.validator.clone(),
            )
        };
        let payload = &share.payload;
        // The user saw this share in the preview: its current targets are accepted.
        let confirmed_targets = outcome
            .nodes
            .iter()
            .filter_map(|n| match n {
                Node::Connection(c) => target_of(c).map(|t| (c.id, t)),
                Node::Folder(_) => None,
            })
            .collect();
        let link = ShareLink {
            id: Uuid::new_v4(),
            share_id: payload.share_id,
            name: root_name(&outcome),
            role: ShareRole::Subscriber,
            location: Some(location),
            publish_path: None,
            parent_id,
            owner_name: named(&payload.owner.name),
            owner_key: payload.owner.key.clone(),
            last_revision: payload.revision,
            editors_version: payload.editors.version,
            last_sync: Some(now_rfc3339()),
            last_error: None,
            overrides: BTreeMap::new(),
            confirmed_targets,
            owned: None,
        };
        let summary = self.with_store(|store| {
            let doc = store.document();
            if let Some(parent) = parent_id {
                if self.remote_map(doc)?.contains_key(&parent) {
                    return Err(read_only());
                }
                match doc.node(parent) {
                    Some(node) if node.is_folder() => {}
                    Some(_) => {
                        return Err(AppError::invalid_input(
                            "A shared folder can only be added into a folder.",
                        ))
                    }
                    None => {
                        return Err(AppError::not_found("The destination folder was not found."))
                    }
                }
            }
            if doc.shares.iter().any(|l| l.share_id == link.share_id) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "You already have this shared folder.",
                ));
            }
            let path = self.cache_path(link.id);
            write_private_atomic(&path, &share.raw).map_err(|_| {
                AppError::new(ErrorCode::Io, "The shared folder could not be saved.")
            })?;
            let saved = store.apply_batch(|doc| {
                doc.shares.push(link.clone());
                Ok(())
            });
            if let Err(e) = saved {
                self.remove_cache(link.id);
                return Err(e.into());
            }
            let mut inner = self.inner()?;
            inner.mounts.insert(
                link.id,
                Loaded {
                    nodes: outcome.nodes.clone(),
                    signer_name: named(&payload.signer_name),
                    published_at: clean_text(&payload.published_at),
                },
            );
            inner.validators.insert(link.id, validator);
            inner.previews.retain(|p| p.id != preview_id);
            drop(inner);
            self.summary_of(store.document(), link.id)
        })?;
        Ok(summary)
    }

    /// Removes the link (a subscribed share disappears from the tree; for an owner, the folder
    /// stays and the file is left where it is) and the cached copy.
    pub fn unsubscribe(&self, link_id: Uuid) -> Result<(), AppError> {
        self.with_store(|store| {
            if !store.document().shares.iter().any(|l| l.id == link_id) {
                return Err(AppError::not_found(
                    "This shared folder is not in the document.",
                ));
            }
            store.apply_batch(|doc| {
                doc.shares.retain(|l| l.id != link_id);
                Ok(())
            })?;
            let mut inner = self.inner()?;
            inner.mounts.remove(&link_id);
            inner.validators.remove(&link_id);
            inner.checked.remove(&link_id);
            inner.error_kinds.remove(&link_id);
            inner.sync_gates.remove(&link_id);
            Ok(())
        })?;
        self.remove_cache(link_id);
        Ok(())
    }

    // ---- publishing ----

    /// Signs `nodes` with the vault identity `identity` (the only one when `None`). Needs the
    /// unlocked vault; nothing is written here, so a failure leaves everything as it was.
    fn sign_share(
        &self,
        identity: Option<Uuid>,
        share_id: Uuid,
        revision: u64,
        editors_version: u64,
        nodes: Vec<Node>,
    ) -> Result<(Vec<u8>, IdentityRecord), AppError> {
        let (record, secret) = {
            let vault = lock_vault_mutex(&self.vault)?;
            let record = vault
                .identities()?
                .into_iter()
                .find(|r| identity.is_none_or(|id| r.id == id))
                .ok_or_else(|| {
                    AppError::not_found("There is no signing identity: create one first.")
                })?;
            let secret = vault.identity_secret(record.id)?;
            (record, secret)
        };
        let owner_key = VerifyingKey::from_bytes(&record.public_key)
            .map_err(|_| AppError::internal("the identity key is not valid"))?;
        // The signing key only exists for the signature: dropped right after.
        let signer = Identity::from_secret(record.id, &record.name, &secret);
        drop(secret);
        let keys: Vec<KeyEntry> = Vec::new();
        let payload = SharePayload {
            share_id,
            name: nodes
                .iter()
                .find(|n| n.parent_id().is_none())
                .map(|n| n.name().to_owned())
                .unwrap_or_default(),
            revision,
            published_at: now_rfc3339(),
            signer_name: record.name.clone(),
            owner: KeyEntry::new(&record.name, &owner_key),
            editors: EditorList {
                version: editors_version,
                owner_signature: sign_editors(share_id, editors_version, &keys, &signer),
                keys,
            },
            nodes,
        };
        let bytes = match encode(&payload, &signer) {
            Ok(bytes) => bytes,
            Err(ShareError::TooLarge) => {
                return Err(AppError::invalid_input(
                    "This folder is too large to share (5 MiB limit).",
                ))
            }
            Err(e) => return Err(share_error(&e)),
        };
        Ok((bytes, record))
    }

    fn write_share_file(path: &str, bytes: &[u8]) -> Result<(), AppError> {
        transport::write_atomic(std::path::Path::new(path), bytes)
            .map_err(|_| AppError::new(ErrorCode::Io, "The share file could not be written."))
    }

    /// The current content of a file that is about to be overwritten; `None` when it cannot be
    /// read or is too big to be a share.
    fn read_previous_share_file(path: &str) -> Option<Vec<u8>> {
        let path = std::path::Path::new(path);
        let size = std::fs::metadata(path).ok()?.len();
        if size > portolano_shares::format::MAX_SHARE_BYTES as u64 {
            return None;
        }
        std::fs::read(path).ok()
    }

    /// Fails when the folder no longer holds what was signed (it was edited while the vault was
    /// signing): publishing the old content would silently drop that edit.
    fn unchanged_since_signing(
        doc: &Document,
        folder_id: Uuid,
        signed: &[Node],
    ) -> Result<(), AppError> {
        let current = prepare_for_publish(doc, folder_id).map_err(|e| share_error(&e))?;
        if current != signed {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "The folder changed while it was being published. Try again.",
            ));
        }
        Ok(())
    }

    /// First publication of the local folder `folder_id` to `path`.
    pub fn publish(&self, folder_id: Uuid, path: &str) -> Result<ShareSummary, AppError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(AppError::invalid_input(
                "Choose where to save the share file.",
            ));
        }
        let _one_at_a_time = self.publish_gate.lock().map_err(poisoned)?;
        // The checks that can fail the user early; they are repeated, together with the
        // insert of the link, in the one critical section at the end.
        let check = |doc: &Document| -> Result<(), AppError> {
            if self.remote_map(doc)?.contains_key(&folder_id) {
                return Err(read_only());
            }
            match doc.node(folder_id) {
                Some(node) if node.is_folder() => {}
                Some(_) => return Err(AppError::invalid_input("Only a folder can be shared.")),
                None => return Err(AppError::not_found("The folder was not found.")),
            }
            if doc
                .shares
                .iter()
                .any(|l| l.owned.is_some_and(|o| o.folder_id == folder_id))
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This folder is already shared.",
                ));
            }
            Ok(())
        };
        let nodes = self.with_store(|store| {
            let doc = store.document();
            check(doc)?;
            // The local document: mounted shares below this folder are not published again.
            prepare_for_publish(doc, folder_id).map_err(|e| share_error(&e))
        })?;
        let share_id = Uuid::new_v4();
        // Signing runs without the store lock: what was signed is compared again under it.
        let signed = nodes.clone();
        let (bytes, record) = self.sign_share(None, share_id, 1, 1, nodes)?;
        let owner_key = portolano_shares::identity::encode_key(
            &VerifyingKey::from_bytes(&record.public_key)
                .map_err(|_| AppError::internal("the identity key is not valid"))?,
        );
        // Re-check and insert under one lock, before the file exists: a folder deleted or
        // shared meanwhile leaves nothing behind.
        let link_id = Uuid::new_v4();
        let existed = std::path::Path::new(path).exists();
        // What an existing file held, so that a failed save of the link can put it back.
        let previous = if existed {
            Self::read_previous_share_file(path)
        } else {
            None
        };
        let summary = self.with_store(|store| {
            check(store.document())?;
            Self::unchanged_since_signing(store.document(), folder_id, &signed)?;
            let name = store
                .document()
                .node(folder_id)
                .map(|n| n.name().to_owned())
                .unwrap_or_default();
            Self::write_share_file(path, &bytes)?;
            let link = ShareLink {
                id: link_id,
                share_id,
                name,
                role: ShareRole::Owner,
                location: None,
                publish_path: Some(path.to_owned()),
                parent_id: None,
                owner_name: record.name.clone(),
                owner_key: owner_key.clone(),
                last_revision: 1,
                editors_version: 1,
                last_sync: Some(now_rfc3339()),
                last_error: None,
                overrides: BTreeMap::new(),
                confirmed_targets: BTreeMap::new(),
                owned: Some(OwnedShare {
                    folder_id,
                    identity_id: record.id,
                }),
            };
            let saved = store.apply_batch(|doc| {
                doc.shares.push(link);
                Ok(())
            });
            if let Err(e) = saved {
                // The link could not be saved: do not leave the file we just wrote (a new file
                // is removed, an overwritten one gets its old content back).
                if !existed {
                    if let Err(rm) = std::fs::remove_file(path) {
                        tracing::warn!(error = %rm, "could not remove an unused share file");
                    }
                } else if let Some(old) = &previous {
                    if transport::write_atomic(std::path::Path::new(path), old).is_err() {
                        tracing::warn!("could not restore the previous share file");
                    }
                } else {
                    tracing::warn!("the overwritten share file could not be read, so not restored");
                }
                return Err(e.into());
            }
            self.summary_of(store.document(), link_id)
        })?;
        Ok(summary)
    }

    /// Publishes the current content of an owned folder again: revision + 1, same path.
    pub fn republish(&self, link_id: Uuid) -> Result<ShareSummary, AppError> {
        let (link, owned, path, nodes) = self.with_store(|store| {
            let doc = store.document();
            let link = doc
                .shares
                .iter()
                .find(|l| l.id == link_id)
                .ok_or_else(|| AppError::not_found("This shared folder is not in the document."))?
                .clone();
            let (Some(owned), Some(path)) = (link.owned, link.publish_path.clone()) else {
                return Err(AppError::invalid_input(
                    "You do not own this shared folder.",
                ));
            };
            if doc.node(owned.folder_id).is_none() {
                return Err(AppError::not_found("The shared folder was deleted."));
            }
            let nodes = prepare_for_publish(doc, owned.folder_id).map_err(|e| share_error(&e))?;
            Ok((link, owned, path, nodes))
        })?;
        let revision = link
            .last_revision
            .checked_add(1)
            .ok_or_else(|| AppError::internal("the revision cannot grow any more"))?;
        let signed = nodes.clone();
        let (bytes, record) = self.sign_share(
            Some(owned.identity_id),
            link.share_id,
            revision,
            link.editors_version,
            nodes,
        )?;
        // The revision is saved first: if the file cannot be written the number is skipped,
        // which is harmless, while a file with a revision the link does not know could be
        // published twice with different content.
        self.with_store(|store| {
            Self::unchanged_since_signing(store.document(), owned.folder_id, &signed)?;
            store.apply_batch(|doc| {
                if let Some(l) = doc.shares.iter_mut().find(|l| l.id == link_id) {
                    l.last_revision = revision;
                    l.last_sync = Some(now_rfc3339());
                    l.last_error = None;
                    l.owner_name = record.name.clone();
                }
                Ok(())
            })?;
            self.inner()?.error_kinds.remove(&link_id);
            Ok(())
        })?;
        if let Err(e) = Self::write_share_file(&path, &bytes) {
            self.set_error(link_id, ShareStatus::Error, &e.message);
            return Err(e);
        }
        self.with_store(|store| self.summary_of(store.document(), link_id))
    }

    // ---- syncing ----

    /// Syncs one subscribed share (`Some`) or all of them (`None`). A share that cannot be
    /// reached or fails a check keeps what it had and gets a `last_error`; that is not an
    /// error of this call. Returns the summaries of the shares that were synced.
    pub async fn sync(
        self: &Arc<Self>,
        link_id: Option<Uuid>,
    ) -> Result<Vec<ShareSummary>, AppError> {
        let ids = self.with_store(|store| {
            let links = &store.document().shares;
            match link_id {
                Some(id) => {
                    if links.iter().any(|l| l.id == id) {
                        Ok(vec![id])
                    } else {
                        Err(AppError::not_found(
                            "This shared folder is not in the document.",
                        ))
                    }
                }
                None => Ok(links
                    .iter()
                    .filter(|l| l.role == ShareRole::Subscriber && l.location.is_some())
                    .map(|l| l.id)
                    .collect()),
            }
        })?;
        // All at once: a share on a server that does not answer only delays itself.
        let mut tasks = tokio::task::JoinSet::new();
        for id in ids.iter().copied() {
            let service = Arc::clone(self);
            tasks.spawn(async move { service.sync_gated(id).await });
        }
        let mut changed = false;
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(c)) => changed |= c,
                Ok(Err(e)) => {
                    tracing::warn!(code = e.code.as_str(), "share sync failed: {}", e.message)
                }
                Err(e) => tracing::warn!("share sync task ended abnormally: {e}"),
            }
        }
        if changed {
            self.emit_changed();
        }
        self.with_store(|store| {
            let all = self.summaries(store.document())?;
            Ok(all.into_iter().filter(|s| ids.contains(&s.id)).collect())
        })
    }

    /// `sync_one` behind the link's gate (a manual sync waits for the periodic one).
    async fn sync_gated(&self, id: Uuid) -> Result<bool, AppError> {
        let gate = Arc::clone(self.inner()?.sync_gates.entry(id).or_default());
        let _held = gate.lock().await;
        self.sync_one(id).await
    }

    /// Whether anything the UI shows changed.
    async fn sync_one(&self, id: Uuid) -> Result<bool, AppError> {
        let (link, previous, has_mount) = self
            .with_store(|store| {
                let Some(link) = store.document().shares.iter().find(|l| l.id == id) else {
                    return Ok(None);
                };
                let inner = self.inner()?;
                Ok(Some((
                    link.clone(),
                    inner.validators.get(&id).cloned().unwrap_or_default(),
                    inner.mounts.contains_key(&id),
                )))
            })?
            .ok_or_else(|| AppError::not_found("This shared folder is not in the document."))?;
        if link.role != ShareRole::Subscriber {
            return Ok(false);
        }
        let Some(location) = link.location.clone() else {
            return Ok(false);
        };
        let fetched = match transport::fetch(&location, &previous).await {
            Ok(fetched) => fetched,
            Err(e) => {
                let kind = if e.is_offline() {
                    ShareStatus::Offline
                } else {
                    ShareStatus::Error
                };
                return self.record_error(id, kind, &e.to_string());
            }
        };
        let (data, validator) = match fetched {
            Fetched::NotModified => {
                self.mark_checked(id)?;
                return Ok(false);
            }
            Fetched::Bytes { data, validator } => (data, validator),
        };
        let cached_hash = if has_mount {
            self.cached_payload_hash(id)
        } else {
            None
        };
        match check_update(&link, &data, has_mount, cached_hash) {
            Err(text) => self.record_error(id, ShareStatus::Rejected, &text),
            Ok(Checked::Same) => self.record_same(id, validator, link.last_error.is_some()),
            Ok(Checked::New(update)) => self.apply_update(id, *update, validator),
        }
    }

    fn mark_checked(&self, id: Uuid) -> Result<(), AppError> {
        self.inner()?.checked.insert(id, now_rfc3339());
        Ok(())
    }

    /// The same revision again: nothing to mount, but a recovered share loses its error.
    fn record_same(
        &self,
        id: Uuid,
        validator: Validator,
        had_error: bool,
    ) -> Result<bool, AppError> {
        {
            let mut inner = self.inner()?;
            inner.validators.insert(id, validator);
            inner.checked.insert(id, now_rfc3339());
            inner.error_kinds.remove(&id);
        }
        if !had_error {
            return Ok(false);
        }
        self.with_store(|store| {
            store.apply_batch(|doc| {
                if let Some(link) = doc.shares.iter_mut().find(|l| l.id == id) {
                    link.last_error = None;
                    link.last_sync = Some(now_rfc3339());
                }
                Ok(())
            })?;
            Ok(true)
        })
    }

    /// Keeps the cache and the mounted tree, remembers why the sync failed.
    fn record_error(&self, id: Uuid, kind: ShareStatus, text: &str) -> Result<bool, AppError> {
        let kind_changed = {
            let mut inner = self.inner()?;
            inner.validators.remove(&id);
            inner.error_kinds.insert(id, kind) != Some(kind)
        };
        self.with_store(|store| {
            let current = store
                .document()
                .shares
                .iter()
                .find(|l| l.id == id)
                .map(|l| l.last_error.clone());
            let Some(current) = current else {
                return Ok(false);
            };
            if current.as_deref() == Some(text) {
                return Ok(kind_changed);
            }
            store.apply_batch(|doc| {
                if let Some(link) = doc.shares.iter_mut().find(|l| l.id == id) {
                    link.last_error = Some(text.to_owned());
                }
                Ok(())
            })?;
            Ok(true)
        })
    }

    /// Best effort `record_error` for paths that already return their own error.
    fn set_error(&self, id: Uuid, kind: ShareStatus, text: &str) {
        if let Err(e) = self.record_error(id, kind, text) {
            tracing::warn!(code = e.code.as_str(), "could not record a share error");
        }
    }

    fn apply_update(
        &self,
        id: Uuid,
        update: Update,
        validator: Validator,
    ) -> Result<bool, AppError> {
        let payload = &update.share.payload;
        // The cached copy first: if it cannot be saved, nothing else changes.
        if write_private_atomic(&self.cache_path(id), &update.share.raw).is_err() {
            return self.record_error(
                id,
                ShareStatus::Error,
                "The new version could not be saved on this computer.",
            );
        }
        let kept: BTreeSet<Uuid> = update.outcome.nodes.iter().map(Node::id).collect();
        let name = root_name(&update.outcome);
        let applied = self.with_store(|store| {
            let exists = store.document().shares.iter().any(|l| l.id == id);
            if !exists {
                return Ok(false);
            }
            store.apply_batch(|doc| {
                if let Some(link) = doc.shares.iter_mut().find(|l| l.id == id) {
                    link.name = name.clone();
                    link.last_revision = payload.revision;
                    link.editors_version = payload.editors.version;
                    link.last_sync = Some(now_rfc3339());
                    link.last_error = None;
                    // Choices about nodes that no longer exist are dropped.
                    link.overrides.retain(|node, _| kept.contains(node));
                    link.confirmed_targets.retain(|node, _| kept.contains(node));
                }
                Ok(())
            })?;
            let mut inner = self.inner()?;
            inner.mounts.insert(
                id,
                Loaded {
                    nodes: update.outcome.nodes.clone(),
                    signer_name: named(&payload.signer_name),
                    published_at: clean_text(&payload.published_at),
                },
            );
            inner.validators.insert(id, validator);
            inner.checked.remove(&id);
            inner.error_kinds.remove(&id);
            Ok(true)
        })?;
        if !applied {
            // Unsubscribed while the file was being fetched.
            self.remove_cache(id);
        }
        Ok(applied)
    }

    // ---- background sync ----

    /// Syncs everything at start-up and then every 15 minutes, until [`ShareService::shutdown`].
    /// Never blocks the caller.
    pub fn spawn_periodic(self: &Arc<Self>) {
        let service = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    () = service.stop.notified() => break,
                    result = service.sync(None) => {
                        if let Err(e) = result {
                            tracing::warn!(code = e.code.as_str(), "periodic share sync failed: {}", e.message);
                        }
                    }
                }
                tokio::select! {
                    () = service.stop.notified() => break,
                    () = tokio::time::sleep(SYNC_INTERVAL) => {}
                }
            }
        });
    }

    /// Stops the periodic task (application exit).
    pub fn shutdown(&self) {
        self.stop.notify_one();
    }
}
