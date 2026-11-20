//! Portolano's own export: `{ "format": "portolano-export", "version": 1, "nodes": [...] }`.
//!
//! Import remaps every id (the source ids mean nothing in the target document), drops jump hosts
//! that point outside the file and any password-like key. The file itself is untrusted: before
//! the preview is shown `Draft::sanitize` turns every credential reference into `None`, resets
//! `ignoreCert` and "allow without NLA", and skips local shells with a custom program. Export writes the same format with the
//! same scrubbing, so an export contains no credential ids and no jump host outside the set.

use std::collections::{HashMap, HashSet};

use portolano_core_model::appearance;
use portolano_core_model::{Connection, ConnectionOptions, CredentialRef, Folder, Node};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::draft::{name_or, Draft, PASSWORD_IGNORED};
use crate::{ImportError, MAX_ENTRIES};

pub(crate) const FORMAT: &str = "portolano-export";
pub(crate) const VERSION: u32 = 1;

/// The file as read. Serde's own error text is never shown: it can quote values.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile {
    #[allow(dead_code)] // checked on the untyped value first
    format: String,
    #[allow(dead_code)] // checked on the untyped value first
    version: u32,
    nodes: Vec<Node>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportOut<'a> {
    format: &'static str,
    version: u32,
    nodes: &'a [Node],
}

pub(crate) fn parse(text: &str) -> Result<Draft, ImportError> {
    let invalid = |e: serde_json::Error| {
        ImportError::InvalidInput(format!(
            "not a valid Portolano export (line {}, column {})",
            e.line(),
            e.column()
        ))
    };
    let value: serde_json::Value = serde_json::from_str(text).map_err(invalid)?;
    if value.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
        return Err(ImportError::InvalidInput(
            "not a Portolano export file".into(),
        ));
    }
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(VERSION) => {}
        Some(v) if v > u64::from(VERSION) => {
            return Err(ImportError::Unsupported(
                "this export was written by a newer version of Portolano".into(),
            ))
        }
        _ => {
            return Err(ImportError::InvalidInput(
                "unknown Portolano export version".into(),
            ))
        }
    }
    let file: ExportFile = serde_json::from_str(text).map_err(invalid)?;
    if file.nodes.len() > MAX_ENTRIES {
        return Err(ImportError::InvalidInput(format!(
            "the file has more than {MAX_ENTRIES} entries"
        )));
    }

    let mut draft = Draft::new();
    scan_for_passwords(&value, &mut draft);

    let mut ids: HashMap<Uuid, Uuid> = HashMap::with_capacity(file.nodes.len());
    for node in &file.nodes {
        if ids.insert(node.id(), Uuid::new_v4()).is_some() {
            return Err(ImportError::InvalidInput(
                "the export contains a duplicate node id".into(),
            ));
        }
    }

    for node in file.nodes {
        let new_id = ids.get(&node.id()).copied().unwrap_or_else(Uuid::new_v4);
        let parent = match node.parent_id() {
            None => None,
            Some(old) => match ids.get(&old) {
                Some(new) => Some(*new),
                None => {
                    draft.warn(
                        None,
                        "an entry's parent folder is not in the file: it was imported at the root",
                    );
                    None
                }
            },
        };
        let node = match node {
            Node::Folder(folder) => Node::Folder(Folder {
                sort_index: folder.sort_index,
                icon: appearance::sanitize_icon(folder.icon),
                color: appearance::sanitize_color(folder.color),
                id: new_id,
                parent_id: parent,
                name: name_or(&folder.name, "folder"),
                credential: remap_credential(folder.credential, &mut draft),
            }),
            Node::Connection(conn) => {
                let credential = remap_credential(conn.credential, &mut draft);
                let options = remap_options(conn.options, &ids, &mut draft);
                Node::Connection(Connection {
                    icon: appearance::sanitize_icon(conn.icon.clone()),
                    color: appearance::sanitize_color(conn.color.clone()),
                    id: new_id,
                    parent_id: parent,
                    name: name_or(&conn.name, &conn.host),
                    credential,
                    options,
                    ..conn
                })
            }
        };
        draft.push(node, None)?;
    }
    draft.break_jump_cycles();
    Ok(draft)
}

/// Credential references are neutralised by `Draft::sanitize` (which also warns), for every
/// import format alike.
fn remap_credential(credential: CredentialRef, _draft: &mut Draft) -> CredentialRef {
    credential
}

fn remap_options(
    options: ConnectionOptions,
    ids: &HashMap<Uuid, Uuid>,
    draft: &mut Draft,
) -> ConnectionOptions {
    match options {
        ConnectionOptions::Ssh {
            jump_host,
            keepalive_secs,
        } => {
            let jump_host = match jump_host {
                None => None,
                Some(old) => match ids.get(&old) {
                    Some(new) => Some(*new),
                    None => {
                        draft.warn(None, "a jump host outside the file was dropped");
                        None
                    }
                },
            };
            ConnectionOptions::Ssh {
                jump_host,
                keepalive_secs,
            }
        }
        other => other,
    }
}

/// Warns for every key that looks like a password, at any depth. The values are not read.
fn scan_for_passwords(value: &serde_json::Value, draft: &mut Draft) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if key.to_ascii_lowercase().contains("password") {
                    draft.warn(None, PASSWORD_IGNORED);
                }
                scan_for_passwords(child, draft);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                scan_for_passwords(item, draft);
            }
        }
        _ => {}
    }
}

/// Serialises `nodes` as a Portolano export. Credential references become `None`; a jump host
/// outside `nodes` is cleared so that the file is self-contained.
pub(crate) fn export_bytes(nodes: &[Node]) -> Result<Vec<u8>, ImportError> {
    let ids: HashSet<Uuid> = nodes.iter().map(Node::id).collect();
    let cleaned: Vec<Node> = nodes
        .iter()
        .map(|node| clean_for_export(node, &ids))
        .collect();
    let out = ExportOut {
        format: FORMAT,
        version: VERSION,
        nodes: &cleaned,
    };
    let text = serde_json::to_string_pretty(&out)
        .map_err(|e| ImportError::Io(std::io::Error::other(e.to_string())))?;
    Ok(text.into_bytes())
}

fn clean_for_export(node: &Node, ids: &HashSet<Uuid>) -> Node {
    let mut node = node.clone();
    match &mut node {
        Node::Folder(folder) => folder.credential = scrub_credential(folder.credential),
        Node::Connection(conn) => {
            conn.credential = scrub_credential(conn.credential);
            if let ConnectionOptions::Ssh { jump_host, .. } = &mut conn.options {
                if jump_host.is_some_and(|jump| !ids.contains(&jump)) {
                    *jump_host = None;
                }
            }
        }
    }
    node
}

fn scrub_credential(credential: CredentialRef) -> CredentialRef {
    match credential {
        CredentialRef::Id { .. } => CredentialRef::None,
        other => other,
    }
}
